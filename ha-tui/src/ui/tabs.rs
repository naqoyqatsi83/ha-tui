use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Tabs};
use ratatui::Frame;

use super::theme;
use crate::app::AppState;

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let filtering = app.is_filtering();
    let label = app.tabs_label();
    let title = if filtering { format!(" {label} (search active) ") } else { format!(" {label} ") };

    let border_style = if filtering { Style::default().fg(theme::BORDER_DIM) } else { Style::default().fg(theme::BORDER) };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD))
        .border_style(border_style);

    let titles = app.group_names();
    let tabs = Tabs::new(titles)
        .block(block)
        .style(Style::default().fg(theme::TEXT_DIM))
        .highlight_style(Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD))
        .select(if filtering { None } else { Some(app.selected_group) })
        .divider(" | ");

    frame.render_widget(tabs, area);
}
