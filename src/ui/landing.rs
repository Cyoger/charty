use ratatui::{
	layout::{Constraint, Direction, Layout, Alignment},
	widgets::{Block, Borders, Paragraph, List, ListItem},
	style::{Style, Color, Modifier},
	text::{Line, Span},
	Frame,
};

use super::{App, LandingPanel};


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
    let popular_focused = app.landing_panel == LandingPanel::Popular;
    let popular_border_style = if popular_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

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
                .title("Popular Stocks & Indices")
                .border_style(popular_border_style),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    f.render_stateful_widget(list, main_chunks[0], &mut app.popular_list_state.clone());

    // Watchlist panel
    let watchlist_focused = app.landing_panel == LandingPanel::Watchlist;
    let watchlist_border_style = if watchlist_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    if app.input_mode {
        // Show search input when in search mode
        let search_text = vec![
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
        ];
        let search = Paragraph::new(search_text)
            .block(Block::default().borders(Borders::ALL).title("Search"))
            .alignment(Alignment::Left);
        f.render_widget(search, main_chunks[1]);
    } else if app.watchlist.is_empty() {
        let hint = vec![
            Line::from(""),
            Line::from(Span::styled(
                "Your watchlist is empty.",
                Style::default().fg(Color::Gray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Open a chart and press 'w' to add",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "a symbol to your watchlist.",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        let panel = Paragraph::new(hint)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Watchlist")
                    .border_style(watchlist_border_style),
            )
            .alignment(Alignment::Left);
        f.render_widget(panel, main_chunks[1]);
    } else {
        let watchlist_items: Vec<ListItem> = app
            .watchlist
            .iter()
            .map(|symbol| {
                ListItem::new(Line::from(Span::styled(
                    symbol.clone(),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                )))
            })
            .collect();

        let watchlist = List::new(watchlist_items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Watchlist  (d: remove)")
                    .border_style(watchlist_border_style),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");

        f.render_stateful_widget(watchlist, main_chunks[1], &mut app.watchlist_state.clone());
    }

    // Footer
    let footer_text = if app.input_mode {
        "Enter: Confirm | Esc: Cancel"
    } else {
        "↑/↓: Navigate | Enter: Select | Tab: Switch panel | s: Search | d: Remove | q: Quit"
    };

    let footer = Paragraph::new(footer_text)
        .block(Block::default().borders(Borders::ALL).title("Controls"))
        .alignment(Alignment::Center);
    f.render_widget(footer, chunks[2]);
}
