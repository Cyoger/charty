use ratatui::{
    widgets::ListState,
    Frame,
};

use crate::stock::StockData;
use std::sync::Arc;
use std::time::Instant;
use std::time::Duration;
use std::collections::VecDeque;
use tokio::sync::Mutex;
use chrono::{DateTime, Utc};

mod landing;
use landing::render_landing;

mod chart;
use chart::render_chart_view;

mod live;
use live::{render_live_ticker, render_live_candles, render_live_mode_select, render_error_log};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum WebSocketStatus {
    Idle,
    Connecting,
    Connected { since: DateTime<Utc> },
    Reconnecting { attempt: u32, next_retry_in: Duration },
    Error { message: String, recoverable: bool },
    Disconnected,
}

pub struct UpdateThrottle {
    last_update: Instant,
    min_interval: Duration,
}

impl UpdateThrottle {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            last_update: Instant::now(),
            min_interval,
        }
    }

    pub fn should_update(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.last_update) >= self.min_interval {
            self.last_update = now;
            true
        } else {
            false
        }
    }
}

pub enum AppState {
    Landing,
    Chart,
    LiveTicker,
    LiveCandles,
}

#[derive(Debug, Clone)]
pub struct Trade {
    pub price: f64,
    pub timestamp: DateTime<Utc>,
    pub volume: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Candlestick {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: u64,
    pub timestamp: DateTime<Utc>,
    pub trade_count: u32,
}

pub struct App {
    pub state: AppState,
    pub symbol: String,
    pub timeframe: crate::stock::TimeFrame,
    pub stock_data: Option<StockData>,
    pub input_mode: bool,
    pub input_buffer: String,
    pub error_message: Option<String>,
    pub loading: bool,
    pub live_updates_enabled: bool,
    pub last_live_price: Option<f64>,
    pub popular_list_state: ListState,
    pub popular_stocks: Vec<(&'static str, &'static str)>,
	pub ws_should_stop: Arc<Mutex<bool>>,
    pub ws_status: WebSocketStatus,
    pub ws_last_update: Option<DateTime<Utc>>,
    pub ws_error_log: VecDeque<String>,
    pub update_throttle: UpdateThrottle,
    pub show_error_log: bool,
    // Live mode fields
    pub show_live_mode_select: bool,
    pub live_trades: VecDeque<Trade>,
    pub live_candles: VecDeque<Candlestick>,
    pub current_candle: Option<Candlestick>,
    pub candle_interval_secs: u64,
    pub total_live_volume: u64,
    pub total_trade_count: u32,
}

impl App {
    pub fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));

        Self {
            state: AppState::Landing,
            symbol: String::new(),
            timeframe: crate::stock::TimeFrame::OneMonth,
            stock_data: None,
            input_mode: false,
            input_buffer: String::new(),
            error_message: None,
            loading: false,
            live_updates_enabled: false,
            last_live_price: None,
            popular_list_state: list_state,
            popular_stocks: vec![
                ("^GSPC", "S&P 500 Index"),
                ("^DJI", "Dow Jones Industrial Average"),
                ("^IXIC", "Nasdaq Composite"),
                ("SPY", "SPDR S&P 500 ETF"),
                ("QQQ", "Invesco QQQ Trust"),
                ("AAPL", "Apple Inc."),
                ("MSFT", "Microsoft Corporation"),
                ("GOOGL", "Alphabet Inc."),
                ("AMZN", "Amazon.com Inc."),
                ("TSLA", "Tesla Inc."),
                ("NVDA", "NVIDIA Corporation"),
                ("META", "Meta Platforms Inc."),
            ],
			ws_should_stop: Arc::new(Mutex::new(false)),
            ws_status: WebSocketStatus::Idle,
            ws_last_update: None,
            ws_error_log: VecDeque::new(),
            update_throttle: UpdateThrottle::new(Duration::from_millis(100)), // Faster for live modes
            show_error_log: false,
            // Live mode fields
            show_live_mode_select: false,
            live_trades: VecDeque::new(),
            live_candles: VecDeque::new(),
            current_candle: None,
            candle_interval_secs: 60, // 1 minute candles
            total_live_volume: 0,
            total_trade_count: 0,
        }
    }

    pub fn fetch_data(&mut self) {
        self.loading = true;
        match crate::stock::fetch_stock_data(&self.symbol, self.timeframe) {
            Ok(data) => {
                self.stock_data = Some(data);
                self.error_message = None;
                self.state = AppState::Chart;
            }
            Err(e) => {
                self.error_message = Some(format!("Error fetching {}: {}", self.symbol, e));
                self.state = AppState::Chart;
            }
        }
        self.loading = false;
    }

    pub fn update_live_price(&mut self, price: f64, volume: Option<u64>) {
        let now = Utc::now();
        self.last_live_price = Some(price);
        self.ws_last_update = Some(now);
        self.total_trade_count += 1;
        if let Some(v) = volume {
            self.total_live_volume += v;
        }

        // Add to trade history for ticker view
        let trade = Trade {
            price,
            timestamp: now,
            volume,
        };
        self.live_trades.push_front(trade);
        if self.live_trades.len() > 100 {
            self.live_trades.pop_back();
        }

        // Aggregate into candlesticks
        self.aggregate_into_candle(price, volume.unwrap_or(0), now);

        // Update stock data for header display
        if let Some(ref mut data) = self.stock_data {
            data.live_current_price = Some(price);
            data.current_price = price;

            data.live_ticks.push_back(crate::stock::LiveTick {
                price,
                timestamp: now,
            });

            if data.live_ticks.len() > 100 {
                data.live_ticks.pop_front();
            }

            data.change = price - data.base_historical_price;
            data.change_percent = (data.change / data.base_historical_price) * 100.0;
        }
    }

    fn aggregate_into_candle(&mut self, price: f64, volume: u64, timestamp: DateTime<Utc>) {
        let interval_secs = self.candle_interval_secs as i64;
        let candle_start = timestamp.timestamp() / interval_secs * interval_secs;

        match &mut self.current_candle {
            Some(candle) => {
                let current_start = candle.timestamp.timestamp() / interval_secs * interval_secs;

                if candle_start == current_start {
                    // Same candle - update OHLC
                    candle.high = candle.high.max(price);
                    candle.low = candle.low.min(price);
                    candle.close = price;
                    candle.volume += volume;
                    candle.trade_count += 1;
                } else {
                    // New candle - finalize current and start new
                    let finished_candle = candle.clone();
                    self.live_candles.push_back(finished_candle);
                    if self.live_candles.len() > 60 {
                        self.live_candles.pop_front();
                    }

                    *candle = Candlestick {
                        open: price,
                        high: price,
                        low: price,
                        close: price,
                        volume,
                        timestamp,
                        trade_count: 1,
                    };
                }
            }
            None => {
                // Start first candle
                self.current_candle = Some(Candlestick {
                    open: price,
                    high: price,
                    low: price,
                    close: price,
                    volume,
                    timestamp,
                    trade_count: 1,
                });
            }
        }
    }

    pub fn clear_live_data(&mut self) {
        self.live_trades.clear();
        self.live_candles.clear();
        self.current_candle = None;
        self.total_live_volume = 0;
        self.total_trade_count = 0;
        self.last_live_price = None;
        if let Some(ref mut data) = self.stock_data {
            data.live_ticks.clear();
            data.live_current_price = None;
        }
    }

    pub fn add_error_to_log(&mut self, error: String) {
        let timestamp = Utc::now().format("%H:%M:%S").to_string();
        let error_entry = format!("[{}] {}", timestamp, error);

        self.ws_error_log.push_back(error_entry);

        // Keep only last 10 errors
        if self.ws_error_log.len() > 10 {
            self.ws_error_log.pop_front();
        }
    }

	pub fn get_base_price(&self) -> f64 { 
        self.stock_data
            .as_ref()
            .map(|d| d.current_price)
            .unwrap_or(150.0)
    }

    pub fn next_popular(&mut self) {
        let i = match self.popular_list_state.selected() {
            Some(i) => {
                if i >= self.popular_stocks.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.popular_list_state.select(Some(i));
    }

    pub fn previous_popular(&mut self) {
        let i = match self.popular_list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.popular_stocks.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.popular_list_state.select(Some(i));
    }

    pub fn select_popular(&mut self) {
        if let Some(i) = self.popular_list_state.selected() {
            self.symbol = self.popular_stocks[i].0.to_string();
            self.fetch_data();
        }
    }
}

pub fn ui(f: &mut Frame, app: &App) {
    match app.state {
        AppState::Landing => render_landing(f, app),
        AppState::Chart => render_chart_view(f, app),
        AppState::LiveTicker => render_live_ticker(f, app),
        AppState::LiveCandles => render_live_candles(f, app),
    }

    // Render popups on top
    if app.show_live_mode_select {
        render_live_mode_select(f);
    }
    if app.show_error_log {
        render_error_log(f, app);
    }
}
