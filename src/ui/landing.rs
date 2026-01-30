use ratatui::{
	layout::{Constraint, Direction, Layout, Alignment},
	widgets::{Block, Borders, Paragraph, List, ListItem},
	style::{Style, Color, Modifier},
	text::{Line, Span},
	Frame,
};

use super::App;


pub fn render_landing(f: &mut Frame, app: &App) {
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