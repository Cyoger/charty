use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use tokio::sync::mpsc;
use std::sync::Arc;
use tokio::sync::Mutex;

mod stock;
mod ui;
mod websocket;

use ui::{App, AppState, WebSocketStatus};
use websocket::LivePrice;
use std::fs::OpenOptions;
use std::io::Write;

fn should_auto_start_live_mode(symbol: &str) -> bool {
    // Don't auto-start for indices (starts with ^)
    !symbol.starts_with('^')
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    dotenv::dotenv().ok();
    let mut log_file = OpenOptions::new()
    .create(true)
    .append(true)
    .open("debug.log")?;

    writeln!(log_file, "Starting app...")?;

    let mut app = App::new();

    let (tx, mut rx) = mpsc::unbounded_channel::<LivePrice>();
    let (status_tx, mut status_rx) = mpsc::unbounded_channel::<WebSocketStatus>();

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run the app
    let res = run_app(&mut terminal, &mut app, &mut rx, &mut status_rx, tx, status_tx).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("Error: {:?}", err);
    }

    Ok(())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    rx: &mut mpsc::UnboundedReceiver<LivePrice>,
    status_rx: &mut mpsc::UnboundedReceiver<WebSocketStatus>,
    tx: mpsc::UnboundedSender<LivePrice>,
    status_tx: mpsc::UnboundedSender<WebSocketStatus>,
) -> Result<(), io::Error> {
    let mut ws_task_handle: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        terminal.draw(|f| ui::ui(f, app))?;

        // Check for WebSocket status updates
        while let Ok(status) = status_rx.try_recv() {
            // Add errors to error log
            if let WebSocketStatus::Error { ref message, .. } = status {
                app.add_error_to_log(message.clone());
            }
            app.ws_status = status;
        }

        // Check for live price updates with throttling
        if let Ok(live_price) = rx.try_recv() {
            if app.live_updates_enabled && app.update_throttle.should_update() {
                app.update_live_price(live_price.price);
            }
        }

        // Check for keyboard input
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if handle_input(app, key.code, &mut ws_task_handle, &tx, &status_tx).await {
                    // Stop WebSocket before quitting
                    stop_websocket(&mut ws_task_handle, &app.ws_should_stop).await;
                    return Ok(());
                }
            }
        }
    }
}

async fn stop_websocket(
    ws_task_handle: &mut Option<tokio::task::JoinHandle<()>>,
    should_stop: &Arc<Mutex<bool>>,
) {
    *should_stop.lock().await = true;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    if let Some(handle) = ws_task_handle.take() {
        handle.abort();
    }
}

async fn handle_input(
    app: &mut App,
    key: KeyCode,
    ws_task_handle: &mut Option<tokio::task::JoinHandle<()>>,
    tx: &mpsc::UnboundedSender<LivePrice>,
    status_tx: &mpsc::UnboundedSender<WebSocketStatus>,
) -> bool {
    match app.state {
        AppState::Landing => {
            if app.input_mode {
                match key {
                    KeyCode::Char('q') if app.input_buffer.is_empty() => return true,
                    KeyCode::Enter => {
                        if !app.input_buffer.is_empty() {
                            app.symbol = app.input_buffer.to_uppercase();
                            app.input_buffer.clear();
                            app.input_mode = false;

                            // Stop existing WebSocket and fetch new data
                            stop_websocket(ws_task_handle, &app.ws_should_stop).await;
                            app.fetch_data();

                            // Auto-start live mode if appropriate
                            if should_auto_start_live_mode(&app.symbol) {
                                app.live_updates_enabled = true;
                                *app.ws_should_stop.lock().await = false;

                                let symbol_clone = app.symbol.clone();
                                let base_price = app.get_base_price();
                                let tx_clone = tx.clone();
                                let status_tx_clone = status_tx.clone();
                                let should_stop = app.ws_should_stop.clone();

                                *ws_task_handle = Some(tokio::spawn(async move {
                                    websocket::start_websocket(symbol_clone, base_price, tx_clone, status_tx_clone, should_stop).await;
                                }));
                            }
                        }
                    }
                    KeyCode::Esc => {
                        app.input_buffer.clear();
                        app.input_mode = false;
                    }
                    KeyCode::Backspace => {
                        app.input_buffer.pop();
                    }
                    KeyCode::Char(c) => {
                        app.input_buffer.push(c);
                    }
                    _ => {}
                }
            } else {
                match key {
                    KeyCode::Char('q') => return true,
                    KeyCode::Char('s') => {
                        app.input_mode = true;
                    }
                    KeyCode::Up => {
                        app.previous_popular();
                    }
                    KeyCode::Down => {
                        app.next_popular();
                    }
                    KeyCode::Enter => {
                        // Stop existing WebSocket
                        stop_websocket(ws_task_handle, &app.ws_should_stop).await;

                        app.select_popular();

                        // Auto-start live mode if appropriate
                        if should_auto_start_live_mode(&app.symbol) {
                            app.live_updates_enabled = true;
                            *app.ws_should_stop.lock().await = false;

                            let symbol_clone = app.symbol.clone();
                            let base_price = app.get_base_price();
                            let tx_clone = tx.clone();
                            let status_tx_clone = status_tx.clone();
                            let should_stop = app.ws_should_stop.clone();

                            *ws_task_handle = Some(tokio::spawn(async move {
                                websocket::start_websocket(symbol_clone, base_price, tx_clone, status_tx_clone, should_stop).await;
                            }));
                        }
                    }
                    _ => {}
                }
            }
            false
        }
        AppState::Chart => {
            // Handle error log popup first
            if app.show_error_log {
                match key {
                    KeyCode::Esc => {
                        app.show_error_log = false;
                        return false;
                    }
                    _ => return false,
                }
            }

            match key {
                KeyCode::Char('q') => true,
                KeyCode::Char('b') => {
                    app.state = AppState::Landing;
                    app.stock_data = None;
                    app.error_message = None;
                    app.live_updates_enabled = false;

                    // Stop WebSocket when going back
                    stop_websocket(ws_task_handle, &app.ws_should_stop).await;
                    false
                }
                KeyCode::Char('s') => {
                    app.state = AppState::Landing;
                    app.input_mode = true;
                    false
                }
                KeyCode::Char('e') => {
                    app.show_error_log = !app.show_error_log;
                    false
                }
                KeyCode::Char('l') => {
                    app.live_updates_enabled = !app.live_updates_enabled;

                    // Start WebSocket if enabling live mode
                    if app.live_updates_enabled {
                        stop_websocket(ws_task_handle, &app.ws_should_stop).await;
                        *app.ws_should_stop.lock().await = false;

                        let symbol_clone = app.symbol.clone();
                        let base_price = app.get_base_price();
                        let tx_clone = tx.clone();
                        let status_tx_clone = status_tx.clone();
                        let should_stop = app.ws_should_stop.clone();

                        *ws_task_handle = Some(tokio::spawn(async move {
                            websocket::start_websocket(symbol_clone, base_price, tx_clone, status_tx_clone, should_stop).await;
                        }));
                    } else {
                        stop_websocket(ws_task_handle, &app.ws_should_stop).await;
                        app.ws_status = WebSocketStatus::Idle;
                    }
                    false
                }
                KeyCode::Char('r') => {
                    let was_live = app.live_updates_enabled;

                    // Stop WebSocket, clear live data, and refresh historical
                    stop_websocket(ws_task_handle, &app.ws_should_stop).await;
                    if let Some(ref mut data) = app.stock_data {
                        data.live_ticks.clear();
                        data.live_current_price = None;
                    }
                    app.fetch_data();

                    // Restart WebSocket if live mode was enabled
                    if was_live {
                        *app.ws_should_stop.lock().await = false;

                        let symbol_clone = app.symbol.clone();
                        let base_price = app.get_base_price();
                        let tx_clone = tx.clone();
                        let status_tx_clone = status_tx.clone();
                        let should_stop = app.ws_should_stop.clone();

                        *ws_task_handle = Some(tokio::spawn(async move {
                            websocket::start_websocket(symbol_clone, base_price, tx_clone, status_tx_clone, should_stop).await;
                        }));
                    }
                    false
                }
                KeyCode::Left => {
                    let was_live = app.live_updates_enabled;

                    // Stop WebSocket and change timeframe
                    stop_websocket(ws_task_handle, &app.ws_should_stop).await;
                    if let Some(ref mut data) = app.stock_data {
                        data.live_ticks.clear();
                        data.live_current_price = None;
                    }
                    app.timeframe = app.timeframe.prev();
                    app.fetch_data();

                    // Restart WebSocket if live mode was enabled
                    if was_live {
                        *app.ws_should_stop.lock().await = false;

                        let symbol_clone = app.symbol.clone();
                        let base_price = app.get_base_price();
                        let tx_clone = tx.clone();
                        let status_tx_clone = status_tx.clone();
                        let should_stop = app.ws_should_stop.clone();

                        *ws_task_handle = Some(tokio::spawn(async move {
                            websocket::start_websocket(symbol_clone, base_price, tx_clone, status_tx_clone, should_stop).await;
                        }));
                    }
                    false
                }
                KeyCode::Right => {
                    let was_live = app.live_updates_enabled;

                    // Stop WebSocket and change timeframe
                    stop_websocket(ws_task_handle, &app.ws_should_stop).await;
                    if let Some(ref mut data) = app.stock_data {
                        data.live_ticks.clear();
                        data.live_current_price = None;
                    }
                    app.timeframe = app.timeframe.next();
                    app.fetch_data();

                    // Restart WebSocket if live mode was enabled
                    if was_live {
                        *app.ws_should_stop.lock().await = false;

                        let symbol_clone = app.symbol.clone();
                        let base_price = app.get_base_price();
                        let tx_clone = tx.clone();
                        let status_tx_clone = status_tx.clone();
                        let should_stop = app.ws_should_stop.clone();

                        *ws_task_handle = Some(tokio::spawn(async move {
                            websocket::start_websocket(symbol_clone, base_price, tx_clone, status_tx_clone, should_stop).await;
                        }));
                    }
                    false
                }
                _ => false,
            }
        },
    }
}