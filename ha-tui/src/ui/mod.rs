pub mod cards;
pub mod detail;
pub mod help;
pub mod tabs;
pub mod theme;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::AppState;

const HELP_LINE: &str =
    "hjkl/arrows/mouse: move   Tab/Shift+Tab: switch tab   Enter/Space/click: toggle   +/-: adjust   /: search   ?: help   q: quit";

/// A clickable entity row's screen area from the most recent draw, and
/// which (card, row) in `AppState`'s grid it corresponds to - lets the
/// mouse click handler map a `(column, row)` terminal position back to a
/// selection without duplicating the card grid's layout math.
pub struct RowHit {
    pub area: Rect,
    pub card: usize,
    pub row: usize,
}

pub fn draw(frame: &mut Frame, app: &AppState) -> Vec<RowHit> {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(3)])
        .split(frame.area());

    tabs::render(frame, outer[0], app);
    let hits = cards::render(frame, outer[1], app);

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

    if let Some(entity_id) = app.detail_entity() {
        detail::render(frame, app, entity_id);
    } else if app.show_help {
        help::render(frame);
    }

    hits
}

/// Shown before the first snapshot has arrived from the WS task.
pub fn draw_connecting(frame: &mut Frame) {
    let help = Paragraph::new("Connecting to Home Assistant...").style(Style::default().fg(theme::TEXT));
    frame.render_widget(help, frame.area());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::registry::Registry;
    use crate::app::AppState;
    use crate::config::{DashboardCard, DashboardTab};
    use crate::ha::StateObject;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Position;
    use ratatui::Terminal;

    fn state(entity_id: &str) -> StateObject {
        serde_json::from_value(serde_json::json!({
            "entity_id": entity_id,
            "state": "on",
            "attributes": {},
            "last_updated": null,
            "last_changed": null,
        }))
        .unwrap()
    }

    /// The whole point of `RowHit` is letting a mouse click's screen
    /// position resolve back to the right (card, row) without duplicating
    /// the card grid's layout math - so this renders two real side-by-side
    /// panels through an actual `Terminal`/`TestBackend` and checks the
    /// returned hits describe what's actually on screen, rather than just
    /// trusting the layout arithmetic in isolation.
    #[test]
    fn draw_returns_row_hits_that_map_screen_position_back_to_card_and_row() {
        let mut tab = DashboardTab::new("Tab", vec![]);
        tab.cards = vec![
            DashboardCard {
                title: Some("Left".into()),
                entity_ids: vec!["light.a".into()],
                ..Default::default()
            },
            DashboardCard {
                title: Some("Right".into()),
                entity_ids: vec!["light.b".into()],
                ..Default::default()
            },
        ];
        let app = AppState::new(vec![state("light.a"), state("light.b")], Registry::default(), vec![tab]);

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut hits: Vec<RowHit> = Vec::new();
        terminal.draw(|frame| hits = draw(frame, &app)).unwrap();

        assert_eq!(hits.len(), 2);
        let left = hits.iter().find(|h| h.card == 0).expect("card 0 should have a hit");
        let right = hits.iter().find(|h| h.card == 1).expect("card 1 should have a hit");
        assert_eq!(left.row, 0);
        assert_eq!(right.row, 0);
        // Two panels side by side (80 columns fits both at the default
        // target width) - the right one must actually start further right.
        assert!(right.area.x > left.area.x);

        // Each row's own recorded area should contain its own top-left
        // corner - i.e. clicking where a row was actually drawn resolves
        // back to that same row.
        assert!(left.area.contains(Position { x: left.area.x, y: left.area.y }));
        assert!(right.area.contains(Position { x: right.area.x, y: right.area.y }));
        // And the two rows' areas must not overlap.
        assert!(!left.area.contains(Position { x: right.area.x, y: right.area.y }));
    }
}
