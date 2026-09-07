use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::widgets::{Axis, Block, Borders, Chart, Clear, Dataset, GraphType, Paragraph};
use ratatui::Frame;

use super::theme;
use crate::app::entity::Entity;
use crate::app::AppState;

/// A measurement's line always gets the same color regardless of chart
/// position, so temperature/humidity/battery read consistently across
/// every popup rather than shuffling with whichever row happened to be
/// activated. Checked against `device_class` first (HA's own controlled
/// vocabulary), falling back to matching the entity_id/friendly_name for
/// dashboards that don't set it.
const TEMPERATURE_COLOR: Color = Color::Rgb(230, 160, 60); // amber/orange
const HUMIDITY_COLOR: Color = Color::Rgb(90, 160, 220); // blue
const BATTERY_COLOR: Color = Color::Rgb(210, 90, 90); // red
/// Cycled for any entity that isn't one of the three recognized kinds
/// above. The first (green) matches the single-series chart's own
/// original color, so an unrecognized primary entity with no card-mates
/// still renders exactly as before.
const OTHER_COLORS: [Color; 3] = [theme::ON, Color::Rgb(190, 130, 190), Color::Rgb(150, 150, 150)];

fn unit_of(app: &AppState, entity_id: &str) -> Option<String> {
    app.entities.get(entity_id).and_then(unit_of_entity)
}

fn unit_of_entity(entity: &Entity) -> Option<String> {
    entity.as_sensor().and_then(|s| s.unit().map(str::to_string))
}

/// This entity's chart line color, by recognized measurement kind - not
/// its position among the chart's datasets.
fn color_for(entity: &Entity, other_colors: &mut impl Iterator<Item = Color>) -> Color {
    if let Some(device_class) = entity.attributes.get("device_class").and_then(|v| v.as_str()) {
        match device_class {
            "temperature" => return TEMPERATURE_COLOR,
            "humidity" => return HUMIDITY_COLOR,
            "battery" => return BATTERY_COLOR,
            _ => {}
        }
    }
    let haystack = format!("{} {}", entity.entity_id, entity.friendly_name()).to_lowercase();
    if haystack.contains("temperature") {
        TEMPERATURE_COLOR
    } else if haystack.contains("humidity") {
        HUMIDITY_COLOR
    } else if haystack.contains("battery") {
        BATTERY_COLOR
    } else {
        other_colors.next().unwrap()
    }
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

    let primary_unit = unit_of(app, entity_id);
    let mates = app.detail_group();
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
    let same_unit_series: Vec<(&Entity, Vec<(f64, f64)>)> = same_unit
        .into_iter()
        .map(|e| (e, app.history_points_since(&e.entity_id, series.oldest())))
        .filter(|(_, pts)| !pts.is_empty())
        .collect();
    let other_unit_series: Vec<(&Entity, Vec<(f64, f64)>)> = other_unit
        .into_iter()
        .map(|e| (e, app.history_points_since(&e.entity_id, series.oldest())))
        .filter(|(_, pts)| !pts.is_empty())
        .collect();

    let x_tick_count = ((area.width / 12).clamp(2, 8)) as usize;
    let y_tick_count = ((area.height / 4).clamp(2, 6)) as usize;
    let x_labels = series.time_labels(x_tick_count);
    let x_max = series.points.last().map(|p| p.0).unwrap_or(0.0);

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
    let mut other_colors = OTHER_COLORS.iter().copied().cycle();
    let mut datasets = vec![{
        let color = app.entities.get(entity_id).map(|e| color_for(e, &mut other_colors)).unwrap_or(theme::ON);
        let mut d = Dataset::default()
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(color))
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
                .style(Style::default().fg(color_for(e, &mut other_colors)))
                .name(e.friendly_name().to_string())
                .data(pts),
        );
    }
    for (e, pts) in &rescaled_other {
        datasets.push(
            Dataset::default()
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(color_for(e, &mut other_colors)))
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
    fn color_for_recognizes_device_class_regardless_of_name() {
        let mut others = OTHER_COLORS.iter().copied().cycle();
        assert_eq!(color_for(&entity("sensor.foo", Some("temperature")), &mut others), TEMPERATURE_COLOR);
        assert_eq!(color_for(&entity("sensor.bar", Some("humidity")), &mut others), HUMIDITY_COLOR);
        assert_eq!(color_for(&entity("sensor.baz", Some("battery")), &mut others), BATTERY_COLOR);
    }

    #[test]
    fn color_for_falls_back_to_matching_the_entity_id_or_name() {
        // Mirrors dashboards (like an imported apexcharts-card) whose
        // series entries don't set `device_class` at all.
        let mut others = OTHER_COLORS.iter().copied().cycle();
        assert_eq!(color_for(&entity("sensor.kitchen_ble_temperature", None), &mut others), TEMPERATURE_COLOR);
        assert_eq!(color_for(&entity("sensor.kitchen_ble_humidity", None), &mut others), HUMIDITY_COLOR);
        assert_eq!(color_for(&entity("sensor.kitchen_ble_battery", None), &mut others), BATTERY_COLOR);
    }

    #[test]
    fn color_for_cycles_the_fallback_palette_for_unrecognized_sensors() {
        let mut others = OTHER_COLORS.iter().copied().cycle();
        assert_eq!(color_for(&entity("sensor.co2", None), &mut others), OTHER_COLORS[0]);
        assert_eq!(color_for(&entity("sensor.pressure", None), &mut others), OTHER_COLORS[1]);
    }

    /// A slow-changing sensor like a battery level often has just one
    /// recorded point over the history window (HA only stores changes,
    /// and it may not have changed at all) - it should still appear in a
    /// combined multi-series popup instead of silently vanishing because
    /// there's no "line" to draw for it.
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
