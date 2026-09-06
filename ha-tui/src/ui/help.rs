use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

const LINES: &[&str] = &[
    "j / down       move down",
    "k / up         move up",
    "Tab            next room",
    "Shift+Tab      previous room",
    "Enter / Space  toggle light or switch",
    "+ / =          increase brightness or target temperature",
    "-              decrease brightness or target temperature",
    "/              search entities by name",
    "  (while searching) Enter keeps the filter, Esc clears it",
    "?              toggle this help",
    "q              quit",
    "",
    "press any key to close",
];

pub fn render(frame: &mut Frame) {
    let width = LINES.iter().map(|l| l.len()).max().unwrap_or(20) as u16 + 4;
    let height = LINES.len() as u16 + 2;

    let area = centered(width, height, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Keybindings")
        .border_style(Style::default().fg(Color::Cyan));

    let text = LINES.join("\n");
    let paragraph = Paragraph::new(text).block(block);
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
