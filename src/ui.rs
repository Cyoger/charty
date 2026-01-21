use ratatui::{
    layout::{Constraint, Direction, Layout, Alignment},
    widgets::{Block, Borders, Paragraph, Chart, Dataset, Axis, GraphType, List, ListItem, ListState},
    symbols,
    style::{Style, Color, Modifier},
    text::{Line, Span},
    Frame,
};

use crate::stock::{self, StockData, TimeFrame};
use std::sync::Arc;
use std::time::Instant;
use std::time::Duration;
use std::collections::VecDeque;
use tokio::sync::Mutex;
use chrono::{DateTime, Utc, Local};

#[derive(Debug, Clone)]
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

fn render_landing(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(f.area());

    // Header
    let title = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Charty",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center),
        Line::from(Span::styled(
            "Terminal-based Stock Market Viewer",
            Style::default().fg(Color::Gray),
        ))
        .alignment(Alignment::Center),
    ];

    let header = Paragraph::new(title).block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Main content
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    // Popular stocks list
    let items: Vec<ListItem> = app
        .popular_stocks
        .iter()
        .map(|(ticker, name)| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:8}", ticker),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(name.to_string(), Style::default().fg(Color::White)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Popular Stocks & Indices"),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    f.render_stateful_widget(list, main_chunks[0], &mut app.popular_list_state.clone());

    // Custom search
    let search_text = if app.input_mode {
        vec![
            Line::from(""),
            Line::from("Enter a stock symbol:"),
            Line::from(""),
            Line::from(Span::styled(
                format!("> {}_", app.input_buffer),
                Style::default().fg(Color::Yellow),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press Enter to search, Esc to cancel",
                Style::default().fg(Color::Gray),
            )),
        ]
    } else {
        vec![
            Line::from(""),
            Line::from("Search for any stock:"),
            Line::from(""),
            Line::from(Span::styled(
                "Press 's' to search",
                Style::default().fg(Color::Green),
            )),
            Line::from(""),
            Line::from("Examples:"),
            Line::from("  • AAPL, MSFT, GOOGL"),
            Line::from("  • ^GSPC (S&P 500)"),
            Line::from("  • SPY, QQQ (ETFs)"),
        ]
    };

    let search = Paragraph::new(search_text)
        .block(Block::default().borders(Borders::ALL).title("Custom Search"))
        .alignment(Alignment::Left);
    f.render_widget(search, main_chunks[1]);

    // Footer
    let footer_text = if app.input_mode {
        "Enter: Confirm | Esc: Cancel | q: Quit"
    } else {
        "↑/↓: Navigate | Enter: Select | s: Search | q: Quit"
    };

    let footer = Paragraph::new(footer_text)
        .block(Block::default().borders(Borders::ALL).title("Controls"))
        .alignment(Alignment::Center);
    f.render_widget(footer, chunks[2]);
}

fn render_chart_view(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(5),
        ])
        .split(f.area());

    render_header(f, app, chunks[0]);
    render_chart(f, app, chunks[1]);
    render_footer(f, app, chunks[2]);
}

fn render_header(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    if let Some(ref stock_data) = app.stock_data {
        let price_color = if stock_data.change >= 0.0 {
            Color::Green
        } else {
            Color::Red
        };

        let change_symbol = if stock_data.change >= 0.0 { "▲" } else { "▼" };

        let header_text = vec![Line::from(vec![
            Span::raw(format!("{} ", stock_data.symbol)),
            Span::styled(
                format!("${:.2}", stock_data.current_price),
                Style::default()
                    .fg(price_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!(
                    "{} ${:.2} ({:.2}%)",
                    change_symbol,
                    stock_data.change.abs(),
                    stock_data.change_percent.abs()
                ),
                Style::default().fg(price_color),
            ),
            Span::raw(format!("  [{}]", app.timeframe.display())),
        ])];

        let header = Paragraph::new(header_text)
            .block(Block::default().borders(Borders::ALL).title("Stock Info"));
        f.render_widget(header, area);
    } else if app.loading {
        let loading_text = Paragraph::new("Loading...")
            .block(Block::default().borders(Borders::ALL).title("Stock Info"));
        f.render_widget(loading_text, area);
    }
}

fn format_timestamp(dt: &DateTime<Utc>, timeframe: &TimeFrame) -> String {
    let format_str = match timeframe {
        TimeFrame::OneDay => "%m/%d %H:%M",
        TimeFrame::OneWeek => "%m/%d",
        TimeFrame::OneMonth => "%m/%d",
        TimeFrame::ThreeMonths => "%m/%d",
        TimeFrame::OneYear => "%m/%Y",
    };
    return dt.with_timezone(&Local).format(format_str).to_string();
}

fn render_chart(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    if app.loading {
        let loading = Paragraph::new("Loading stock data...")
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL).title("Chart"));
        f.render_widget(loading, area);
        return;
    }

    if let Some(ref stock_data) = app.stock_data {
        let price_color = if stock_data.change >= 0.0 {
            Color::Green
        } else {
            Color::Red
        };

        let chart_data: Vec<(f64, f64)> = stock_data
            .prices
            .iter()
            .enumerate()
            .map(|(i, &price)| (i as f64, price))
            .collect();

        let max_price = stock_data
            .prices
            .iter()
            .fold(f64::NEG_INFINITY, |a, &b| a.max(b));
        let min_price = stock_data.prices.iter().fold(f64::INFINITY, |a, &b| a.min(b));
        let max_x = (stock_data.prices.len() - 1) as f64;

        let first_date = format_timestamp(stock_data
            .timestamps
            .first()
            .unwrap(), &app.timeframe);
        let last_date = format_timestamp(stock_data
            .timestamps
            .last()
            .unwrap(), &app.timeframe);

        let dataset = Dataset::default()
            .name(stock_data.symbol.as_str())
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(price_color))
            .data(&chart_data);

        let mut x_labels = vec![Span::raw(first_date), Span::raw(last_date)];


        match app.timeframe{
            TimeFrame::OneDay => {
                let mid_idx = chart_data.len() / 2;
                let mid_date = format_timestamp(stock_data.timestamps.get(mid_idx).unwrap(), &app.timeframe);
                x_labels.insert(1, Span::raw(mid_date));
            }
            TimeFrame::OneWeek => {
                let mid_idx = chart_data.len() / 2;
                let mid_date = format_timestamp(stock_data.timestamps.get(mid_idx).unwrap(), &app.timeframe);
                x_labels.insert(1, Span::raw(mid_date));
            },
            TimeFrame::OneMonth => {
                let first_quarter_idx = chart_data.len() / 4;
                let mid_idx = chart_data.len() / 2;
                let third_quarter_idx = chart_data.len() * 3 / 4;
                let first_quarter_date = format_timestamp(stock_data
                    .timestamps
                    .get(first_quarter_idx)
                    .unwrap(),&app.timeframe);
                let mid_date = format_timestamp(stock_data
                    .timestamps
                    .get(mid_idx)
                    .unwrap(), &app.timeframe);
                let third_quarter_date = format_timestamp(stock_data
                    .timestamps
                    .get(third_quarter_idx)
                    .unwrap(), &app.timeframe);
                x_labels.insert(1, Span::raw(first_quarter_date));
                x_labels.insert(2, Span::raw(mid_date));
                x_labels.insert(3, Span::raw(third_quarter_date));
            },
            TimeFrame::ThreeMonths => {
                let first_month_idx = chart_data.len() / 3;
                let second_month_idx = chart_data.len() * 2 / 3;
                let first_month_date = format_timestamp(stock_data
                    .timestamps
                    .get(first_month_idx)
                    .unwrap(), &app.timeframe);
                    
                let second_month_date = format_timestamp(stock_data
                    .timestamps
                    .get(second_month_idx)
                    .unwrap(), &app.timeframe);

                x_labels.insert(1, Span::raw(first_month_date));
                x_labels.insert(2, Span::raw(second_month_date));
            },
            TimeFrame::OneYear => {
                let first_quarter_idx = chart_data.len() / 4;
                let mid_idx = chart_data.len() / 2;
                let third_quarter_idx = chart_data.len() * 3 / 4;
                let first_quarter_date = format_timestamp(stock_data
                    .timestamps
                    .get(first_quarter_idx)
                    .unwrap(), &app.timeframe);

                let mid_date = format_timestamp(stock_data
                    .timestamps
                    .get(mid_idx)
                    .unwrap(), &app.timeframe);   

                let third_quarter_date = format_timestamp(stock_data
                    .timestamps
                    .get(third_quarter_idx)
                    .unwrap(), &app.timeframe);
                
                x_labels.insert(1, Span::raw(first_quarter_date));
                x_labels.insert(2, Span::raw(mid_date));
                x_labels.insert(3, Span::raw(third_quarter_date));
            },
        }

        let y_labels = vec![
            Span::raw(format!("${:.2}", min_price)),
            Span::raw(format!("${:.2}", (min_price + max_price) / 2.0)),
            Span::raw(format!("${:.2}", max_price)),
        ];

        let chart = Chart::new(vec![dataset])
            .block(
                Block::default().borders(Borders::ALL).title(format!(
                    "{} - {}",
                    stock_data.symbol,
                    app.timeframe.display()
                )),
            )
            .x_axis(
                Axis::default()
                    .title("Time")
                    .style(Style::default().fg(Color::Gray))
                    .bounds([0.0, max_x])
                    .labels(x_labels),
            )
            .y_axis(
                Axis::default()
                    .title("Price")
                    .style(Style::default().fg(Color::Gray))
                    .bounds([min_price - 5.0, max_price + 5.0])
                    .labels(y_labels),
            );

        f.render_widget(chart, area);
    } else if let Some(ref error) = app.error_message {
        let error_text = Paragraph::new(error.as_str())
            .style(Style::default().fg(Color::Red))
            .block(Block::default().borders(Borders::ALL).title("Error"));
        f.render_widget(error_text, area);
    }
}

fn render_footer(f: &mut Frame, _app: &App, area: ratatui::layout::Rect) {
    let footer_text = vec![
        Line::from("Controls:"),
        Line::from("'b': Back | 's': Search | '←/→': Timeframe | 'l': Live Mode | 'r': Refresh | 'q': Quit"),
    ];

    let footer = Paragraph::new(footer_text)
        .block(Block::default().borders(Borders::ALL).title("Controls"));
    f.render_widget(footer, area);
}

fn render_error_log(f: &mut Frame, app: &App) {
    // Create centered popup area
    let area = f.area();
    let popup_width = area.width.min(60);
    let popup_height = area.height.min(15);
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = ratatui::layout::Rect {
        x: popup_x,
        y: popup_y,
        width: popup_width,
        height: popup_height,
    };

    // Render error log content
    let error_items: Vec<ListItem> = if app.ws_error_log.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No errors logged yet",
            Style::default().fg(Color::Gray),
        )))]
    } else {
        app.ws_error_log
            .iter()
            .map(|error| {
                ListItem::new(Line::from(Span::styled(
                    error.clone(),
                    Style::default().fg(Color::Red),
                )))
            })
            .collect()
    };

    let error_list = List::new(error_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("WebSocket Error Log (ESC to close)")
            .style(Style::default().bg(Color::Black)),
    );

    f.render_widget(error_list, popup_area);
}

fn render_live_mode_select(f: &mut Frame) {
    let area = f.area();
    let popup_width = 40;
    let popup_height = 9;
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = ratatui::layout::Rect {
        x: popup_x,
        y: popup_y,
        width: popup_width,
        height: popup_height,
    };

    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Select Live Mode",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(" [1] ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw("Live Ticker (Trade Feed)"),
        ]),
        Line::from(vec![
            Span::styled(" [2] ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw("Live Candles (1min OHLC)"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Press ESC to cancel",
            Style::default().fg(Color::Gray),
        )),
    ];

    let popup = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Live Mode")
                .style(Style::default().bg(Color::Black)),
        );

    f.render_widget(popup, popup_area);
}

fn render_live_ticker(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(f.area());

    // Header with current price
    render_live_header(f, app, chunks[0], "LIVE TICKER");

    // Trade feed
    let trades: Vec<ListItem> = if app.live_trades.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "Waiting for trades...",
            Style::default().fg(Color::Gray),
        )))]
    } else {
        app.live_trades
            .iter()
            .map(|trade| {
                let time = trade.timestamp.with_timezone(&Local).format("%H:%M:%S").to_string();
                let direction = if let Some(prev) = app.live_trades.get(1) {
                    if trade.price > prev.price {
                        Span::styled(" ↑ ", Style::default().fg(Color::Green))
                    } else if trade.price < prev.price {
                        Span::styled(" ↓ ", Style::default().fg(Color::Red))
                    } else {
                        Span::styled(" - ", Style::default().fg(Color::Gray))
                    }
                } else {
                    Span::styled(" - ", Style::default().fg(Color::Gray))
                };

                let vol_str = match trade.volume {
                    Some(v) if v > 0 => format!("{:>8}", format_volume(v)),
                    _ => "        ".to_string(),
                };

                ListItem::new(Line::from(vec![
                    Span::styled(time, Style::default().fg(Color::DarkGray)),
                    Span::raw("  "),
                    Span::styled(
                        format!("${:<10.2}", trade.price),
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                    ),
                    direction,
                    Span::styled(vol_str, Style::default().fg(Color::Cyan)),
                ]))
            })
            .collect()
    };

    let trades_list = List::new(trades).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Recent Trades ({})", app.total_trade_count)),
    );
    f.render_widget(trades_list, chunks[1]);

    // Footer
    render_live_footer(f, chunks[2]);
}

fn render_live_candles(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(0),
            Constraint::Length(5),
        ])
        .split(f.area());

    // Header with current price
    render_live_header(f, app, chunks[0], "LIVE CANDLES (1min)");

    // Candlestick chart area
    let chart_area = chunks[1];

    // Build all candles including current
    let mut all_candles: Vec<&Candlestick> = app.live_candles.iter().collect();
    if let Some(ref current) = app.current_candle {
        all_candles.push(current);
    }

    if all_candles.is_empty() {
        let waiting = Paragraph::new("Waiting for trades to build candles...")
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title("Candlesticks"));
        f.render_widget(waiting, chart_area);
    } else {
        // Render candlestick chart
        render_candlestick_chart(f, chart_area, &all_candles, app.current_candle.is_some());
    }

    // Footer with OHLC info
    render_candle_footer(f, app, chunks[2]);
}

fn render_live_header(f: &mut Frame, app: &App, area: ratatui::layout::Rect, mode_name: &str) {
    let price = app.last_live_price.unwrap_or(0.0);
    let (change, change_pct) = if let Some(ref data) = app.stock_data {
        (data.change, data.change_percent)
    } else {
        (0.0, 0.0)
    };

    let price_color = if change >= 0.0 { Color::Green } else { Color::Red };
    let change_symbol = if change >= 0.0 { "▲" } else { "▼" };

    let status_span = match &app.ws_status {
        WebSocketStatus::Connected { since } => {
            let secs = Utc::now().signed_duration_since(*since).num_seconds();
            Span::styled(format!("[● {}s]", secs), Style::default().fg(Color::Green))
        }
        WebSocketStatus::Connecting => {
            Span::styled("[CONNECTING...]", Style::default().fg(Color::Yellow))
        }
        WebSocketStatus::Reconnecting { attempt, .. } => {
            Span::styled(format!("[RECONNECTING {}/5]", attempt), Style::default().fg(Color::Yellow))
        }
        _ => Span::styled("[DISCONNECTED]", Style::default().fg(Color::Gray)),
    };

    let header_text = vec![
        Line::from(vec![
            Span::styled(
                format!("{} - {} ", app.symbol, mode_name),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            status_span,
        ]),
        Line::from(vec![
            Span::styled(
                format!("${:.2}", price),
                Style::default().fg(price_color).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{} ${:.2} ({:.2}%)", change_symbol, change.abs(), change_pct.abs()),
                Style::default().fg(price_color),
            ),
            Span::raw("  "),
            Span::styled(
                format!("Vol: {}", format_volume(app.total_live_volume)),
                Style::default().fg(Color::Cyan),
            ),
        ]),
    ];

    let header = Paragraph::new(header_text)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, area);
}

fn render_live_footer(f: &mut Frame, area: ratatui::layout::Rect) {
    let footer = Paragraph::new("'l': Switch Mode | 'h': Historical | 'e': Errors | 'q': Quit")
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Controls"));
    f.render_widget(footer, area);
}

fn render_candle_footer(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let ohlc_line = if let Some(ref candle) = app.current_candle {
        Line::from(vec![
            Span::styled("Current: ", Style::default().fg(Color::Gray)),
            Span::styled(format!("O:{:.2} ", candle.open), Style::default().fg(Color::White)),
            Span::styled(format!("H:{:.2} ", candle.high), Style::default().fg(Color::Green)),
            Span::styled(format!("L:{:.2} ", candle.low), Style::default().fg(Color::Red)),
            Span::styled(format!("C:{:.2} ", candle.close), Style::default().fg(Color::Cyan)),
            Span::styled(format!("Ticks:{}", candle.trade_count), Style::default().fg(Color::DarkGray)),
        ])
    } else {
        Line::from(Span::styled("Waiting for candle data...", Style::default().fg(Color::Gray)))
    };

    let footer_text = vec![
        ohlc_line,
        Line::from(""),
        Line::from("'l': Switch Mode | 'h': Historical | 'e': Errors | 'q': Quit"),
    ];

    let footer = Paragraph::new(footer_text)
        .block(Block::default().borders(Borders::ALL).title("Controls"));
    f.render_widget(footer, area);
}

fn render_candlestick_chart(f: &mut Frame, area: ratatui::layout::Rect, candles: &[&Candlestick], has_current: bool) {
    let inner = Block::default().borders(Borders::ALL).title("Candlesticks");
    let inner_area = inner.inner(area);
    f.render_widget(inner, area);

    if candles.is_empty() || inner_area.width < 5 || inner_area.height < 3 {
        return;
    }

    // Find price range
    let mut min_price = f64::INFINITY;
    let mut max_price = f64::NEG_INFINITY;
    for candle in candles {
        min_price = min_price.min(candle.low);
        max_price = max_price.max(candle.high);
    }

    // Add some padding to price range
    let price_range = max_price - min_price;
    let padding = if price_range > 0.0 { price_range * 0.1 } else { 1.0 };
    min_price -= padding;
    max_price += padding;

    let height = inner_area.height as f64;
    let width = inner_area.width as usize;

    // Calculate how many candles we can show (2 chars per candle + 1 space)
    let candle_width = 3;
    let max_candles = width / candle_width;
    let candles_to_show = candles.len().min(max_candles);
    let start_idx = candles.len().saturating_sub(candles_to_show);
    let visible_candles = &candles[start_idx..];

    // Render each row
    for row in 0..inner_area.height {
        let y = inner_area.y + row;
        let price_at_row = max_price - ((row as f64 / height) * (max_price - min_price));

        let mut spans = Vec::new();

        for (i, candle) in visible_candles.iter().enumerate() {
            let is_current = has_current && i == visible_candles.len() - 1;
            let is_bullish = candle.close >= candle.open;

            let body_top = candle.open.max(candle.close);
            let body_bottom = candle.open.min(candle.close);

            let char_str = if price_at_row >= candle.low && price_at_row <= candle.high {
                if price_at_row >= body_bottom && price_at_row <= body_top {
                    // Body
                    "█"
                } else {
                    // Wick
                    "│"
                }
            } else {
                " "
            };

            let color = if is_current {
                Color::Yellow
            } else if is_bullish {
                Color::Green
            } else {
                Color::Red
            };

            spans.push(Span::styled(format!(" {}", char_str), Style::default().fg(color)));
        }

        let line = Line::from(spans);
        f.render_widget(
            Paragraph::new(vec![line]),
            ratatui::layout::Rect {
                x: inner_area.x,
                y,
                width: inner_area.width,
                height: 1,
            },
        );
    }
}

fn format_volume(vol: u64) -> String {
    if vol >= 1_000_000 {
        format!("{:.1}M", vol as f64 / 1_000_000.0)
    } else if vol >= 1_000 {
        format!("{:.1}K", vol as f64 / 1_000.0)
    } else {
        format!("{}", vol)
    }
}