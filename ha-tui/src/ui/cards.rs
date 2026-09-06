use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use super::theme;
use crate::app::entity::Entity;
use crate::app::{AppState, ResolvedCard};

const TARGET_CARD_WIDTH: u16 = 34;
const MAX_COLUMNS: usize = 4;
const MIN_CARD_HEIGHT: u16 = 3;
const MAX_CARD_HEIGHT: u16 = 12;

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
            let max_entities = row.iter().map(|c| c.entities.len()).max().unwrap_or(0);
            (max_entities as u16 + 2).clamp(MIN_CARD_HEIGHT, MAX_CARD_HEIGHT)
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

    // Capacity is the block's inner height (area minus top+bottom border).
    let capacity = area.height.saturating_sub(2) as usize;
    let truncated = capacity > 0 && card.entities.len() > capacity;
    let visible_count = if truncated { capacity.saturating_sub(1) } else { card.entities.len() };

    // Give the name column roughly half the card's inner width (minimum
    // enough for short names), state text gets the rest.
    let inner_width = area.width.saturating_sub(2) as usize;
    let name_width = (inner_width * 55 / 100).clamp(10, inner_width.saturating_sub(4).max(10));

    let mut items: Vec<ListItem> = card.entities[..visible_count.min(card.entities.len())]
        .iter()
        .map(|entity| {
            let state_display = app.display_state(entity);
            let text = format!("{:<name_width$} {}", truncate(entity.friendly_name(), name_width), state_display);
            ListItem::new(text).style(row_style(entity))
        })
        .collect();
    if truncated {
        let hidden = card.entities.len() - visible_count;
        items.push(ListItem::new(format!("+{hidden} more")).style(Style::default().fg(theme::TEXT_DIM)));
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().bg(theme::HIGHLIGHT_BG).fg(theme::HIGHLIGHT_FG).add_modifier(Modifier::BOLD));

    let mut state = ListState::default();
    if let Some(row) = selected_row {
        if row < visible_count {
            state.select(Some(row));
        }
    }

    frame.render_stateful_widget(list, area, &mut state);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}

/// Color by state: on/active = green, unavailable/unknown = dimmed,
/// everything else default.
fn row_style(entity: &Entity) -> Style {
    if entity.is_unavailable() {
        return Style::default().fg(theme::UNAVAILABLE);
    }
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
}
