use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Frame;

use crate::app::AppState;

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let items: Vec<ListItem> = app
        .group_names()
        .into_iter()
        .map(ListItem::new)
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Rooms"))
        .highlight_style(Style::default().bg(Color::Blue).add_modifier(Modifier::BOLD));

    let mut state = ListState::default();
    state.select(Some(app.selected_group));

    frame.render_stateful_widget(list, area, &mut state);
}
