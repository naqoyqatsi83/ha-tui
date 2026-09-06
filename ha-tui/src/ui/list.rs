use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Frame;

use crate::app::entity::Entity;
use crate::app::AppState;

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let entities = app.visible_entities();

    let items: Vec<ListItem> = entities
        .iter()
        .map(|entity| {
            let state_display = app.display_state(entity);
            let text = format!("{:<40} {}", entity.friendly_name(), state_display);
            ListItem::new(text).style(row_style(entity))
        })
        .collect();

    let title = if let Some(query) = app.filter_query() {
        format!("Search: {query}")
    } else {
        app.selected_group_name().unwrap_or_else(|| "Entities".to_string())
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().bg(Color::Blue).add_modifier(Modifier::BOLD));

    let mut state = ListState::default();
    if !entities.is_empty() {
        state.select(Some(app.visible_selected_index()));
    }

    frame.render_stateful_widget(list, area, &mut state);
}

/// Color by state: on/active = green, unavailable/unknown = dimmed red,
/// everything else default.
fn row_style(entity: &Entity) -> Style {
    if entity.is_unavailable() {
        return Style::default().fg(Color::DarkGray);
    }
    let is_on = entity
        .as_light()
        .map(|l| l.is_on())
        .or_else(|| entity.as_switch().map(|s| s.is_on()))
        .unwrap_or(false)
        || entity.state == "on";
    if is_on {
        Style::default().fg(Color::Green)
    } else {
        Style::default()
    }
}
