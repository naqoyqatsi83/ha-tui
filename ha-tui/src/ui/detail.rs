use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::widgets::{Axis, Block, Borders, Chart, Clear, Dataset, GraphType, Paragraph};
use ratatui::Frame;

use super::theme;
use crate::app::entity::Entity;
use crate::app::AppState;

/// Chart line colors assigned by an entity's *position* within its
/// dashboard card, not by what kind of measurement it is - matching how
/// an unstyled apexcharts-card (or most charting tools) colors a series
/// list. As long as a card's entities are listed in the same order (e.g.
/// temperature, humidity, battery), every popup opened from it colors
/// them the same way, regardless of which row was actually activated.
const PALETTE: [Color; 11] = [
    Color::Rgb(230, 160, 60),  // amber
    Color::Rgb(90, 160, 220),  // blue
    Color::Rgb(210, 90, 90),   // red
    Color::Rgb(150, 100, 200), // violet
    Color::Rgb(210, 190, 80),  // yellow
    Color::Rgb(110, 170, 100), // green
    Color::Rgb(80, 180, 170),  // turquoise
    Color::Rgb(100, 100, 100), // graphite
    Color::Rgb(190, 110, 80),  // terracotta
    Color::Rgb(150, 150, 150), // grey
    Color::Rgb(60, 140, 140),  // teal
];

fn unit_of(app: &AppState, entity_id: &str) -> Option<String> {
    app.entities.get(entity_id).and_then(unit_of_entity)
}

fn unit_of_entity(entity: &Entity) -> Option<String> {
    entity.as_sensor().and_then(|s| s.unit().map(str::to_string))
}

/// `entity_id`'s chart line color: its position within `group` (the
/// on-screen card's full graphed entity list, in the dashboard's own
/// order - see `AppState::detail_group`) indexes into `PALETTE`, cycling
/// if the card has more graphed entities than colors.
fn color_at(group: &[&Entity], entity_id: &str) -> Color {
    let index = group.iter().position(|e| e.entity_id == entity_id).unwrap_or(0);
    PALETTE[index % PALETTE.len()]
}

/// Centered popup with a full X/Y-axis line chart of `entity_id`'s
/// history, opened via Enter on a graphed row. Other graphed entities
/// imported from the same dashboard card (e.g. an apexcharts-card
/// plotting temperature alongside humidity/battery) are plotted alongside
/// it: ones sharing `entity_id`'s unit share its axis outright, ones with
/// a different unit get a second, independently-scaled axis on the right
/// (its real values, rescaled onto the same plot as everything else).
/// Any key or an outside click closes it (see main's input dispatch).
pub fn render(frame: &mut Frame, app: &AppState, entity_id: &str) {
    let area = centered_percent(frame.area(), 80, 70);
    frame.render_widget(Clear, area);

    let name = app.entities.get(entity_id).map(|e| e.friendly_name()).unwrap_or(entity_id);
    let current = app.entities.get(entity_id).map(|e| app.display_state(e)).unwrap_or_default();
    let title = format!(" {name} - {current}  (any key to close) ");

    let Some(series) = app.history_series(entity_id) else {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD))
            .border_style(Style::default().fg(theme::ACCENT));
        let message = Paragraph::new("Not enough history yet - keep the app open a little longer.")
            .style(Style::default().fg(theme::TEXT_DIM))
            .block(block);
        frame.render_widget(message, area);
        return;
    };

    let x_tick_count = ((area.width / 12).clamp(2, 8)) as usize;
    let y_tick_count = ((area.height / 4).clamp(2, 6)) as usize;
    let x_labels = series.time_labels(x_tick_count);
    let x_max = series.points.last().map(|p| p.0).unwrap_or(0.0);

    let primary_unit = unit_of(app, entity_id);
    // The on-screen card's full graphed entity list, in the dashboard's
    // own order - `color_at` indexes into this so a line's color depends
    // on its position in the card, not on which row was activated.
    let group = app.detail_group();
    let mates: Vec<&Entity> = group.iter().filter(|e| e.entity_id != entity_id).copied().collect();
    let (same_unit, other_unit): (Vec<&Entity>, Vec<&Entity>) = mates.into_iter().partition(|e| unit_of_entity(e) == primary_unit);

    // Every other graphed card-mate's own history, already put on this
    // chart's time axis (`series.oldest()`) so it lines up with `entity_id`'s
    // points regardless of when each entity's buffer actually started. A
    // mate needs only one point (not two, unlike `history_series` for the
    // primary entity below) to still appear - a slow-changing sensor like
    // a battery level often has just one recorded point over the window
    // (HA's history only stores changes, and it may not have changed at
    // all), and it should still show up as a flat reference/legend entry
    // rather than silently vanishing from an otherwise multi-line chart.
    //
    // Each point's elapsed time is also clamped into [0, x_max] - a mate's
    // history can easily predate `entity_id`'s own oldest point (e.g. HA's
    // `history_during_period` handing back a battery sensor's actual last
    // change from days ago, rather than clamping it to the query window),
    // and an unclamped point there would plot off the left edge of the
    // chart's bounds and never actually be visible, even though it's
    // technically "there" in the dataset and the legend.
    let same_unit_series: Vec<(&Entity, Vec<(f64, f64)>)> = same_unit
        .into_iter()
        .map(|e| (e, clamp_points(app.history_points_since(&e.entity_id, series.oldest()), x_max)))
        .filter(|(_, pts)| !pts.is_empty())
        .collect();
    let other_unit_series: Vec<(&Entity, Vec<(f64, f64)>)> = other_unit
        .into_iter()
        .map(|e| (e, clamp_points(app.history_points_since(&e.entity_id, series.oldest()), x_max)))
        .filter(|(_, pts)| !pts.is_empty())
        .collect();

    // The primary (left) axis's range covers `entity_id` and every
    // same-unit card-mate - they share one real scale, no rescaling needed.
    let (mut y_min, mut y_max) = bounds(series.points.iter().map(|p| p.1));
    for (_, pts) in &same_unit_series {
        let (lo, hi) = bounds(pts.iter().map(|p| p.1));
        y_min = y_min.min(lo);
        y_max = y_max.max(hi);
    }

    // A different-unit card-mate group gets its own real range, rescaled
    // into the primary axis's plotting space so it draws on the same
    // chart - the secondary gutter (drawn after the chart) labels the
    // *real* values at those same plotted heights.
    let secondary_bounds =
        (!other_unit_series.is_empty()).then(|| bounds(other_unit_series.iter().flat_map(|(_, pts)| pts.iter().map(|p| p.1))));
    let rescaled_other: Vec<(&Entity, Vec<(f64, f64)>)> = match secondary_bounds {
        Some((sec_min, sec_max)) => other_unit_series
            .iter()
            .map(|(e, pts)| {
                let rescaled = pts
                    .iter()
                    .map(|&(x, v)| {
                        let frac = if sec_max > sec_min { (v - sec_min) / (sec_max - sec_min) } else { 0.5 };
                        (x, y_min + frac * (y_max - y_min))
                    })
                    .collect();
                (*e, rescaled)
            })
            .collect(),
        None => Vec::new(),
    };

    // A representative unit for the secondary group's own gutter labels -
    // same-unit by construction unless a card mixes 3+ units, an edge case
    // this picks one label for rather than trying to show them all.
    let secondary_unit = other_unit_series.first().and_then(|(e, _)| unit_of_entity(e));

    let has_other_series = !same_unit_series.is_empty() || !rescaled_other.is_empty();
    let mut datasets = vec![{
        let mut d = Dataset::default()
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(color_at(&group, entity_id)))
            .data(&series.points);
        if has_other_series {
            d = d.name(name.to_string());
        }
        d
    }];
    for (e, pts) in &same_unit_series {
        datasets.push(
            Dataset::default()
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(color_at(&group, &e.entity_id)))
                .name(e.friendly_name().to_string())
                .data(pts),
        );
    }
    for (e, pts) in &rescaled_other {
        datasets.push(
            Dataset::default()
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(color_at(&group, &e.entity_id)))
                .name(e.friendly_name().to_string())
                .data(pts),
        );
    }

    // Room for the secondary axis's own value labels on the right, only
    // when there's a second unit group to show.
    let gutter_width = if secondary_bounds.is_some() { 7u16.min(area.width / 4) } else { 0 };
    let chart_area = Rect { width: area.width.saturating_sub(gutter_width), ..area };

    // Every tick carries its own unit suffix (not just an axis title) so
    // it's unambiguous which scale a number belongs to even at a glance,
    // or if a title gets lost behind the legend box.
    let primary_unit_suffix = primary_unit.clone().unwrap_or_default();
    let y_labels: Vec<String> = (0..y_tick_count)
        .map(|i| {
            let frac = i as f64 / (y_tick_count - 1).max(1) as f64;
            format!("{:.1}{primary_unit_suffix}", y_min + frac * (y_max - y_min))
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(theme::ACCENT));

    let chart = Chart::new(datasets)
        .block(block)
        .x_axis(
            Axis::default()
                .title("time (UTC)")
                .style(Style::default().fg(theme::BORDER))
                .bounds([0.0, x_max])
                .labels(x_labels),
        )
        .y_axis(
            Axis::default()
                .title(primary_unit.clone().unwrap_or_else(|| "value".to_string()))
                .style(Style::default().fg(theme::BORDER))
                .bounds([y_min, y_max])
                .labels(y_labels),
        );
    frame.render_widget(chart, chart_area);

    if let Some((sec_min, sec_max)) = secondary_bounds {
        render_secondary_gutter(frame, area, chart_area, y_tick_count, sec_min, sec_max, secondary_unit.as_deref());
    }
}

/// Clamps every point's elapsed-time (x) into `[0, x_max]`, the chart's
/// own visible time bounds - see the call site for why a card-mate's
/// points can otherwise land outside them and never actually be seen.
fn clamp_points(points: Vec<(f64, f64)>, x_max: f64) -> Vec<(f64, f64)> {
    points.into_iter().map(|(x, v)| (x.clamp(0.0, x_max), v)).collect()
}

/// (min, max) of an f64 iterator, widened to a visible +-1 span if every
/// value was identical (a flat series would otherwise divide by zero when
/// normalized into an axis range).
fn bounds(values: impl Iterator<Item = f64>) -> (f64, f64) {
    let (min, max) = values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
    if max > min {
        (min, max)
    } else {
        (min - 1.0, min + 1.0)
    }
}

/// Draws the second axis's own real-value tick labels in a narrow strip to
/// the right of `chart_area`, at the same rows `Chart` places its own
/// (left) axis ticks at - mirrors `ratatui::widgets::Chart`'s internal
/// layout math (border inset, one row for X labels, one for the X axis
/// line) since the widget doesn't expose where its ticks actually landed.
fn render_secondary_gutter(frame: &mut Frame, area: Rect, chart_area: Rect, y_tick_count: usize, sec_min: f64, sec_max: f64, unit: Option<&str>) {
    let inner = Block::default().borders(Borders::ALL).inner(chart_area);
    let unit_suffix = unit.unwrap_or("");

    let mut y = inner.bottom().saturating_sub(1);
    if y > inner.top() {
        y -= 1; // x-axis tick label row
    }
    if y > inner.top() {
        y -= 1; // x-axis line row
    }
    let graph_top = inner.top();
    let graph_height = y.saturating_sub(inner.top()).saturating_add(1);

    let gutter_x = chart_area.right();
    let gutter_width = area.right().saturating_sub(gutter_x);
    if gutter_width == 0 {
        return;
    }

    for i in 0..y_tick_count {
        let dy = if y_tick_count > 1 {
            (i as u16) * graph_height.saturating_sub(1) / (y_tick_count as u16 - 1)
        } else {
            0
        };
        let row_y = graph_top + graph_height.saturating_sub(1).saturating_sub(dy);
        if row_y < area.top() || row_y >= area.bottom() {
            continue;
        }
        let frac = i as f64 / (y_tick_count - 1).max(1) as f64;
        let value = sec_min + frac * (sec_max - sec_min);
        let rect = Rect { x: gutter_x, y: row_y, width: gutter_width, height: 1 };
        frame.render_widget(Paragraph::new(format!(" {value:.0}{unit_suffix}")).style(Style::default().fg(theme::TEXT_DIM)), rect);
    }

    // A small header above the ticks names the secondary axis itself,
    // since (unlike the primary axis) it has no title slot of its own.
    if !unit_suffix.is_empty() && chart_area.top() < area.bottom() {
        let header = Rect { x: gutter_x, y: chart_area.top(), width: gutter_width, height: 1 };
        frame.render_widget(Paragraph::new(format!(" ({unit_suffix})")).style(Style::default().fg(theme::TEXT_DIM).add_modifier(Modifier::BOLD)), header);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::registry::Registry;
    use crate::config::{DashboardCard, DashboardTab};
    use crate::ha::StateObject;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::collections::HashMap;

    fn state(entity_id: &str, value: &str, unit: &str) -> StateObject {
        serde_json::from_value(serde_json::json!({
            "entity_id": entity_id,
            "state": value,
            "attributes": {"unit_of_measurement": unit},
            "last_updated": null,
            "last_changed": null,
        }))
        .unwrap()
    }

    fn entity(entity_id: &str, device_class: Option<&str>) -> Entity {
        let mut attributes = serde_json::json!({"friendly_name": entity_id});
        if let Some(dc) = device_class {
            attributes["device_class"] = dc.into();
        }
        Entity::from_state(
            serde_json::from_value(serde_json::json!({
                "entity_id": entity_id,
                "state": "1",
                "attributes": attributes,
                "last_updated": null,
                "last_changed": null,
            }))
            .unwrap(),
        )
    }

    #[test]
    fn color_at_follows_position_in_the_group_not_measurement_kind() {
        // Order-based, apexcharts-style: whichever entity is first in the
        // card's own list gets the first palette color, regardless of
        // what it actually measures.
        let battery = entity("sensor.battery", Some("battery"));
        let temp = entity("sensor.temp", Some("temperature"));
        let humidity = entity("sensor.humidity", Some("humidity"));
        let group = vec![&battery, &temp, &humidity];

        assert_eq!(color_at(&group, "sensor.battery"), PALETTE[0]);
        assert_eq!(color_at(&group, "sensor.temp"), PALETTE[1]);
        assert_eq!(color_at(&group, "sensor.humidity"), PALETTE[2]);
    }

    #[test]
    fn color_at_cycles_when_a_card_has_more_entities_than_palette_colors() {
        let entities: Vec<Entity> = (0..PALETTE.len() + 2).map(|i| entity(&format!("sensor.s{i}"), None)).collect();
        let group: Vec<&Entity> = entities.iter().collect();
        assert_eq!(color_at(&group, "sensor.s0"), PALETTE[0]);
        assert_eq!(color_at(&group, &format!("sensor.s{}", PALETTE.len())), PALETTE[0]);
        assert_eq!(color_at(&group, &format!("sensor.s{}", PALETTE.len() + 1)), PALETTE[1]);
    }

    /// A slow-changing sensor like a battery level often has just one
    /// recorded point over the history window (HA only stores changes,
    /// and it may not have changed at all) - it should still appear in a
    /// combined multi-series popup instead of silently vanishing because
    /// there's no "line" to draw for it.
    #[test]
    fn clamp_points_pulls_out_of_range_points_back_into_the_visible_span() {
        // Mirrors a slow-changing sensor (e.g. battery) whose actual last
        // recorded change predates the primary entity's own oldest point -
        // HA's history API doesn't clamp a "no change since before the
        // query window" value to the window start, so left unclamped this
        // would plot off the left edge of the chart and never be seen.
        let points = vec![(-500.0, 10.0), (5.0, 20.0), (999.0, 30.0)];
        assert_eq!(clamp_points(points, 100.0), vec![(0.0, 10.0), (5.0, 20.0), (100.0, 30.0)]);
    }

    #[test]
    fn a_card_mate_with_only_one_history_point_still_appears() {
        let mut tab = DashboardTab::new("Home", vec![]);
        tab.cards = vec![DashboardCard {
            title: Some("Kitchen".into()),
            entity_ids: vec!["sensor.temp".into(), "sensor.battery".into()],
            graph_entity_ids: vec!["sensor.temp".into(), "sensor.battery".into()],
        }];
        let mut app = AppState::new(
            vec![state("sensor.temp", "22.0", "°C"), state("sensor.battery", "92", "%")],
            Registry::default(),
            vec![tab],
        );
        let mut history = HashMap::new();
        history.insert("sensor.temp".to_string(), vec![(0.0, 20.0), (30.0, 22.0), (60.0, 24.0)]);
        history.insert("sensor.battery".to_string(), vec![(30.0, 92.0)]); // just one point
        app.apply_history(history);
        // Snapshots the card-mate group (as main.rs does before ever
        // rendering the popup) - `render` reads it via `detail_group()`.
        app.open_detail();

        // Large enough that `Chart` actually has room to draw its legend
        // (it hides the legend rather than cramming it into a tiny area).
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app, "sensor.temp")).unwrap();

        let text: String = terminal.backend().buffer().content.iter().map(|c| c.symbol()).collect();
        assert!(text.contains("sensor.battery"), "expected the one-point card-mate to still be named somewhere (e.g. the legend): {text}");
    }
}
