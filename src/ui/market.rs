use ratatui::{
    layout::{Constraint, Direction, Layout, Alignment},
    widgets::{Block, Borders, Paragraph, List, ListItem},
    style::{Style, Color, Modifier},
    text::{Line, Span},
    Frame,
};

use super::{App, MarketPanel};
use crate::stock::MarketMover;

fn format_volume(vol: u64) -> String {
    if vol >= 1_000_000_000 {
        format!("{:.1}B", vol as f64 / 1_000_000_000.0)
    } else if vol >= 1_000_000 {
        format!("{:.1}M", vol as f64 / 1_000_000.0)
    } else if vol >= 1_000 {
        format!("{:.1}K", vol as f64 / 1_000.0)
    } else {
        format!("{}", vol)
    }
}

fn make_mover_items(movers: &[MarketMover], show_volume: bool) -> Vec<ListItem<'static>> {
    movers
        .iter()
        .map(|m| {
            let change_color = if m.change >= 0.0 { Color::Green } else { Color::Red };
            let sign = if m.change >= 0.0 { "+" } else { "" };

            let right_col = if show_volume {
                format!("{:>8}", format_volume(m.volume))
            } else {
                format!("{}{:.2}%", sign, m.change_percent)
            };

            let name_truncated = if m.name.len() > 20 {
                format!("{:.20}", m.name)
            } else {
                m.name.clone()
            };

            ratatui::text::Text::from(vec![
                Line::from(vec![
                    Span::styled(
                        format!("{:<8}", m.symbol),
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{:>8.2}", m.price),
                        Style::default().fg(Color::White),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("{:>10}", right_col),
                        Style::default().fg(change_color).add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(Span::styled(
                    format!("  {}", name_truncated),
                    Style::default().fg(Color::DarkGray),
                )),
            ])
        })
        .map(ListItem::new)
        .collect()
}

fn make_header(label: &str) -> ListItem<'static> {
    ListItem::new(Line::from(Span::styled(
        label.to_string(),
        Style::default().fg(Color::DarkGray),
    )))
}

pub fn render_market_view(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(f.area());

    // Header
    let header = Paragraph::new(Line::from(Span::styled(
        "Market Overview",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    )))
    .block(Block::default().borders(Borders::ALL))
    .alignment(Alignment::Center);
    f.render_widget(header, chunks[0]);

    if app.market_loading {
        let loading = Paragraph::new("Loading market data...")
            .block(Block::default().borders(Borders::ALL))
            .alignment(Alignment::Center);
        f.render_widget(loading, chunks[1]);
    } else if let Some(ref err) = app.market_error {
        let error = Paragraph::new(err.clone())
            .block(Block::default().borders(Borders::ALL).title("Error"))
            .style(Style::default().fg(Color::Red))
            .alignment(Alignment::Center);
        f.render_widget(error, chunks[1]);
    } else {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(33),
                Constraint::Percentage(34),
                Constraint::Percentage(33),
            ])
            .split(chunks[1]);

        // Column header row: SYMBOL | PRICE | CHANGE%
        let col_header = format!("{:<8}  {:>8}  {:>10}", "SYMBOL", "PRICE", "CHANGE%");
        let vol_header = format!("{:<8}  {:>8}  {:>10}", "SYMBOL", "PRICE", "VOLUME");

        // Gainers
        let gainers_focused = app.market_panel == MarketPanel::Gainers;
        let gainers_border = if gainers_focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let mut gainers_items = vec![make_header(&col_header)];
        gainers_items.extend(make_mover_items(&app.market_gainers, false));
        let gainers_list = List::new(gainers_items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Top Gainers ")
                    .border_style(gainers_border),
            )
            .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
            .highlight_symbol(">> ");
        // Offset selection by 1 to account for header row
        let mut gainers_state = app.market_gainers_state.clone();
        if let Some(i) = gainers_state.selected() {
            gainers_state.select(Some(i + 1));
        }
        f.render_stateful_widget(gainers_list, cols[0], &mut gainers_state);

        // Losers
        let losers_focused = app.market_panel == MarketPanel::Losers;
        let losers_border = if losers_focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let mut losers_items = vec![make_header(&col_header)];
        losers_items.extend(make_mover_items(&app.market_losers, false));
        let losers_list = List::new(losers_items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Top Losers ")
                    .border_style(losers_border),
            )
            .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
            .highlight_symbol(">> ");
        let mut losers_state = app.market_losers_state.clone();
        if let Some(i) = losers_state.selected() {
            losers_state.select(Some(i + 1));
        }
        f.render_stateful_widget(losers_list, cols[1], &mut losers_state);

        // Most Active
        let active_focused = app.market_panel == MarketPanel::Active;
        let active_border = if active_focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let mut active_items = vec![make_header(&vol_header)];
        active_items.extend(make_mover_items(&app.market_active, true));
        let active_list = List::new(active_items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Most Active ")
                    .border_style(active_border),
            )
            .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
            .highlight_symbol(">> ");
        let mut active_state = app.market_active_state.clone();
        if let Some(i) = active_state.selected() {
            active_state.select(Some(i + 1));
        }
        f.render_stateful_widget(active_list, cols[2], &mut active_state);
    }

    // Footer
    let footer = Paragraph::new(
        "↑/↓: Navigate | Tab: Switch panel | Enter: View chart | r: Refresh | b: Back | q: Quit",
    )
    .block(Block::default().borders(Borders::ALL).title("Controls"))
    .alignment(Alignment::Center);
    f.render_widget(footer, chunks[2]);
}
