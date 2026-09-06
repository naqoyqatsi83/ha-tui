use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Frame;

use crate::app::AppState;

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let filtering = app.is_filtering();

    let items: Vec<ListItem> = app.group_names().into_iter().map(ListItem::new).collect();

    let title = if filtering { "Rooms (search active)" } else { "Rooms" };
    let border_style = if filtering {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title).border_style(border_style))
        .highlight_style(if filtering {
            Style::default()
        } else {
            Style::default().bg(Color::Blue).add_modifier(Modifier::BOLD)
        });

    let mut state = ListState::default();
    if !filtering {
        state.select(Some(app.selected_group));
    }

    frame.render_stateful_widget(list, area, &mut state);
}
