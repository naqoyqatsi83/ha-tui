use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Sparkline};
use ratatui::Frame;

use super::theme;
use crate::app::entity::Entity;
use crate::app::{AppState, ResolvedCard};

const TARGET_CARD_WIDTH: u16 = 28;
const MAX_COLUMNS: usize = 5;
const MIN_CARD_HEIGHT: u16 = 3;
const MAX_CARD_HEIGHT: u16 = 12;

/// How many panels the grid currently lays out per row for `width` and
/// `card_count` panels - shared with `AppState`'s left/right navigation so
/// they always agree with what's actually on screen.
pub fn columns_for(width: u16, card_count: usize) -> usize {
    if card_count == 0 {
        return 1;
    }
    ((width / TARGET_CARD_WIDTH).max(1) as usize).min(MAX_COLUMNS).min(card_count)
}
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

    let columns = columns_for(area.width, cards.len());
    let (selected_card, selected_row_idx) = app.selected_position();

    let rows: Vec<&[ResolvedCard]> = cards.chunks(columns).collect();
    let row_heights: Vec<u16> = rows
        .iter()
        .map(|row| {
            let max_units = row.iter().map(|c| card_units(app, c)).max().unwrap_or(0);
            (max_units as u16 + 2).clamp(MIN_CARD_HEIGHT, MAX_CARD_HEIGHT)
        })
        .collect();

    let row_areas = Layout::vertical(row_heights.iter().map(|h| Constraint::Length(*h))).split(area);

    // Panels briefly expand in (top-anchored reveal) right after a tab
    // switch; ease-out so it settles rather than stopping abruptly.
    let linear = app.tab_transition_progress();
    let eased = 1.0 - (1.0 - linear).powi(3);

    for (row_idx, row_cards) in rows.iter().enumerate() {
        let col_areas =
            Layout::horizontal(row_cards.iter().map(|_| Constraint::Ratio(1, row_cards.len() as u32))).split(row_areas[row_idx]);

        for (col_idx, card) in row_cards.iter().enumerate() {
            let global_idx = row_idx * columns + col_idx;
            let selected_row = (global_idx == selected_card).then_some(selected_row_idx);
            let full = col_areas[col_idx];
            let height = ((full.height as f32) * eased).round() as u16;
            if height == 0 {
                continue;
            }
            let area = Rect { height, ..full };
            render_card(frame, area, card, app, selected_row);
        }
    }
}

fn render_card(frame: &mut Frame, area: Rect, card: &ResolvedCard, app: &AppState, selected_row: Option<usize>) {
    let is_selected_panel = selected_row.is_some();
    let title = card.title.clone().unwrap_or_default();
    let mut block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .title_style(Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(if is_selected_panel { theme::ACCENT } else { theme::BORDER }));
    if is_selected_panel {
        // Tints the whole panel (border + interior) so the focused panel
        // reads clearly even before spotting which row inside it is lit -
        // rows rendered on top only set `fg`, so this background shows
        // through everywhere they don't otherwise highlight.
        block = block.style(Style::default().bg(theme::PANEL_SELECTED_BG));
    }
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
