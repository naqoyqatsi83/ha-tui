use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::symbols;
use ratatui::widgets::{Axis, Block, Borders, Chart, Clear, Dataset, GraphType, Paragraph};
use ratatui::Frame;

use super::theme;
use crate::app::AppState;

/// Centered popup with a full X/Y-axis line chart of `entity_id`'s
/// history, opened via Enter on a graphed row. Any key closes it (see
/// main's key dispatch).
pub fn render(frame: &mut Frame, app: &AppState, entity_id: &str) {
    let area = centered_percent(frame.area(), 80, 70);
    frame.render_widget(Clear, area);

    let name = app.entities.get(entity_id).map(|e| e.friendly_name()).unwrap_or(entity_id);
    let current = app.entities.get(entity_id).map(|e| app.display_state(e)).unwrap_or_default();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {name} - {current}  (any key to close) "))
        .title_style(Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(theme::ACCENT));

    let points = app.history_points(entity_id);
    if points.len() < 2 {
        let message = Paragraph::new("Not enough history yet - keep the app open a little longer.")
            .style(Style::default().fg(theme::TEXT_DIM))
            .block(block);
        frame.render_widget(message, area);
        return;
    }

    let min = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let (y_min, y_max) = if max > min { (min, max) } else { (min - 1.0, min + 1.0) };
    let x_max = (points.len() - 1) as f64;

    let dataset = Dataset::default()
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(theme::ON))
        .data(&points);

    let chart = Chart::new(vec![dataset])
        .block(block)
        .x_axis(
            Axis::default()
                .title("time")
                .style(Style::default().fg(theme::BORDER))
                .bounds([0.0, x_max])
                .labels(vec!["oldest".to_string(), "now".to_string()]),
        )
        .y_axis(
            Axis::default()
                .title("value")
                .style(Style::default().fg(theme::BORDER))
                .bounds([y_min, y_max])
                .labels(vec![
                    format!("{y_min:.1}"),
                    format!("{:.1}", (y_min + y_max) / 2.0),
                    format!("{y_max:.1}"),
                ]),
        );

    frame.render_widget(chart, area);
}

fn centered_percent(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let width = area.width * width_pct / 100;
    let height = area.height * height_pct / 100;
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}
