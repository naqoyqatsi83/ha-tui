pub mod list;
pub mod tabs;

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::AppState;

const HELP_LINE: &str =
    "j/k: move   Tab/Shift+Tab: switch room   Enter/Space: toggle   +/-: adjust   q: quit";

pub fn draw(frame: &mut Frame, app: &AppState) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(frame.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(24), Constraint::Min(0)])
        .split(outer[0]);

    tabs::render(frame, body[0], app);
    list::render(frame, body[1], app);

    let bottom = match app.status_message() {
        Some(message) => Paragraph::new(Line::from(message)).style(Style::default().fg(Color::Yellow)),
        None => Paragraph::new(Line::from(HELP_LINE)).style(Style::default().fg(Color::DarkGray)),
    };
    frame.render_widget(bottom, outer[1]);
}

/// Shown before the first snapshot has arrived from the WS task.
pub fn draw_connecting(frame: &mut Frame) {
    let help = Paragraph::new("Connecting to Home Assistant...");
    frame.render_widget(help, frame.area());
}
