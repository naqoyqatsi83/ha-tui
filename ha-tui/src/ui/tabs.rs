use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Tabs};
use ratatui::Frame;

use super::theme;
use super::TabHit;
use crate::app::AppState;

/// `Tabs`'s own render layout, in column cells: default single-space
/// padding on each side of a title, ` | ` between titles. Kept in sync
/// with the widget config below so hit-testing lands on the same cells
/// that were actually drawn.
const PADDING_WIDTH: u16 = 1;
const DIVIDER: &str = " | ";

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) -> Vec<TabHit> {
    let filtering = app.is_filtering();
    let label = app.tabs_label();
    let title = if filtering { format!(" {label} (search active) ") } else { format!(" {label} ") };

    let border_style = if filtering { Style::default().fg(theme::BORDER_DIM) } else { Style::default().fg(theme::BORDER) };
    // Borders alone (title text drawn on the border line, doesn't affect
    // the inner content area) - computed before the block moves into
    // `.block()` below, so hit-testing can start from the same origin
    // `Tabs` itself renders titles from.
    let inner = Block::default().borders(Borders::ALL).inner(area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD))
        .border_style(border_style);

    let titles = app.group_names();
    let tabs = Tabs::new(titles.clone())
        .block(block)
        .style(Style::default().fg(theme::TEXT_DIM))
        .highlight_style(Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD))
        .select(if filtering { None } else { Some(app.selected_group) })
        .divider(DIVIDER);

    frame.render_widget(tabs, area);

    // Mirrors `Tabs`'s own internal layout (padding_left, title,
    // padding_right, divider, repeat) exactly, since the widget doesn't
    // expose where each title actually landed.
    let mut hits = Vec::new();
    let right = inner.x + inner.width;
    let mut x = inner.x;
    for (index, name) in titles.iter().enumerate() {
        if x >= right {
            break;
        }
        x += PADDING_WIDTH;
        if x >= right {
            break;
        }
        let start = x;
        let width = (name.chars().count() as u16).min(right.saturating_sub(x));
        x += width;
        hits.push(TabHit { area: Rect { x: start, y: inner.y, width, height: 1 }, index });
        if x >= right {
            break;
        }
        x += PADDING_WIDTH + DIVIDER.chars().count() as u16;
    }
    hits
}
