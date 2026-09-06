use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Sparkline};
use ratatui::Frame;

use super::theme;
use crate::app::entity::Entity;
use crate::app::{AppState, ResolvedCard};

const TARGET_CARD_WIDTH: u16 = 34;
const MAX_COLUMNS: usize = 4;
const MIN_CARD_HEIGHT: u16 = 3;
const MAX_CARD_HEIGHT: u16 = 12;
/// Row height (in terminal lines) for an entity: a graphed one gets an
/// extra line underneath its name/value for the sparkline.
fn entity_units(app: &AppState, entity: &Entity) -> u16 {
    if app.is_graphed(&entity.entity_id) {
        2
    } else {
        1
    }
}

fn card_units(app: &AppState, card: &ResolvedCard) -> usize {
    card.entities.iter().map(|e| entity_units(app, e) as usize).sum()
}

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let cards = app.visible_cards();

    if cards.is_empty() {
        let message = Paragraph::new("No entities here.")
            .style(Style::default().fg(theme::TEXT_DIM))
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(theme::BORDER_DIM)));
        frame.render_widget(message, area);
        return;
    }

    let columns = ((area.width / TARGET_CARD_WIDTH).max(1) as usize).min(MAX_COLUMNS).min(cards.len());

    // Map the flat selection index onto (card_index, row_within_card) so
    // the right row in the right panel gets the highlight.
    let selected_flat = app.visible_selected_index();
    let mut running = 0usize;
    let mut selected = None;
    for (i, card) in cards.iter().enumerate() {
        if selected_flat < running + card.entities.len() {
            selected = Some((i, selected_flat - running));
            break;
        }
        running += card.entities.len();
    }

    let rows: Vec<&[ResolvedCard]> = cards.chunks(columns).collect();
    let row_heights: Vec<u16> = rows
        .iter()
        .map(|row| {
            let max_units = row.iter().map(|c| card_units(app, c)).max().unwrap_or(0);
            (max_units as u16 + 2).clamp(MIN_CARD_HEIGHT, MAX_CARD_HEIGHT)
        })
        .collect();

    let row_areas = Layout::vertical(row_heights.iter().map(|h| Constraint::Length(*h))).split(area);

    for (row_idx, row_cards) in rows.iter().enumerate() {
        let col_areas =
            Layout::horizontal(row_cards.iter().map(|_| Constraint::Ratio(1, row_cards.len() as u32))).split(row_areas[row_idx]);

        for (col_idx, card) in row_cards.iter().enumerate() {
            let global_idx = row_idx * columns + col_idx;
            let selected_row = selected.filter(|(i, _)| *i == global_idx).map(|(_, row)| row);
            render_card(frame, col_areas[col_idx], card, app, selected_row);
        }
    }
}

fn render_card(frame: &mut Frame, area: Rect, card: &ResolvedCard, app: &AppState, selected_row: Option<usize>) {
    let title = card.title.clone().unwrap_or_default();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .title_style(Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(if selected_row.is_some() { theme::ACCENT } else { theme::BORDER }));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Give the name column roughly half the card's inner width (minimum
    // enough for short names), state text gets the rest.
    let inner_width = inner.width as usize;
    let name_width = (inner_width * 55 / 100).clamp(10, inner_width.saturating_sub(4).max(10));

    // Fit as many entities as the card's height (in row-units - a graphed
    // entity takes 2) allows, reserving one line for "+N more" if not all
    // fit.
    let capacity = inner.height as usize;
    let mut visible: Vec<(usize, &Entity, u16)> = Vec::new();
    let mut used = 0usize;
    for (i, entity) in card.entities.iter().enumerate() {
        let units = entity_units(app, entity) as usize;
        if used + units > capacity {
            break;
        }
        visible.push((i, entity, units as u16));
        used += units;
    }
    let truncated = visible.len() < card.entities.len();
    if truncated {
        while used + 1 > capacity {
            let Some((_, _, units)) = visible.pop() else { break };
            used -= units as usize;
        }
    }

    let mut y = inner.y;
    for (i, entity, units) in &visible {
        let row_area = Rect { x: inner.x, y, width: inner.width, height: *units };
        let is_selected = selected_row == Some(*i);
        render_entity_row(frame, row_area, entity, app, is_selected, name_width);
        y += units;
    }
    if truncated {
        let hidden = card.entities.len() - visible.len();
        let rect = Rect { x: inner.x, y, width: inner.width, height: 1 };
        frame.render_widget(Paragraph::new(format!("+{hidden} more")).style(Style::default().fg(theme::TEXT_DIM)), rect);
    }
}

fn render_entity_row(frame: &mut Frame, area: Rect, entity: &Entity, app: &AppState, selected: bool, name_width: usize) {
    let style = if selected {
        Style::default().bg(theme::HIGHLIGHT_BG).fg(theme::HIGHLIGHT_FG).add_modifier(Modifier::BOLD)
    } else {
        row_style(entity, app)
    };

    let text = format!("{:<name_width$} {}", truncate(entity.friendly_name(), name_width), app.display_state(entity));
    let text_area = Rect { height: 1, ..area };
    frame.render_widget(Paragraph::new(text).style(style), text_area);

    if area.height > 1 {
        let data = app.sparkline_data(&entity.entity_id);
        let spark_area = Rect { y: area.y + 1, height: area.height - 1, ..area };
        let spark_style = Style::default().fg(if selected { theme::HIGHLIGHT_BG } else { theme::ACCENT });
        let sparkline = Sparkline::default().data(&data).style(spark_style);
        frame.render_widget(sparkline, spark_area);
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}

/// Color by state: on/active = green, unavailable/unknown = dimmed,
/// everything else default. A row whose state changed within the last
/// `FLASH_DURATION` gets a brief reversed-video flash on top.
fn row_style(entity: &Entity, app: &AppState) -> Style {
    let base = if entity.is_unavailable() {
        Style::default().fg(theme::UNAVAILABLE)
    } else {
        let is_on = entity
            .as_light()
            .map(|l| l.is_on())
            .or_else(|| entity.as_switch().map(|s| s.is_on()))
            .unwrap_or(false)
            || entity.state == "on";
        if is_on {
            Style::default().fg(theme::ON)
        } else {
            Style::default().fg(theme::TEXT)
        }
    };

    if app.is_recently_changed(&entity.entity_id) {
        base.add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        base
    }
}
