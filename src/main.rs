use crossterm::{
    event::{self, Event, KeyCode}, 
    execute, 
    terminal::{ EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode}
};

use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout}, 
    style::{Color, Style , Modifier}, 
    symbols, 
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph},
    text::{Line, Span},
};

use std::io;

mod stock;


fn main() -> Result<(), Box<dyn std::error::Error>>{

    println!("Fetching stock data for AAPL...");
    let stock_data = stock::fetch_stock_data("AAPL")?;

    
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, stock_data );

    disable_raw_mode()?;

    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("Error: {:?}", err);
    }

    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    stock_data: stock::StockData,
) -> Result<(), io::Error> {

    let chart_data: Vec<(f64, f64)> = stock_data
        .prices
        .iter()
        .enumerate()
        .map(|(i, &price)| (i as f64, price))
        .collect();

    // Calculate bounds for the chart
    let max_price = stock_data.prices.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
    let min_price = stock_data.prices.iter().fold(f64::INFINITY, |a, &b| a.min(b));
    let max_x = (stock_data.prices.len() - 1) as f64;
    
    let first_date = stock_data.timestamps.first().unwrap().format("%m/%d").to_string();
    let last_date = stock_data.timestamps.last().unwrap().format("%m/%d").to_string();

    let price_color = if stock_data.change >= 0.0 {
        Color::Green
    } else {
        Color::Red
    };

    let change_symbol = if stock_data.change >= 0.0 { "▲" } else { "▼" };

    
    loop {
        terminal.draw(|f| {
            let size = f.area();
            
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(0),
                    Constraint::Length(3),
                ])
                .split(size);
            
            // Header with stock info
            let header_text = vec![
                Line::from(vec![
                    Span::raw(format!("{} ", stock_data.symbol)),
                    Span::styled(
                        format!("${:.2}", stock_data.current_price),
                        Style::default().fg(price_color).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("{} ${:.2} ({:.2}%)", 
                            change_symbol, 
                            stock_data.change.abs(), 
                            stock_data.change_percent.abs()
                        ),
                        Style::default().fg(price_color),
                    ),
                ]),
            ];
            
            let header = Paragraph::new(header_text)
                .block(Block::default().borders(Borders::ALL).title("Stock Info"));
            f.render_widget(header, chunks[0]);
            
            // Create dataset
            let dataset = Dataset::default()
                .name(stock_data.symbol.as_str())
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(price_color))
                .data(&chart_data);
            
            // Create axis labels
            let x_labels = vec![
                Span::raw(first_date.clone()),
                Span::raw(last_date.clone()),
            ];
            
            let y_labels = vec![
                Span::raw(format!("${:.0}", min_price)),
                Span::raw(format!("${:.0}", (min_price + max_price) / 2.0)),
                Span::raw(format!("${:.0}", max_price)),
            ];
            
            // Create chart with proper axes
            let chart = Chart::new(vec![dataset])
                .block(Block::default()
                    .borders(Borders::ALL)
                    .title(format!("{} - Last 30 Days", stock_data.symbol)))
                .x_axis(
                    Axis::default()
                        .title("Date")
                        .style(Style::default().fg(Color::Gray))
                        .bounds([0.0, max_x])
                        .labels(x_labels)
                )
                .y_axis(
                    Axis::default()
                        .title("Price")
                        .style(Style::default().fg(Color::Gray))
                        .bounds([min_price - 5.0, max_price + 5.0])
                        .labels(y_labels)
                );
            
            f.render_widget(chart, chunks[1]);
            
            // Footer
            let footer = Paragraph::new("Press 'q' to quit | 'r' to refresh")
                .block(Block::default().borders(Borders::ALL).title("Controls"));
            f.render_widget(footer, chunks[2]);
        })?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    return Ok(());
                }
            }
        }
    }
}