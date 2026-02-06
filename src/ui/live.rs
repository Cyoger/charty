use ratatui::{
	layout::{Constraint, Direction, Layout, Alignment},
	widgets::{Block, Borders, Paragraph, List, ListItem},
	style::{Style, Color, Modifier},
	text::{Line, Span},
	Frame,
};

use chrono::{Utc, Local};

use super::{App, WebSocketStatus, Candlestick};

pub fn render_live_ticker(f: &mut Frame, app: &App) {
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

pub fn render_live_candles(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(0),
            Constraint::Length(5),
        ])
        .split(f.area());

    // Header with current price
    let header_title = format!("LIVE CANDLES ({})", app.candle_interval.to_string());
    render_live_header(f, app, chunks[0], &header_title);

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
    let footer = Paragraph::new("'b': Back | 'l': Switch | 'h': Help | 'e': Errors | 'q': Quit")
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Controls"));
    f.render_widget(footer, area);
}

pub fn render_live_mode_select(f: &mut Frame) {
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
        Line::from("'←/→': Interval | 'b': Back | 'l': Switch | 'h': Help | 'e': Errors | 'q': Quit"),
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


pub fn render_error_log(f: &mut Frame, app: &App) {
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


fn format_volume(vol: u64) -> String {
    if vol >= 1_000_000 {
        format!("{:.1}M", vol as f64 / 1_000_000.0)
    } else if vol >= 1_000 {
        format!("{:.1}K", vol as f64 / 1_000.0)
    } else {
        format!("{}", vol)
    }
}