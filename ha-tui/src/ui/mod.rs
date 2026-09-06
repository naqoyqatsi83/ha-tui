pub mod cards;
pub mod help;
pub mod tabs;
pub mod theme;

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::AppState;

const HELP_LINE: &str =
    "j/k: move   Tab/Shift+Tab: switch tab   Enter/Space: toggle   +/-: adjust   /: search   ?: help   q: quit";

pub fn draw(frame: &mut Frame, app: &AppState) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(3)])
        .split(frame.area());

    tabs::render(frame, outer[0], app);
    cards::render(frame, outer[1], app);

    let status_block = Block::default()
        .borders(Borders::ALL)
        .title(" Status ")
        .title_style(Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(theme::BORDER));
    let inner = status_block.inner(outer[2]);
    frame.render_widget(status_block, outer[2]);

    let status_line = if app.is_filter_editing() {
        let query = app.filter_query().unwrap_or_default();
        Paragraph::new(Line::from(format!("/{query}"))).style(Style::default().fg(theme::ACCENT))
    } else if let Some(message) = app.status_message() {
        Paragraph::new(Line::from(message)).style(Style::default().fg(theme::ACCENT))
    } else {
        Paragraph::new(Line::from(HELP_LINE)).style(Style::default().fg(theme::TEXT_DIM))
    };
    frame.render_widget(status_line, inner);

    if app.show_help {
        help::render(frame);
    }
}

/// Shown before the first snapshot has arrived from the WS task.
pub fn draw_connecting(frame: &mut Frame) {
    let help = Paragraph::new("Connecting to Home Assistant...").style(Style::default().fg(theme::TEXT));
    frame.render_widget(help, frame.area());
}
