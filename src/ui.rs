use ratatui::{
    layout::{Constraint, Direction, Layout, Alignment},
    widgets::{Block, Borders, Paragraph, Chart, Dataset, Axis, GraphType, List, ListItem, ListState},
    symbols,
    style::{Style, Color, Modifier},
    text::{Line, Span},
    Frame,
};

use crate::stock::StockData;
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
            update_throttle: UpdateThrottle::new(Duration::from_millis(500)),
            show_error_log: false,
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

    pub fn update_live_price(&mut self, price: f64) {
        self.last_live_price = Some(price);
        self.ws_last_update = Some(Utc::now());

        if let Some(ref mut data) = self.stock_data {
            // Update live price
            data.live_current_price = Some(price);
            data.current_price = price;

            // Add to live ticks (separate from historical data)
            data.live_ticks.push_back(crate::stock::LiveTick {
                price,
                timestamp: Utc::now(),
            });

            // Keep only last 100 live ticks
            if data.live_ticks.len() > 100 {
                data.live_ticks.pop_front();
            }

            // Calculate change from base historical price, not first historical price
            data.change = price - data.base_historical_price;
            data.change_percent = (data.change / data.base_historical_price) * 100.0;
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

    // Render error log popup if requested
    if app.show_error_log {
        render_error_log(f, app);
    }
}

fn render_header(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    if let Some(ref stock_data) = app.stock_data {
        let price_color = if stock_data.change >= 0.0 {
            Color::Green
        } else {
            Color::Red
        };

        let change_symbol = if stock_data.change >= 0.0 { "▲" } else { "▼" };

        let live_indicator = if app.live_updates_enabled {
            match &app.ws_status {
                WebSocketStatus::Connected { since } => {
                    let duration = Utc::now().signed_duration_since(*since);
                    let seconds = duration.num_seconds();
                    Span::styled(
                        format!(" [LIVE ● {}s]", seconds),
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )
                }
                WebSocketStatus::Connecting => {
                    Span::styled(
                        " [CONNECTING...]",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )
                }
                WebSocketStatus::Reconnecting { attempt, next_retry_in } => {
                    let secs = next_retry_in.as_secs();
                    Span::styled(
                        format!(" [RECONNECTING {}/5 ({}s)]", attempt, secs),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )
                }
                WebSocketStatus::Error { message, recoverable } => {
                    let display_msg = if message.len() > 20 {
                        format!("{}...", &message[..17])
                    } else {
                        message.clone()
                    };
                    let color = if *recoverable { Color::Yellow } else { Color::Red };
                    Span::styled(
                        format!(" [ERROR: {}]", display_msg),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    )
                }
                WebSocketStatus::Disconnected => {
                    Span::styled(
                        " [DISCONNECTED]",
                        Style::default().fg(Color::Gray),
                    )
                }
                WebSocketStatus::Idle => {
                    Span::styled(
                        " [PAUSED]",
                        Style::default().fg(Color::Gray),
                    )
                }
            }
        } else {
            Span::styled(
                " [PAUSED]",
                Style::default().fg(Color::Gray)
            )
        };

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
            live_indicator,
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

        let first_date = stock_data
            .timestamps
            .first()
            .unwrap()
            .with_timezone(&Local)
            .format("%m/%d %H:%M")
            .to_string();
        let last_date = stock_data
            .timestamps
            .last()
            .unwrap()
            .with_timezone(&Local)
            .format("%m/%d %H:%M")
            .to_string();

        let dataset = Dataset::default()
            .name(stock_data.symbol.as_str())
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(price_color))
            .data(&chart_data);

        let x_labels = vec![Span::raw(first_date), Span::raw(last_date)];

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
        Line::from("'b': Back | 's': Symbol | '←/→': Timeframe | 'l': Live | 'r': Refresh | 'e': Errors | 'q': Quit"),
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