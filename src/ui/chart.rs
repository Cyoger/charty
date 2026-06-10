use ratatui::{
	layout::{Constraint, Direction, Layout, Alignment, Rect},
	widgets::{Block, Borders, Paragraph, Chart, Dataset, Axis, GraphType},
	symbols,
	style::{Style, Color, Modifier},
	text::{Line, Span},
	Frame,
};
use chrono::{DateTime, Utc, Local};

use super::{App, Candlestick, nav_key};
use crate::stock::{TimeFrame, MarketState};

pub fn render_chart_view(f: &mut Frame, app: &App) {
    let show_vol = app.show_volume && app.stock_data.is_some();

    let constraints = if show_vol {
        vec![
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(7),
            Constraint::Length(5),
        ]
    } else {
        vec![
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(5),
        ]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(f.area());

    render_header(f, app, chunks[0]);
    render_chart(f, app, chunks[1]);
    if show_vol {
        // Mirror ratatui's internal graph_area.left() calculation so bars align exactly.
        let offset = graph_left_offset(app, chunks[1]);
        render_volume_bars(f, app, chunks[2], offset);
        render_footer(f, app, chunks[3]);
    } else {
        render_footer(f, app, chunks[2]);
    }
}

/// Replicates ratatui's Chart::layout() to find how many columns are consumed
/// to the left of the actual plot area (y-axis labels + the axis line itself).
fn graph_left_offset(app: &App, chart_area: Rect) -> u16 {
    let Some(ref data) = app.stock_data else { return 0; };
    if data.prices.is_empty() || data.timestamps.is_empty() { return 0; }

    let max_price = data.prices.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let min_price = data.prices.iter().cloned().fold(f64::INFINITY,     f64::min);

    // Same three y-labels used in render_chart
    let y_label_w = [
        format!("${:.2}", min_price).len() as u16,
        format!("${:.2}", (min_price + max_price) / 2.0).len() as u16,
        format!("${:.2}", max_price).len() as u16,
    ]
    .into_iter()
    .max()
    .unwrap_or(0);

    // First x-label width (Alignment::Left, has_y_axis=true → subtract 1)
    let first_x_w = format_timestamp(data.timestamps.first().unwrap(), &app.timeframe)
        .len() as u16;
    let x_contribution = first_x_w.saturating_sub(1);

    // chart inner width (block has Borders::ALL → −2)
    let inner_w = chart_area.width.saturating_sub(2);

    // ratatui clamps to 1/3 of inner width
    let labels_w = y_label_w.max(x_contribution).min(inner_w / 3);

    // +1 for the y-axis vertical line (axis_y = Some(x); x += 1)
    labels_w + 1
}

fn render_header(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    if let Some(ref stock_data) = app.stock_data {
        let price_color = if stock_data.change >= 0.0 {
            Color::Green
        } else {
            Color::Red
        };

        let change_symbol = if stock_data.change >= 0.0 { "▲" } else { "▼" };

        let (market_badge, badge_color) = match stock_data.market_state {
            MarketState::Regular => (Some(" ● Market Open"), Color::Green),
            MarketState::Pre    => (Some(" ◑ Pre-Market"), Color::Yellow),
            MarketState::Post   => (Some(" ☾ After Hours"), Color::Yellow),
            MarketState::Closed => (Some(" ● Market Closed"), Color::DarkGray),
        };

        let mut spans = vec![
            Span::raw(format!("{} ", stock_data.symbol)),
            Span::styled(
                format!("${:.2}", stock_data.current_price),
                Style::default().fg(price_color).add_modifier(Modifier::BOLD),
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
        ];

        if let Some(badge) = market_badge {
            spans.push(Span::styled(badge, Style::default().fg(badge_color)));
        }

        if app.show_sma {
            spans.push(Span::styled("  SMA20", Style::default().fg(Color::Cyan)));
            spans.push(Span::styled("  SMA50", Style::default().fg(Color::Yellow)));
        }

        let header = Paragraph::new(Line::from(spans))
            .block(Block::default().borders(Borders::ALL).title("Stock Info"));
        f.render_widget(header, area);
    } else if app.loading {
        let loading_text = Paragraph::new("Loading...")
            .block(Block::default().borders(Borders::ALL).title("Stock Info"));
        f.render_widget(loading_text, area);
    }
}

fn compute_sma(prices: &[f64], period: usize) -> Vec<(f64, f64)> {
    if prices.len() < period {
        return Vec::new();
    }
    prices
        .windows(period)
        .enumerate()
        .map(|(i, w)| ((i + period - 1) as f64, w.iter().sum::<f64>() / period as f64))
        .collect()
}

fn render_chart(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    if app.loading {
        let loading = Paragraph::new("Loading stock data...")
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL).title("Chart"));
        f.render_widget(loading, area);
        return;
    }

    // Candlestick path
    if app.show_candlesticks {
        if let Some(ref stock_data) = app.stock_data {
            let candles = app.convert_to_candlesticks();
            if !candles.is_empty() {
                let title = format!(
                    "{} - {} (Candlesticks: {})",
                    stock_data.symbol,
                    app.timeframe.display(),
                    app.candle_interval.to_string()
                );
                let first_ts = candles.first().unwrap().timestamp.clone();
                let last_ts  = candles.last().unwrap().timestamp.clone();
                let x_labels = vec![
                    Span::raw(format_timestamp(&first_ts, &app.timeframe)),
                    Span::raw(format_timestamp(&last_ts,  &app.timeframe)),
                ];
                render_candlestick_chart(f, &candles, area, title, x_labels, &stock_data.symbol);
                return;
            }
        }
    }

    if let Some(ref stock_data) = app.stock_data {
        let price_color = if stock_data.change >= 0.0 { Color::Green } else { Color::Red };

        let chart_data: Vec<(f64, f64)>;
        let max_price: f64;
        let min_price: f64;
        let max_x: f64;
        let first_ts: DateTime<Utc>;
        let last_ts: DateTime<Utc>;

        // Regular line chart
        chart_data = stock_data
            .prices
            .iter()
            .enumerate()
            .map(|(i, &p)| (i as f64, p))
            .collect();
        max_price = stock_data.prices.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        min_price = stock_data.prices.iter().cloned().fold(f64::INFINITY,     f64::min);
        max_x     = (stock_data.prices.len() - 1) as f64;
        first_ts  = *stock_data.timestamps.first().unwrap();
        last_ts   = *stock_data.timestamps.last().unwrap();

        // Pre-compute SMA data (must outlive the datasets vec)
        let sma20_data = if app.show_sma { compute_sma(&stock_data.prices, 20) } else { Vec::new() };
        let sma50_data = if app.show_sma { compute_sma(&stock_data.prices, 50) } else { Vec::new() };

        let mut datasets = vec![
            Dataset::default()
                .name(stock_data.symbol.as_str())
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(price_color))
                .data(&chart_data),
        ];

        if app.show_sma {
            if !sma20_data.is_empty() {
                datasets.push(
                    Dataset::default()
                        .name("SMA20")
                        .marker(symbols::Marker::Braille)
                        .graph_type(GraphType::Line)
                        .style(Style::default().fg(Color::Cyan))
                        .data(&sma20_data),
                );
            }
            if !sma50_data.is_empty() {
                datasets.push(
                    Dataset::default()
                        .name("SMA50")
                        .marker(symbols::Marker::Braille)
                        .graph_type(GraphType::Line)
                        .style(Style::default().fg(Color::Yellow))
                        .data(&sma50_data),
                );
            }
        }

        let first_date = format_timestamp(&first_ts, &app.timeframe);
        let last_date  = format_timestamp(&last_ts,  &app.timeframe);
        let mut x_labels = vec![Span::raw(first_date), Span::raw(last_date)];

        let data_len = stock_data.timestamps.len();
        match app.timeframe {
            TimeFrame::OneDay | TimeFrame::OneWeek => {
                let mid = format_timestamp(stock_data.timestamps.get(data_len / 2).unwrap(), &app.timeframe);
                x_labels.insert(1, Span::raw(mid));
            }
            TimeFrame::OneMonth | TimeFrame::OneYear => {
                let q1  = format_timestamp(stock_data.timestamps.get(data_len / 4).unwrap(),     &app.timeframe);
                let mid = format_timestamp(stock_data.timestamps.get(data_len / 2).unwrap(),     &app.timeframe);
                let q3  = format_timestamp(stock_data.timestamps.get(data_len * 3 / 4).unwrap(), &app.timeframe);
                x_labels.insert(1, Span::raw(q1));
                x_labels.insert(2, Span::raw(mid));
                x_labels.insert(3, Span::raw(q3));
            }
            TimeFrame::ThreeMonths => {
                let t1  = format_timestamp(stock_data.timestamps.get(data_len / 3).unwrap(),     &app.timeframe);
                let t2  = format_timestamp(stock_data.timestamps.get(data_len * 2 / 3).unwrap(), &app.timeframe);
                x_labels.insert(1, Span::raw(t1));
                x_labels.insert(2, Span::raw(t2));
            }
        }

        let y_labels = vec![
            Span::raw(format!("${:.2}", min_price)),
            Span::raw(format!("${:.2}", (min_price + max_price) / 2.0)),
            Span::raw(format!("${:.2}", max_price)),
        ];

        let title = if app.show_sma {
            format!("{} - {}  SMA20 SMA50", stock_data.symbol, app.timeframe.display())
        } else {
            format!("{} - {}", stock_data.symbol, app.timeframe.display())
        };

        let chart = Chart::new(datasets)
            .block(Block::default().borders(Borders::ALL).title(title))
            .x_axis(
                Axis::default()
                    .style(Style::default().fg(Color::Gray))
                    .bounds([0.0, max_x])
                    .labels(x_labels),
            )
            .y_axis(
                Axis::default()
                    .style(Style::default().fg(Color::Gray))
                    .bounds([min_price - 5.0, max_price + 5.0])
                    .labels(y_labels),
            );

        f.render_widget(chart, area);
    } else if let Some(ref error) = app.error_message {
        let error_text = Paragraph::new(error.as_str())
            .style(Style::default().fg(Color::Red))
            .wrap(ratatui::widgets::Wrap { trim: true })
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title("Error"));
        f.render_widget(error_text, area);
    }
}

fn render_volume_bars(f: &mut Frame, app: &App, area: Rect, left_offset: u16) {
    let Some(ref data) = app.stock_data else { return; };
    if data.volumes.is_empty() { return; }

    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .title("Volume");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let bar_height     = inner.height as usize;
    let inner_width    = inner.width as usize;
    let offset         = left_offset as usize;
    if bar_height == 0 || inner_width == 0 { return; }

    let bar_area_width = inner_width.saturating_sub(offset);
    if bar_area_width == 0 { return; }

    let n = data.prices.len().min(data.volumes.len());
    if n == 0 { return; }

    let scale_vol = data.volumes.iter().cloned().fold(0.0f64, f64::max);
    if scale_vol == 0.0 { return; }

    // Mirror ratatui's x-axis mapping: data index i → pixel i*(width-1)/(n-1)
    // so bar at column col uses data index col*(n-1)/(width-1)
    let bars: Vec<(f64, bool)> = (0..bar_area_width)
        .map(|col| {
            let i = if bar_area_width > 1 && n > 1 {
                (col * (n - 1) / (bar_area_width - 1)).min(n - 1)
            } else {
                0
            };
            let is_up = i == 0 || data.prices[i] >= data.prices[i - 1];
            (data.volumes[i], is_up)
        })
        .collect();

    // Draw a visual axis line at position (offset-1) so the volume section visually
    // shares the same y-axis line as the chart above it.
    let pre_axis = " ".repeat(offset.saturating_sub(1));
    let mut lines: Vec<Line> = Vec::new();
    for row in 0..bar_height {
        let from_bottom = bar_height - 1 - row;
        let mut spans = vec![
            Span::raw(pre_axis.clone()),
            Span::styled("│", Style::default().fg(Color::DarkGray)),
        ];
        for &(vol, is_up) in &bars {
            // Compute height in eighths for sub-row precision
            let total_eighths = ((vol / scale_vol) * bar_height as f64 * 8.0) as usize;
            let full_rows     = total_eighths / 8;
            let partial       = total_eighths % 8;
            let color = if is_up { Color::Green } else { Color::Red };

            let ch: &'static str = if from_bottom < full_rows {
                "█"
            } else if from_bottom == full_rows && partial > 0 {
                match partial {
                    1 => "▁", 2 => "▂", 3 => "▃", 4 => "▄",
                    5 => "▅", 6 => "▆", 7 => "▇", _ => " ",
                }
            } else {
                " "
            };

            if ch == " " {
                spans.push(Span::raw(" "));
            } else {
                spans.push(Span::styled(ch, Style::default().fg(color)));
            }
        }
        lines.push(Line::from(spans));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn render_footer(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    use ratatui::layout::{Constraint, Direction, Layout};

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(2)])
        .split(area);

    // Row 1 — shared nav bar with toggle indicators for v/i
    let vol_style = if app.show_volume {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    };
    let sma_style = if app.show_sma {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    };

    let nav = Line::from(vec![
        nav_key("←/→"), Span::raw(" Timeframe   "),
        nav_key("l"),   Span::raw(" Live   "),
        nav_key("w"),   Span::raw(" Watchlist   "),
        nav_key("a"),   Span::raw(" Alert   "),
        nav_key("r"),   Span::raw(" Refresh   "),
        Span::styled("v", vol_style), Span::raw(" Vol   "),
        Span::styled("i", sma_style), Span::raw(" SMA   "),
        nav_key("s"),   Span::raw(" Search   "),
        nav_key("b"),   Span::raw(" Back   "),
        nav_key("q"),   Span::raw(" Quit"),
    ]);
    let nav_bar = Paragraph::new(nav)
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Center);
    f.render_widget(nav_bar, chunks[0]);

    // Row 2 — alert status
    let alert_line = if let Some(alert) = app.alert_for_symbol(&app.symbol) {
        if alert.triggered {
            Line::from(Span::styled(
                format!("  ⚡ {} crossed ${:.2} — press a to clear", alert.symbol, alert.target),
                Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD),
            ))
        } else {
            let direction = if alert.above { "↑" } else { "↓" };
            Line::from(Span::styled(
                format!("  Alert: ${:.2} {}  (a: clear)", alert.target, direction),
                Style::default().fg(Color::Yellow),
            ))
        }
    } else {
        Line::from(Span::styled("  No alert set", Style::default().fg(Color::DarkGray)))
    };
    f.render_widget(Paragraph::new(alert_line), chunks[1]);
}

fn format_timestamp(dt: &DateTime<Utc>, timeframe: &TimeFrame) -> String {
    let fmt = match timeframe {
        TimeFrame::OneDay => "%m/%d %H:%M",
        TimeFrame::OneWeek | TimeFrame::OneMonth | TimeFrame::ThreeMonths => "%m/%d",
        TimeFrame::OneYear => "%m/%Y",
    };
    dt.with_timezone(&Local).format(fmt).to_string()
}

fn render_candlestick_chart(f: &mut Frame, candles: &[Candlestick], area: Rect, title: String, x_labels: Vec<Span>, _symbol: &str) {
    if candles.is_empty() { return; }

    let max_price  = candles.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
    let min_price  = candles.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    let price_range = max_price - min_price;
    if price_range == 0.0 { return; }

    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chart_height  = inner.height.saturating_sub(3) as usize;
    let chart_width   = inner.width.saturating_sub(10) as usize;
    if chart_height == 0 || chart_width == 0 { return; }

    let max_candles   = chart_width / 2;
    let display_start = if candles.len() > max_candles { candles.len() - max_candles } else { 0 };
    let displayed     = &candles[display_start..];
    let candle_width  = if !displayed.is_empty() { (chart_width / displayed.len()).max(2) } else { 2 };

    let price_label_rows   = [0, chart_height / 4, chart_height / 2, chart_height * 3 / 4, chart_height.saturating_sub(1)];
    let price_label_values = [
        format!("${:.2}", max_price),
        format!("${:.2}", max_price - price_range * 0.25),
        format!("${:.2}", max_price - price_range * 0.5),
        format!("${:.2}", max_price - price_range * 0.75),
        format!("${:.2}", min_price),
    ];

    let price_to_row = |price: f64| -> usize {
        let norm = (max_price - price) / price_range;
        ((norm * chart_height as f64) as usize).min(chart_height - 1)
    };

    let mut lines: Vec<Line> = Vec::new();
    for row in 0..chart_height {
        let mut spans = Vec::new();

        let label = price_label_rows.iter().position(|&r| r == row)
            .and_then(|idx| price_label_values.get(idx));
        if let Some(lbl) = label {
            spans.push(Span::styled(format!("{:>8} ", lbl), Style::default().fg(Color::Gray)));
        } else {
            spans.push(Span::raw("         "));
        }

        for candle in displayed {
            let is_bullish   = candle.close >= candle.open;
            let color        = if is_bullish { Color::Green } else { Color::Red };
            let body_top     = candle.open.max(candle.close);
            let body_bottom  = candle.open.min(candle.close);
            let high_row     = price_to_row(candle.high);
            let low_row      = price_to_row(candle.low);
            let body_top_row = price_to_row(body_top);
            let body_bot_row = price_to_row(body_bottom);

            let (ch, col) = if row >= high_row && row <= low_row {
                if row >= body_top_row && row <= body_bot_row { ("█", color) } else { ("│", color) }
            } else {
                (" ", Color::White)
            };
            spans.push(Span::styled(ch.repeat(candle_width.min(3)), Style::default().fg(col)));
        }
        lines.push(Line::from(spans));
    }

    let time_line = Line::from(vec![
        Span::raw("         "),
        Span::styled(
            format!("{:width$}", x_labels.first().map(|s| s.content.as_ref()).unwrap_or(""), width = chart_width / 3),
            Style::default().fg(Color::Gray),
        ),
        Span::styled(
            format!("{:^width$}", x_labels.get(x_labels.len() / 2).map(|s| s.content.as_ref()).unwrap_or(""), width = chart_width / 3),
            Style::default().fg(Color::Gray),
        ),
        Span::styled(
            format!("{:>width$}", x_labels.last().map(|s| s.content.as_ref()).unwrap_or(""), width = chart_width / 3),
            Style::default().fg(Color::Gray),
        ),
    ]);
    lines.push(Line::from(""));
    lines.push(time_line);

    f.render_widget(Paragraph::new(lines), inner);
}
