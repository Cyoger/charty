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

use ui::{App, AppState, Candlestick, LandingPanel, MarketPanel, WebSocketStatus};
use std::collections::HashMap;
use crate::stock::{QuoteSnapshot, log_debug};
use websocket::LivePrice;

enum AppUpdate {
    StockData { symbol: String, result: Result<stock::StockData, String> },
    MarketData {
        gainers: Vec<stock::MarketMover>,
        losers: Vec<stock::MarketMover>,
        active: Vec<stock::MarketMover>,
    },
    MarketError(String),
    HistoricalCandles(Vec<Candlestick>),
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    dotenv::dotenv().ok();
    #[cfg(debug_assertions)]
    { let _ = std::fs::write("debug.log", ""); }
    log_debug("=== charty started ===");

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
                Err(e) => {
                    log_debug(&format!("[session] init failed: {}", e));
                    None
                }
            }
        }).await;
        if let Ok(Some(quotes)) = result {
            log_debug(&format!("[session] initial quote fetch succeeded with {} symbols", quotes.len()));
            let _ = quotes_tx_init.send(quotes);
        } else {
            log_debug("[session] initial quote fetch produced no results");
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
        let msg = format!("Fatal error: {:?}", err);
        eprintln!("{}", msg);
        log_debug(&msg);
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
    let (update_tx, mut update_rx) = mpsc::unbounded_channel::<AppUpdate>();
    let mut ws_task_handle: Option<tokio::task::JoinHandle<()>> = None;
    let mut last_alert_check = std::time::Instant::now();
    const ALERT_CHECK_SECS: u64 = 30;
    let mut needs_redraw = true;

    loop {
        if needs_redraw {
            terminal.draw(|f| ui::ui(f, app))?;
            needs_redraw = false;
        }

        // Apply results from background data fetches
        while let Ok(update) = update_rx.try_recv() {
            match update {
                AppUpdate::StockData { symbol, result } => app.apply_stock_data(&symbol, result),
                AppUpdate::MarketData { gainers, losers, active } => app.apply_market_data(gainers, losers, active),
                AppUpdate::MarketError(e) => app.apply_market_error(e),
                AppUpdate::HistoricalCandles(candles) => app.apply_historical_candles(candles),
            }
            needs_redraw = true;
        }

        // Check for WebSocket status updates
        while let Ok(status) = status_rx.try_recv() {
            if let WebSocketStatus::Error { ref message, .. } = status {
                app.add_error_to_log(message.clone());
            }
            app.ws_status = status;
            needs_redraw = true;
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
            // Sync market_state into stock_data from the fresh quote
            let updated_state = app.stock_data.as_ref()
                .and_then(|d| app.landing_quotes.get(&d.symbol))
                .map(|q| q.market_state.clone());
            if let (Some(data), Some(state)) = (&mut app.stock_data, updated_state) {
                log_debug(&format!("[quote sync] {} market_state -> {:?}", data.symbol, state));
                data.market_state = state;
            } else {
                let sym = app.stock_data.as_ref().map(|d| d.symbol.as_str()).unwrap_or("<none>");
                log_debug(&format!("[quote sync] no update for stock_data symbol={}", sym));
            }
            needs_redraw = true;
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
        let mut latest_price = None;
        while let Ok(live_price) = rx.try_recv() {
            latest_price = Some(live_price);
        }

        if let Some(live_price) = latest_price {
            if app.live_updates_enabled && app.update_throttle.should_update() {
                app.update_live_price(live_price.price, live_price.volume);
                needs_redraw = true;
            }
        }

        // Poll for a key event on a dedicated thread so the tokio runtime
        // stays free. Times out after 50ms so live mode still gets periodic redraws.
        let poll_result = tokio::task::spawn_blocking(|| -> io::Result<Option<Event>> {
            if event::poll(std::time::Duration::from_millis(50))? {
                Ok(Some(event::read()?))
            } else {
                Ok(None)
            }
        }).await;

        match poll_result {
            Ok(Ok(Some(Event::Key(key)))) if key.kind == KeyEventKind::Press => {
                let quit = handle_input(app, key.code, &mut ws_task_handle, &tx, &status_tx, &update_tx, &quotes_tx).await;
                needs_redraw = true;
                if quit {
                    stop_websocket(&mut ws_task_handle, &app.ws_should_stop).await;
                    return Ok(());
                }
            }
            Ok(Ok(None)) => {
                if app.live_updates_enabled {
                    needs_redraw = true;
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {}
        }
    }
}

async fn stop_websocket(
    ws_task_handle: &mut Option<tokio::task::JoinHandle<()>>,
    should_stop: &Arc<Mutex<bool>>,
) {
    *should_stop.lock().await = true;
    if let Some(handle) = ws_task_handle.take() {
        handle.abort();
    }
}

fn spawn_stock_fetch(symbol: String, timeframe: stock::TimeFrame, update_tx: mpsc::UnboundedSender<AppUpdate>) {
    tokio::spawn(async move {
        let sym = symbol.clone();
        let result = tokio::task::spawn_blocking(move || {
            stock::fetch_stock_data(&sym, timeframe).map_err(|e| e.to_string())
        }).await.unwrap_or_else(|e| Err(e.to_string()));
        let _ = update_tx.send(AppUpdate::StockData { symbol, result });
    });
}

fn spawn_market_fetch(update_tx: mpsc::UnboundedSender<AppUpdate>) {
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(|| {
            let gainers = stock::fetch_market_movers("day_gainers", 10).map_err(|e| e.to_string())?;
            let losers  = stock::fetch_market_movers("day_losers",  10).map_err(|e| e.to_string())?;
            let active  = stock::fetch_market_movers("most_actives", 10).map_err(|e| e.to_string())?;
            Ok::<_, String>((gainers, losers, active))
        }).await.unwrap_or_else(|e| Err(e.to_string()));
        match result {
            Ok((gainers, losers, active)) => { let _ = update_tx.send(AppUpdate::MarketData { gainers, losers, active }); }
            Err(e) => { let _ = update_tx.send(AppUpdate::MarketError(e)); }
        }
    });
}

fn spawn_quotes_fetch(symbols: Vec<String>, quotes_tx: mpsc::UnboundedSender<HashMap<String, QuoteSnapshot>>) {
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            let session = stock::YahooSession::new().map_err(|e| e.to_string())?;
            let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
            stock::fetch_batch_quotes(&session, &refs).map_err(|e| e.to_string())
        }).await.unwrap_or_else(|e| Err(e.to_string()));
        if let Ok(quotes) = result {
            let _ = quotes_tx.send(quotes);
        }
    });
}

fn spawn_candles_fetch(symbol: String, interval: String, update_tx: mpsc::UnboundedSender<AppUpdate>) {
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            stock::fetch_historical_candles(&symbol, &interval).map_err(|e| e.to_string())
        }).await.unwrap_or_else(|e| Err(e.to_string()));
        if let Ok(candles) = result {
            let _ = update_tx.send(AppUpdate::HistoricalCandles(candles));
        }
    });
}

async fn handle_input(
    app: &mut App,
    key: KeyCode,
    ws_task_handle: &mut Option<tokio::task::JoinHandle<()>>,
    tx: &mpsc::UnboundedSender<LivePrice>,
    status_tx: &mpsc::UnboundedSender<WebSocketStatus>,
    update_tx: &mpsc::UnboundedSender<AppUpdate>,
    quotes_tx: &mpsc::UnboundedSender<HashMap<String, QuoteSnapshot>>,
) -> bool {
    // Normalize char keys to lowercase so Caps Lock doesn't break shortcuts.
    let key = match key {
        KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
        other => other,
    };

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
                    KeyCode::Char('q') => return true,
                    KeyCode::Char('h') | KeyCode::Esc => {
                        app.show_help = false;
                    }
                    _ => {}
                }
                return false;
            }

            if app.input_mode {
                match key {
                    KeyCode::Enter => {
                        if !app.input_buffer.is_empty() {
                            app.symbol = app.input_buffer.to_uppercase();
                            app.input_buffer.clear();
                            app.input_mode = false;

                            stop_websocket(ws_task_handle, &app.ws_should_stop).await;
                            app.fetch_data();
                            spawn_stock_fetch(app.symbol.clone(), app.timeframe, update_tx.clone());
                            spawn_quotes_fetch(vec![app.symbol.clone()], quotes_tx.clone());
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
                        if !app.symbol.is_empty() {
                            app.fetch_data();
                            spawn_stock_fetch(app.symbol.clone(), app.timeframe, update_tx.clone());
                            spawn_quotes_fetch(vec![app.symbol.clone()], quotes_tx.clone());
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
                        spawn_market_fetch(update_tx.clone());
                    }
                    KeyCode::Char('r') => {
                        let mut symbols: Vec<String> = app.popular_stocks.iter().map(|(t, _)| t.to_string()).collect();
                        for s in &app.watchlist {
                            if !symbols.contains(s) { symbols.push(s.clone()); }
                        }
                        for a in app.alerts.iter().filter(|a| !a.triggered) {
                            if !symbols.contains(&a.symbol) { symbols.push(a.symbol.clone()); }
                        }
                        spawn_quotes_fetch(symbols, quotes_tx.clone());
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
                    spawn_market_fetch(update_tx.clone());
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
                    if !app.symbol.is_empty() {
                        app.fetch_data();
                        spawn_stock_fetch(app.symbol.clone(), app.timeframe, update_tx.clone());
                        spawn_quotes_fetch(vec![app.symbol.clone()], quotes_tx.clone());
                    }
                }
                _ => {}
            }
            false
        }
        AppState::Chart => {
            // Handle popups first
            if app.show_error_log {
                match key {
                    KeyCode::Char('q') => return true,
                    KeyCode::Char('e') | KeyCode::Esc => {
                        app.show_error_log = false;
                    }
                    _ => {}
                }
                return false;
            }

            if app.show_help {
                match key {
                    KeyCode::Char('q') => return true,
                    KeyCode::Char('h') | KeyCode::Esc => {
                        app.show_help = false;
                    }
                    _ => {}
                }
                return false;
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
                        spawn_candles_fetch(app.symbol.clone(), app.candle_interval.to_string().to_owned(), update_tx.clone());
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
                KeyCode::Char('v') => {
                    app.show_volume = !app.show_volume;
                    false
                }
                KeyCode::Char('i') => {
                    app.show_sma = !app.show_sma;
                    false
                }
                KeyCode::Char('r') => {
                    app.fetch_data();
                    spawn_stock_fetch(app.symbol.clone(), app.timeframe, update_tx.clone());
                    spawn_quotes_fetch(vec![app.symbol.clone()], quotes_tx.clone());
                    false
                }
                KeyCode::Left => {
                    if app.show_candlesticks {
                        app.candle_interval = app.candle_interval.prev();
                        false
                    } else {
                        app.timeframe = app.timeframe.prev();
                        app.fetch_data();
                        spawn_stock_fetch(app.symbol.clone(), app.timeframe, update_tx.clone());
                        false
                    }
                }
                KeyCode::Right => {
                    if app.show_candlesticks {
                        app.candle_interval = app.candle_interval.next();
                        false
                    } else {
                        app.timeframe = app.timeframe.next();
                        app.fetch_data();
                        spawn_stock_fetch(app.symbol.clone(), app.timeframe, update_tx.clone());
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
                    KeyCode::Char('q') => return true,
                    KeyCode::Char('h') | KeyCode::Esc => {
                        app.show_help = false;
                    }
                    _ => {}
                }
                return false;
            }

            if app.show_error_log {
                match key {
                    KeyCode::Char('q') => return true,
                    KeyCode::Char('e') | KeyCode::Esc => {
                        app.show_error_log = false;
                    }
                    _ => {}
                }
                return false;
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
                    if matches!(app.state, AppState::LiveCandles) {
                        app.candle_interval = app.candle_interval.prev();
                        app.clear_live_data();
                        spawn_candles_fetch(app.symbol.clone(), app.candle_interval.to_string().to_owned(), update_tx.clone());
                    }
                    false
                }
                KeyCode::Right => {
                    if matches!(app.state, AppState::LiveCandles) {
                        app.candle_interval = app.candle_interval.next();
                        app.clear_live_data();
                        spawn_candles_fetch(app.symbol.clone(), app.candle_interval.to_string().to_owned(), update_tx.clone());
                    }
                    false
                }
                _ => false,
            }
        },
    }
}