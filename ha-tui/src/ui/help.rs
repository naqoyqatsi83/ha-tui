use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::theme;

const LINES: &[&str] = &[
    "j / down       move down (panel above/below at the edge)",
    "k / up         move up (panel above/below at the edge)",
    "h / left       switch to the panel on the left",
    "l / right      switch to the panel on the right",
    "Tab            next tab",
    "Shift+Tab      previous tab",
    "Enter / Space  toggle light or switch (opens the history chart on a graphed row)",
    "+ / =          increase brightness or target temperature",
    "-              decrease brightness or target temperature",
    "/              search entities by name",
    "  (while searching) Enter keeps the filter, Esc clears it",
    "?              toggle this help",
    "q              quit",
    "",
    "mouse: click a row to select it, double-click to toggle/open its chart",
    "mouse: click a tab to switch to it, scroll to move within a panel",
    "F2             toggle mouse mode off/on (off = normal terminal text selection/copy)",
    "",
    "press any key or click to close",
];

pub fn render(frame: &mut Frame) {
    let width = LINES.iter().map(|l| l.len()).max().unwrap_or(20) as u16 + 4;
    let height = LINES.len() as u16 + 2;

    let area = centered(width, height, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(theme::PANEL_BORDER)
        .title(theme::badge(" Keybindings ", theme::ACCENT))
        .border_style(Style::default().fg(theme::ACCENT));

    let text = LINES.join("\n");
    let paragraph = Paragraph::new(text).style(Style::default().fg(theme::TEXT)).block(block);
    frame.render_widget(paragraph, area);
}

fn centered(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}
