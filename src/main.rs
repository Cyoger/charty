use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    widgets::{Block, Borders, Paragraph, Chart, Dataset, Axis, GraphType},
    symbols,
    style::{Style, Color, Modifier},
    text::{Line, Span},
    Frame,
    Terminal,
};
use std::io;

mod stock;
use stock::TimeFrame;

struct App {
    symbol: String,
    timeframe: TimeFrame,
    stock_data: Option<stock::StockData>,
    input_mode: bool,
    input_buffer: String,
    error_message: Option<String>,
    loading: bool,
}

impl App {
    fn new() -> Self {
        Self {
            symbol: "AAPL".to_string(),
            timeframe: TimeFrame::OneMonth,
            stock_data: None,
            input_mode: false,
            input_buffer: String::new(),
            error_message: None,
            loading: false,
        }
    }

    fn fetch_data(&mut self) {
        self.loading = true;
        match stock::fetch_stock_data(&self.symbol, self.timeframe) {
            Ok(data) => {
                self.stock_data = Some(data);
                self.error_message = None;
            }
            Err(e) => {
                self.error_message = Some(format!("Error: {}", e));
            }
        }
        self.loading = false;
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new();
    
    app.fetch_data();
    
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("Error: {:?}", err);
    }

    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<(), io::Error> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if handle_input(app, key.code) {
                    return Ok(()); 
                }
            }
        }
    }
}

fn ui(f: &mut Frame, app: &App) {
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
                Span::raw(format!("  [{}]", app.timeframe.display())),
            ]),
        ];
        
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
        
        let max_price = stock_data.prices.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
        let min_price = stock_data.prices.iter().fold(f64::INFINITY, |a, &b| a.min(b));
        let max_x = (stock_data.prices.len() - 1) as f64;
        
        let first_date = stock_data.timestamps.first().unwrap().format("%m/%d").to_string();
        let last_date = stock_data.timestamps.last().unwrap().format("%m/%d").to_string();
        
        let dataset = Dataset::default()
            .name(stock_data.symbol.as_str())
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(price_color))
            .data(&chart_data);
        
        let x_labels = vec![
            Span::raw(first_date),
            Span::raw(last_date),
        ];
        
        let y_labels = vec![
            Span::raw(format!("${:.0}", min_price)),
            Span::raw(format!("${:.0}", (min_price + max_price) / 2.0)),
            Span::raw(format!("${:.0}", max_price)),
        ];
        
        let chart = Chart::new(vec![dataset])
            .block(Block::default()
                .borders(Borders::ALL)
                .title(format!("{} - {}", stock_data.symbol, app.timeframe.display())))
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
        
        f.render_widget(chart, area);
    } else if let Some(ref error) = app.error_message {
        let error_text = Paragraph::new(error.as_str())
            .style(Style::default().fg(Color::Red))
            .block(Block::default().borders(Borders::ALL).title("Error"));
        f.render_widget(error_text, area);
    }
}

fn render_footer(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let footer_text = if app.input_mode {
        vec![
            Line::from("Enter stock symbol (press Enter to confirm, Esc to cancel):"),
            Line::from(Span::styled(
                format!("> {}", app.input_buffer),
                Style::default().fg(Color::Yellow),
            )),
        ]
    } else {
        vec![
            Line::from("Controls:"),
            Line::from("'s': Change symbol | '←/→': Change timeframe | 'r': Refresh | 'q': Quit"),
        ]
    };
    
    let footer = Paragraph::new(footer_text)
        .block(Block::default().borders(Borders::ALL).title("Controls"));
    f.render_widget(footer, area);
}

fn handle_input(app: &mut App, key: KeyCode) -> bool {
    if app.input_mode {
        match key {
            KeyCode::Enter => {
                if !app.input_buffer.is_empty() {
                    app.symbol = app.input_buffer.to_uppercase();
                    app.input_buffer.clear();
                    app.input_mode = false;
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
        false
    } else {
        match key {
            KeyCode::Char('q') => true,
            KeyCode::Char('s') => {
                app.input_mode = true;
                false
            }
            KeyCode::Char('r') => {
                app.fetch_data();
                false
            }
            KeyCode::Left => {
                app.timeframe = app.timeframe.prev();
                app.fetch_data();
                false
            }
            KeyCode::Right => {
                app.timeframe = app.timeframe.next();
                app.fetch_data();
                false
            }
            _ => false,
        }
    }
}