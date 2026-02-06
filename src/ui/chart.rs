use ratatui::{
	layout::{Constraint, Direction, Layout, Alignment, Rect},
	widgets::{Block, Borders, Paragraph, Chart, Dataset, Axis, GraphType},
	symbols,
	style::{Style, Color, Modifier},
	text::{Line, Span},
	Frame,
};
use chrono::{DateTime, Utc, Local};

use super::{App, Candlestick};
use crate::stock::TimeFrame;

pub fn render_chart_view(f: &mut Frame, app: &App) {
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

fn render_chart(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    if app.loading {
        let loading = Paragraph::new("Loading stock data...")
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL).title("Chart"));
        f.render_widget(loading, area);
        return;
    }

    // Check if we should render candlesticks
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
                let last_ts = candles.last().unwrap().timestamp.clone();
                let first_date = format_timestamp(&first_ts, &app.timeframe);
                let last_date = format_timestamp(&last_ts, &app.timeframe);
                let x_labels = vec![Span::raw(first_date), Span::raw(last_date)];

                render_candlestick_chart(f, &candles, area, title, x_labels, &stock_data.symbol);
                return;
            }
        }
    }

    if let Some(ref stock_data) = app.stock_data {
        let price_color = if stock_data.change >= 0.0 {
            Color::Green
        } else {
            Color::Red
        };

        // Prepare chart data based on mode
        let chart_data: Vec<(f64, f64)>;
        let candlestick_data: Vec<(f64, f64)>;
        let max_price: f64;
        let min_price: f64;
        let max_x: f64;
        let first_ts: DateTime<Utc>;
        let last_ts: DateTime<Utc>;

        if app.show_candlesticks {
            // Convert to candlesticks and render as OHLC bars
            let candles = app.convert_to_candlesticks();
            if candles.is_empty() {
                // Fallback to regular chart if no candles
                chart_data = stock_data
                    .prices
                    .iter()
                    .enumerate()
                    .map(|(i, &price)| (i as f64, price))
                    .collect();
                max_price = stock_data.prices.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
                min_price = stock_data.prices.iter().fold(f64::INFINITY, |a, &b| a.min(b));
                max_x = (stock_data.prices.len() - 1) as f64;
                first_ts = stock_data.timestamps.first().unwrap().clone();
                last_ts = stock_data.timestamps.last().unwrap().clone();
                candlestick_data = Vec::new();
            } else {
                // Create OHLC bar representation - plot high-low ranges for each candle
                let mut all_points = Vec::new();
                for (i, candle) in candles.iter().enumerate() {
                    let x = i as f64;
                    // Create vertical bar from low to high
                    all_points.push((x, candle.low));
                    all_points.push((x, candle.high));
                    // Add close point with offset for visibility
                    all_points.push((x + 0.1, candle.close));
                    all_points.push((x - 0.1, candle.open));
                }

                candlestick_data = all_points;
                max_price = candles.iter().map(|c| c.high).fold(f64::NEG_INFINITY, |a, b| a.max(b));
                min_price = candles.iter().map(|c| c.low).fold(f64::INFINITY, |a, b| a.min(b));
                max_x = (candles.len() - 1) as f64;
                first_ts = candles.first().unwrap().timestamp.clone();
                last_ts = candles.last().unwrap().timestamp.clone();
                chart_data = Vec::new();
            }
        } else {
            // Regular line chart
            chart_data = stock_data
                .prices
                .iter()
                .enumerate()
                .map(|(i, &price)| (i as f64, price))
                .collect();
            max_price = stock_data.prices.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
            min_price = stock_data.prices.iter().fold(f64::INFINITY, |a, &b| a.min(b));
            max_x = (stock_data.prices.len() - 1) as f64;
            first_ts = stock_data.timestamps.first().unwrap().clone();
            last_ts = stock_data.timestamps.last().unwrap().clone();
            candlestick_data = Vec::new();
        }

        // Create datasets after data is prepared
        let datasets: Vec<Dataset> = if app.show_candlesticks && !candlestick_data.is_empty() {
            vec![Dataset::default()
                .name(stock_data.symbol.as_str())
                .marker(symbols::Marker::Dot)
                .graph_type(GraphType::Scatter)
                .style(Style::default().fg(price_color))
                .data(&candlestick_data)]
        } else {
            vec![Dataset::default()
                .name(stock_data.symbol.as_str())
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(price_color))
                .data(&chart_data)]
        };

        let first_date = format_timestamp(&first_ts, &app.timeframe);
        let last_date = format_timestamp(&last_ts, &app.timeframe);

        let mut x_labels = vec![Span::raw(first_date), Span::raw(last_date)];


        let data_len = stock_data.timestamps.len();
        match app.timeframe{
            TimeFrame::OneDay => {
                let mid_idx = data_len / 2;
                let mid_date = format_timestamp(stock_data.timestamps.get(mid_idx).unwrap(), &app.timeframe);
                x_labels.insert(1, Span::raw(mid_date));
            }
            TimeFrame::OneWeek => {
                let mid_idx = data_len / 2;
                let mid_date = format_timestamp(stock_data.timestamps.get(mid_idx).unwrap(), &app.timeframe);
                x_labels.insert(1, Span::raw(mid_date));
            },
            TimeFrame::OneMonth => {
                let first_quarter_idx = data_len / 4;
                let mid_idx = data_len / 2;
                let third_quarter_idx = data_len * 3 / 4;
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
                let first_month_idx = data_len / 3;
                let second_month_idx = data_len * 2 / 3;
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
                let first_quarter_idx = data_len / 4;
                let mid_idx = data_len / 2;
                let third_quarter_idx = data_len * 3 / 4;
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

        let title = if app.show_candlesticks {
            format!(
                "{} - {} (Candlesticks: {})",
                stock_data.symbol,
                app.timeframe.display(),
                app.candle_interval.to_string()
            )
        } else {
            format!(
                "{} - {}",
                stock_data.symbol,
                app.timeframe.display()
            )
        };

        let chart = Chart::new(datasets)
            .block(
                Block::default().borders(Borders::ALL).title(title),
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
            .wrap(ratatui::widgets::Wrap { trim: true })
            .alignment(Alignment::Center)
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

fn render_candlestick_chart(f: &mut Frame, candles: &[Candlestick], area: Rect, title: String, x_labels: Vec<Span>, _symbol: &str) {
    if candles.is_empty() {
        return;
    }

    // Calculate price range
    let max_price = candles.iter().map(|c| c.high).fold(f64::NEG_INFINITY, |a, b| a.max(b));
    let min_price = candles.iter().map(|c| c.low).fold(f64::INFINITY, |a, b| a.min(b));
    let price_range = max_price - min_price;

    if price_range == 0.0 {
        return;
    }

    // Create outer block with title and borders
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Reserve space for axes labels
    let chart_height = inner.height.saturating_sub(3) as usize;
    let chart_width = inner.width.saturating_sub(10) as usize;

    if chart_height == 0 || chart_width == 0 {
        return;
    }

    // Limit number of candles to display (show last N candles that fit)
    let max_candles = chart_width / 2; // 2 chars per candle minimum
    let display_start = if candles.len() > max_candles {
        candles.len() - max_candles
    } else {
        0
    };
    let displayed_candles = &candles[display_start..];
    let candle_width = if displayed_candles.len() > 0 {
        (chart_width / displayed_candles.len()).max(2)
    } else {
        2
    };

    // Build the chart line by line
    let mut lines: Vec<Line> = Vec::new();

    // Calculate which rows should show price labels (5 evenly spaced)
    let price_label_rows = [0, chart_height / 4, chart_height / 2, chart_height * 3 / 4, chart_height - 1];
    let price_labels_idx = [0, 1, 2, 3, 4];
    let price_label_values = [
        format!("${:.2}", max_price),
        format!("${:.2}", max_price - price_range * 0.25),
        format!("${:.2}", max_price - price_range * 0.5),
        format!("${:.2}", max_price - price_range * 0.75),
        format!("${:.2}", min_price),
    ];

    // Helper to convert price to row
    let price_to_row = |price: f64| -> usize {
        let normalized = (max_price - price) / price_range;
        let row = (normalized * chart_height as f64) as usize;
        row.min(chart_height - 1)
    };

    for row in 0..chart_height {
        let mut spans = Vec::new();

        // Add price label on the left (only at specific rows)
        let label_to_show = price_label_rows.iter()
            .position(|&r| r == row)
            .and_then(|idx| price_labels_idx.get(idx))
            .and_then(|&label_idx| price_label_values.get(label_idx));

        if let Some(label) = label_to_show {
            spans.push(Span::styled(
                format!("{:>8} ", label),
                Style::default().fg(Color::Gray)
            ));
        } else {
            spans.push(Span::raw("         "));
        }

        // Draw each candlestick
        for candle in displayed_candles.iter() {
            let is_bullish = candle.close >= candle.open;
            let color = if is_bullish { Color::Green } else { Color::Red };

            let body_top = candle.open.max(candle.close);
            let body_bottom = candle.open.min(candle.close);

            // Calculate row positions for this candle
            let high_row = price_to_row(candle.high);
            let low_row = price_to_row(candle.low);
            let body_top_row = price_to_row(body_top);
            let body_bottom_row = price_to_row(body_bottom);

            // Determine what to draw at this row
            let (char_to_draw, char_color) = if row >= high_row && row <= low_row {
                if row >= body_top_row && row <= body_bottom_row {
                    // In body area
                    ("█", color)
                } else {
                    // In wick area
                    ("│", color)
                }
            } else {
                // Outside candle range
                (" ", Color::White)
            };

            // Draw the candle
            spans.push(Span::styled(
                char_to_draw.repeat(candle_width.min(3)),
                Style::default().fg(char_color)
            ));
        }

        lines.push(Line::from(spans));
    }

    // Add time labels at the bottom
    let time_label_line = Line::from(vec![
        Span::raw("         "),
        Span::styled(
            format!("{:width$}", x_labels.first().map(|s| s.content.as_ref()).unwrap_or(""), width = chart_width / 3),
            Style::default().fg(Color::Gray)
        ),
        Span::styled(
            format!("{:^width$}", x_labels.get(x_labels.len() / 2).map(|s| s.content.as_ref()).unwrap_or(""), width = chart_width / 3),
            Style::default().fg(Color::Gray)
        ),
        Span::styled(
            format!("{:>width$}", x_labels.last().map(|s| s.content.as_ref()).unwrap_or(""), width = chart_width / 3),
            Style::default().fg(Color::Gray)
        ),
    ]);
    lines.push(Line::from(""));
    lines.push(time_label_line);

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}