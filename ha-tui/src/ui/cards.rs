use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Sparkline};
use ratatui::Frame;

use super::theme;
use super::RowHit;
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

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) -> Vec<RowHit> {
    let cards = app.visible_cards();
    let mut hits = Vec::new();

    if cards.is_empty() {
        let message = Paragraph::new("No entities here.")
            .style(Style::default().fg(theme::TEXT_DIM))
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(theme::BORDER_DIM)));
        frame.render_widget(message, area);
        return hits;
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
            render_card(frame, area, card, app, global_idx, selected_row, &mut hits);
        }
    }

    hits
}

fn render_card(
    frame: &mut Frame,
    area: Rect,
    card: &ResolvedCard,
    app: &AppState,
    card_idx: usize,
    selected_row: Option<usize>,
    hits: &mut Vec<RowHit>,
) {
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
    // entity takes 2) allows, reserving a line for "N above"/"N more"
    // indicators if not everything fits. Scrolls just far enough that the
    // selected row (if any) is actually visible - otherwise selecting
    // into the overflow via j/k would never show what got selected.
    let capacity = inner.height as usize;
    let units: Vec<usize> = card.entities.iter().map(|e| entity_units(app, e) as usize).collect();

    // Smallest start such that entities[start..=selected] fit in the full
    // capacity (indicator reservations are handled separately below).
    let start = match selected_row {
        Some(sel) if sel < units.len() => {
            let mut start = 0;
            while start < sel && units[start..=sel].iter().sum::<usize>() > capacity {
                start += 1;
            }
            start
        }
        _ => 0,
    };

    let mut y = inner.y;
    if start > 0 {
        let rect = Rect { x: inner.x, y, width: inner.width, height: 1 };
        frame.render_widget(Paragraph::new(format!("↑ {start} above")).style(Style::default().fg(theme::TEXT_DIM)), rect);
        y += 1;
    }
    let budget = capacity.saturating_sub(if start > 0 { 1 } else { 0 });

    let mut used = 0usize;
    let mut end = start;
    let last_index = card.entities.len() - 1;
    for (i, (entity, &h)) in card.entities.iter().zip(units.iter()).enumerate().skip(start) {
        // Reserve a line for a "below" indicator unless this is the last
        // entity - except for the selected row itself, which must be
        // shown regardless (it was already guaranteed to fit within the
        // full capacity by the `start` search above).
        let reserve = if i == last_index || Some(i) == selected_row { 0 } else { 1 };
        if used + h + reserve > budget {
            break;
        }
        let row_area = Rect { x: inner.x, y, width: inner.width, height: h as u16 };
        render_entity_row(frame, row_area, entity, app, Some(i) == selected_row, name_width);
        hits.push(RowHit { area: row_area, card: card_idx, row: i });
        y += h as u16;
        used += h;
        end = i + 1;
    }

    let below_hidden = card.entities.len() - end;
    if below_hidden > 0 {
        let rect = Rect { x: inner.x, y, width: inner.width, height: 1 };
        frame.render_widget(
            Paragraph::new(format!("+{below_hidden} more (↓ to see)")).style(Style::default().fg(theme::TEXT_DIM)),
            rect,
        );
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
        // `Sparkline` draws one bar per data point rather than stretching
        // to fill the given width, so a history buffer that's shorter
        // than the card (still filling up, or just fewer real samples
        // than columns) would only use part of it - resample to exactly
        // the available width so it always fills the panel edge-to-edge.
        let data = resample(&app.sparkline_data(&entity.entity_id), area.width as usize);
        let spark_area = Rect { y: area.y + 1, height: area.height - 1, ..area };
        let spark_style = Style::default().fg(if selected { theme::HIGHLIGHT_BG } else { theme::ACCENT });
        let sparkline = Sparkline::default().data(&data).style(spark_style);
        frame.render_widget(sparkline, spark_area);
    }
}

/// Nearest-neighbor stretches or compresses `data` to exactly `width`
/// points, so the sparkline always spans the full panel width regardless
/// of how many real history samples are behind it.
fn resample(data: &[u64], width: usize) -> Vec<u64> {
    if width == 0 {
        return Vec::new();
    }
    if data.is_empty() {
        return vec![0; width];
    }
    if data.len() == width {
        return data.to_vec();
    }
    (0..width)
        .map(|i| {
            let src = if width > 1 { i * (data.len() - 1) / (width - 1) } else { 0 };
            data[src]
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_stretches_fewer_points_to_fill_the_width() {
        let data = vec![0, 100];
        let out = resample(&data, 5);
        assert_eq!(out.len(), 5);
        assert_eq!(out[0], 0);
        assert_eq!(out[4], 100);
    }

    #[test]
    fn resample_compresses_more_points_down_to_the_width() {
        let data: Vec<u64> = (0..60).collect();
        let out = resample(&data, 10);
        assert_eq!(out.len(), 10);
        assert_eq!(out[0], 0);
        assert_eq!(out[9], 59);
    }

    #[test]
    fn resample_is_a_noop_when_lengths_already_match() {
        let data = vec![1, 2, 3];
        assert_eq!(resample(&data, 3), data);
    }

    #[test]
    fn resample_handles_empty_data_and_zero_width() {
        assert_eq!(resample(&[], 4), vec![0, 0, 0, 0]);
        assert_eq!(resample(&[1, 2, 3], 0), Vec::<u64>::new());
    }
}
