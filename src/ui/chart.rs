use ratatui::{
	layout::{Constraint, Direction, Layout},
	widgets::{Block, Borders, Paragraph, Chart, Dataset, Axis, GraphType},
	symbols,
	style::{Style, Color, Modifier},
	text::{Line, Span},
	Frame,
};
use chrono::{DateTime, Utc, Local};

use super::App;
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