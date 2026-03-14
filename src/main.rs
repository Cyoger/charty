use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use tokio::sync::mpsc;
use std::sync::Arc;
use tokio::sync::Mutex;

mod alerts;
mod stock;
mod ui;
mod watchlist;
mod websocket;

use ui::{App, AppState, LandingPanel, MarketPanel, WebSocketStatus};
use std::collections::HashMap;
use crate::stock::QuoteSnapshot;
use websocket::LivePrice;
use std::fs::OpenOptions;
use std::io::Write;

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
    let (quotes_tx, mut quotes_rx) = mpsc::unbounded_channel::<HashMap<String, QuoteSnapshot>>();

    // Fetch landing quotes in background so terminal opens immediately
    let quotes_tx_init = quotes_tx.clone();
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            match crate::stock::YahooSession::new() {
                Ok(session) => {
                    let syms: Vec<&str> = vec![
                        "^GSPC", "^DJI", "^IXIC", "SPY", "QQQ",
                        "AAPL", "MSFT", "GOOGL", "AMZN", "TSLA", "NVDA", "META",
                    ];
                    crate::stock::fetch_batch_quotes(&session, &syms).ok()
                }
                Err(_) => None,
            }
        }).await;
        if let Ok(Some(quotes)) = result {
            let _ = quotes_tx_init.send(quotes);
        }
    });

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run the app
    let res = run_app(&mut terminal, &mut app, &mut rx, &mut status_rx, &mut quotes_rx, tx, status_tx, quotes_tx).await;

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
    quotes_rx: &mut mpsc::UnboundedReceiver<HashMap<String, QuoteSnapshot>>,
    tx: mpsc::UnboundedSender<LivePrice>,
    status_tx: mpsc::UnboundedSender<WebSocketStatus>,
    quotes_tx: mpsc::UnboundedSender<HashMap<String, QuoteSnapshot>>,
) -> Result<(), io::Error> {
    let mut ws_task_handle: Option<tokio::task::JoinHandle<()>> = None;
    let mut last_alert_check = std::time::Instant::now();
    const ALERT_CHECK_SECS: u64 = 30;

    loop {
        terminal.draw(|f| ui::ui(f, app))?;

        // Check for WebSocket status updates
        while let Ok(status) = status_rx.try_recv() {
            if let WebSocketStatus::Error { ref message, .. } = status {
                app.add_error_to_log(message.clone());
            }
            app.ws_status = status;
        }

        // Check for background quote updates; run alert checks on arrival
        if let Ok(quotes) = quotes_rx.try_recv() {
            let triggered = app.check_alerts(&quotes);
            for (symbol, target) in triggered {
                let msg = format!("{} crossed ${:.2}", symbol, target);
                let _ = std::process::Command::new("notify-send")
                    .arg("Charty Price Alert")
                    .arg(&msg)
                    .spawn();
            }
            app.landing_quotes.extend(quotes.into_iter());
        }

        // Periodically fetch prices for any pending alerts
        let pending_alert_syms: Vec<String> = app.alerts.iter()
            .filter(|a| !a.triggered)
            .map(|a| a.symbol.clone())
            .collect();
        if !pending_alert_syms.is_empty()
            && last_alert_check.elapsed().as_secs() >= ALERT_CHECK_SECS
        {
            last_alert_check = std::time::Instant::now();
            let qtx = quotes_tx.clone();
            tokio::spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    match crate::stock::YahooSession::new() {
                        Ok(session) => {
                            let syms: Vec<&str> = pending_alert_syms.iter().map(|s| s.as_str()).collect();
                            crate::stock::fetch_batch_quotes(&session, &syms).ok()
                        }
                        Err(_) => None,
                    }
                }).await;
                if let Ok(Some(quotes)) = result {
                    let _ = qtx.send(quotes);
                }
            });
        }

        // Check for live price updates with throttling
        // Drain all pending messages to prevent unbounded queueing
        let mut latest_price = None;
        while let Ok(live_price) = rx.try_recv() {
            latest_price = Some(live_price);
        }

        if let Some(live_price) = latest_price {
            if app.live_updates_enabled && app.update_throttle.should_update() {
                app.update_live_price(live_price.price, live_price.volume);
            }
        }

        // Check for keyboard input
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if handle_input(app, key.code, &mut ws_task_handle, &tx, &status_tx).await {
                        // Stop WebSocket before quitting
                        stop_websocket(&mut ws_task_handle, &app.ws_should_stop).await;
                        return Ok(());
                    }
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
    // Alert input popup is modal — handle it before any state-specific logic
    if app.show_alert_input {
        match key {
            KeyCode::Enter => {
                if let Ok(target) = app.alert_input_buffer.parse::<f64>() {
                    let sym = app.alert_target_symbol.clone();
                    let current = app.current_price_for(&sym).unwrap_or(target);
                    app.set_price_alert(sym, target, current);
                }
                app.alert_input_buffer.clear();
                app.show_alert_input = false;
            }
            KeyCode::Esc => {
                app.alert_input_buffer.clear();
                app.show_alert_input = false;
            }
            KeyCode::Backspace => { app.alert_input_buffer.pop(); }
            KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
                app.alert_input_buffer.push(c);
            }
            _ => {}
        }
        return false;
    }

    match app.state {
        AppState::Landing => {
            // Handle help popup first
            if app.show_help {
                match key {
                    KeyCode::Char('h') | KeyCode::Esc => {
                        app.show_help = false;
                        return false;
                    }
                    _ => return false,
                }
            }

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
                    KeyCode::Tab => {
                        app.landing_panel = match app.landing_panel {
                            LandingPanel::Popular => LandingPanel::Watchlist,
                            LandingPanel::Watchlist => LandingPanel::Popular,
                        };
                    }
                    KeyCode::Up => {
                        match app.landing_panel {
                            LandingPanel::Popular => app.previous_popular(),
                            LandingPanel::Watchlist => app.previous_watchlist(),
                        }
                    }
                    KeyCode::Down => {
                        match app.landing_panel {
                            LandingPanel::Popular => app.next_popular(),
                            LandingPanel::Watchlist => app.next_watchlist(),
                        }
                    }
                    KeyCode::Enter => {
                        stop_websocket(ws_task_handle, &app.ws_should_stop).await;
                        match app.landing_panel {
                            LandingPanel::Popular => app.select_popular(),
                            LandingPanel::Watchlist => app.select_watchlist(),
                        }
                    }
                    KeyCode::Char('d') => {
                        if app.landing_panel == LandingPanel::Watchlist {
                            app.remove_from_watchlist();
                        }
                    }
                    KeyCode::Char('h') => {
                        app.show_help = !app.show_help;
                    }
                    KeyCode::Char('a') => {
                        if let Some(sym) = app.selected_symbol() {
                            if app.alert_for_symbol(&sym).is_some() {
                                app.clear_price_alert(&sym);
                            } else {
                                app.alert_target_symbol = sym;
                                app.alert_input_buffer.clear();
                                app.show_alert_input = true;
                            }
                        }
                    }
                    KeyCode::Char('m') => {
                        app.state = AppState::Market;
                        app.fetch_market_data();
                    }
                    KeyCode::Char('r') => {
                        app.refresh_landing_quotes();
                    }
                    _ => {}
                }
            }
            false
        }
        AppState::Market => {
            match key {
                KeyCode::Char('q') => return true,
                KeyCode::Char('b') | KeyCode::Esc => {
                    app.state = AppState::Landing;
                }
                KeyCode::Char('r') => {
                    app.fetch_market_data();
                }
                KeyCode::Tab => {
                    app.market_panel = match app.market_panel {
                        MarketPanel::Gainers => MarketPanel::Losers,
                        MarketPanel::Losers => MarketPanel::Active,
                        MarketPanel::Active => MarketPanel::Gainers,
                    };
                }
                KeyCode::Up => app.previous_market(),
                KeyCode::Down => app.next_market(),
                KeyCode::Enter => {
                    stop_websocket(ws_task_handle, &app.ws_should_stop).await;
                    app.select_market();
                }
                _ => {}
            }
            false
        }
        AppState::Chart => {
            // Handle popups first
            if app.show_error_log {
                match key {
                    KeyCode::Esc => {
                        app.show_error_log = false;
                        return false;
                    }
                    _ => return false,
                }
            }

            if app.show_help {
                match key { 
                    KeyCode::Char('h') | KeyCode::Esc => {
                        app.show_help = false;
                        return false;
                    }
                    _ => return false,
                }
            }

            if app.show_live_mode_select {
                match key {
                    KeyCode::Char('1') => {
                        // Live Ticker mode
                        app.show_live_mode_select = false;
                        app.clear_live_data();
                        app.live_updates_enabled = true;
                        app.state = AppState::LiveTicker;

                        // Start WebSocket
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
                        return false;
                    }
                    KeyCode::Char('2') => {
                        // Live Candles mode
                        app.show_live_mode_select = false;
                        app.clear_live_data();
                        app.load_historical_candles(); // Load historical candles first
                        app.live_updates_enabled = true;
                        app.state = AppState::LiveCandles;

                        // Start WebSocket
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
                        return false;
                    }
                    KeyCode::Esc => {
                        app.show_live_mode_select = false;
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
                    // Show live mode selection popup
                    app.show_live_mode_select = true;
                    false
                }
                KeyCode::Char('h') => {
                    app.show_help = !app.show_help;
                    false
                }
                KeyCode::Char('w') => {
                    app.add_to_watchlist();
                    false
                }
                // Candlestick toggle disabled for now
                // KeyCode::Char('c') => {
                //     app.show_candlesticks = !app.show_candlesticks;
                //     false
                // }
                KeyCode::Char('a') => {
                    let sym = app.symbol.clone();
                    if app.alert_for_symbol(&sym).is_some() {
                        app.clear_price_alert(&sym);
                    } else {
                        app.alert_target_symbol = sym;
                        app.alert_input_buffer.clear();
                        app.show_alert_input = true;
                    }
                    false
                }
                KeyCode::Char('r') => {
                    app.fetch_data();
                    false
                }
                KeyCode::Left => {
                    if app.show_candlesticks {
                        // Change candle interval in candlestick mode
                        app.candle_interval = app.candle_interval.prev();
                        false
                    } else {
                        // Change timeframe in regular chart mode
                        app.timeframe = app.timeframe.prev();
                        app.fetch_data();
                        false
                    }
                }
                KeyCode::Right => {
                    if app.show_candlesticks {
                        // Change candle interval in candlestick mode
                        app.candle_interval = app.candle_interval.next();
                        false
                    } else {
                        // Change timeframe in regular chart mode
                        app.timeframe = app.timeframe.next();
                        app.fetch_data();
                        false
                    }
                }
                _ => false,
            }
        },
        AppState::LiveTicker | AppState::LiveCandles => {
            // Handle popups first
            if app.show_help {
                match key {
                    KeyCode::Char('h') | KeyCode::Esc => {
                        app.show_help = false;
                        return false;
                    }
                    _ => return false,
                }
            }

            if app.show_error_log {
                match key {
                    KeyCode::Esc => {
                        app.show_error_log = false;
                        return false;
                    }
                    _ => return false,
                }
            }

            if app.show_live_mode_select {
                match key {
                    KeyCode::Char('1') => {
                        app.show_live_mode_select = false;
                        if !matches!(app.state, AppState::LiveTicker) {
                            app.clear_live_data();
                            app.state = AppState::LiveTicker;
                        }
                        return false;
                    }
                    KeyCode::Char('2') => {
                        app.show_live_mode_select = false;
                        if !matches!(app.state, AppState::LiveCandles) {
                            app.clear_live_data();
                            app.state = AppState::LiveCandles;
                        }
                        return false;
                    }
                    KeyCode::Esc => {
                        app.show_live_mode_select = false;
                        return false;
                    }
                    _ => return false,
                }
            }

            match key {
                KeyCode::Char('q') => true,
                KeyCode::Char('b') => {
                    // Go back to historical chart
                    app.live_updates_enabled = false;
                    app.state = AppState::Chart;
                    stop_websocket(ws_task_handle, &app.ws_should_stop).await;
                    app.ws_status = WebSocketStatus::Idle;
                    false
                }
                KeyCode::Char('h') => {
                    app.show_help = !app.show_help;
                    false
                }
                KeyCode::Char('l') => {
                    // Show live mode selection to switch
                    app.show_live_mode_select = true;
                    false
                }
                KeyCode::Char('e') => {
                    app.show_error_log = !app.show_error_log;
                    false
                }
                KeyCode::Char('a') | KeyCode::Char('p') => {
                    let sym = app.symbol.clone();
                    if app.alert_for_symbol(&sym).is_some() {
                        app.clear_price_alert(&sym);
                    } else {
                        app.alert_target_symbol = sym;
                        app.alert_input_buffer.clear();
                        app.show_alert_input = true;
                    }
                    false
                }
                KeyCode::Left => {
                    // Only change interval in LiveCandles mode
                    if matches!(app.state, AppState::LiveCandles) {
                        app.candle_interval = app.candle_interval.prev();
                        app.clear_live_data();
                        app.load_historical_candles();
                    }
                    false
                }
                KeyCode::Right => {
                    // Only change interval in LiveCandles mode
                    if matches!(app.state, AppState::LiveCandles) {
                        app.candle_interval = app.candle_interval.next();
                        app.clear_live_data();
                        app.load_historical_candles();
                    }
                    false
                }
                _ => false,
            }
        },
    }
}