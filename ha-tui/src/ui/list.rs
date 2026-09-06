use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Frame;

use crate::app::AppState;

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let entities = app.selected_group_entities();

    let items: Vec<ListItem> = entities
        .iter()
        .map(|entity| {
            let state_display = match entity.as_sensor() {
                Some(sensor) => match sensor.unit() {
                    Some(unit) => format!("{} {unit}", entity.state),
                    None => entity.state.clone(),
                },
                None => entity.state.clone(),
            };
            ListItem::new(format!("{:<40} {}", entity.friendly_name(), state_display))
        })
        .collect();

    let title = app.selected_group_name().unwrap_or_else(|| "Entities".to_string());
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().bg(Color::Blue).add_modifier(Modifier::BOLD));

    let mut state = ListState::default();
    if !entities.is_empty() {
        state.select(Some(app.selected_entity));
    }

    frame.render_stateful_widget(list, area, &mut state);
}
