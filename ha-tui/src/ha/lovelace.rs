//! Best-effort extraction of entity_ids from a Lovelace dashboard config
//! (`lovelace/config`'s raw JSON), used to mirror the HA web UI's own
//! dashboard as ha-tui tabs.
//!
//! Card schemas are loosely typed and vary a lot by card type (including
//! third-party `custom:*` cards), so this deliberately doesn't model the
//! schema - it just walks the handful of conventional keys cards actually
//! use to reference entities (`entity`, `entities`, `series`, and the
//! nesting keys `cards`/`sections`/`badges`/`card`), collecting anything
//! that looks like an entity_id (`domain.object_id`). Unrecognized card
//! shapes are silently skipped rather than erroring - a partial dashboard
//! import is far more useful than none.

use std::collections::HashSet;

use serde_json::Value;

use crate::config::DashboardTab;

/// One tab per view, named after the view's `title` (falling back to its
/// `path`, then a positional name). Views that yield no entities at all
/// (e.g. a purely markdown/iframe view) are dropped.
pub fn extract_tabs(config: &Value) -> Vec<DashboardTab> {
    let Some(views) = config.get("views").and_then(Value::as_array) else {
        return vec![];
    };

    views
        .iter()
        .enumerate()
        .map(|(i, view)| {
            let name = view
                .get("title")
                .and_then(Value::as_str)
                .or_else(|| view.get("path").and_then(Value::as_str))
                .map(str::to_string)
                .unwrap_or_else(|| format!("View {}", i + 1));

            let mut entity_ids = Vec::new();
            let mut seen = HashSet::new();
            collect(view, &mut entity_ids, &mut seen);

            DashboardTab { name, entity_ids }
        })
        .filter(|tab| !tab.entity_ids.is_empty())
        .collect()
}

fn collect(value: &Value, out: &mut Vec<String>, seen: &mut HashSet<String>) {
    let Value::Object(map) = value else { return };

    if let Some(Value::String(id)) = map.get("entity") {
        push(id, out, seen);
    }
    for key in ["entities", "series", "cards", "sections", "badges"] {
        if let Some(Value::Array(items)) = map.get(key) {
            collect_array(items, out, seen);
        }
    }
    // Used by e.g. "conditional" cards, which wrap a single nested card.
    if let Some(card) = map.get("card") {
        collect(card, out, seen);
    }
}

fn collect_array(items: &[Value], out: &mut Vec<String>, seen: &mut HashSet<String>) {
    for item in items {
        match item {
            // Legacy badges/entities shorthand: a plain entity_id string.
            Value::String(id) => push(id, out, seen),
            Value::Object(_) => collect(item, out, seen),
            _ => {}
        }
    }
}

fn push(candidate: &str, out: &mut Vec<String>, seen: &mut HashSet<String>) {
    // Cheap sanity check that this looks like an entity_id rather than some
    // other string field we happened to walk into.
    if candidate.contains('.') && seen.insert(candidate.to_string()) {
        out.push(candidate.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_entities_from_flat_entities_card() {
        let config = json!({
            "views": [{
                "title": "Living Room",
                "cards": [{
                    "type": "entities",
                    "entities": ["light.a", {"entity": "switch.b"}]
                }]
            }]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].name, "Living Room");
        assert_eq!(tabs[0].entity_ids, vec!["light.a", "switch.b"]);
    }

    #[test]
    fn recurses_through_grid_and_stack_nesting() {
        let config = json!({
            "views": [{
                "title": "Grid View",
                "cards": [{
                    "type": "grid",
                    "cards": [
                        {"type": "sensor", "entity": "sensor.temp"},
                        {"type": "thermostat", "entity": "climate.klima"}
                    ]
                }]
            }]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs[0].entity_ids, vec!["sensor.temp", "climate.klima"]);
    }

    #[test]
    fn recurses_through_sections_layout() {
        let config = json!({
            "views": [{
                "title": "AC",
                "sections": [
                    {"type": "grid", "cards": [{"entity": "climate.klima", "type": "thermostat"}]},
                    {"type": "grid", "cards": [{"entity": "input_boolean.x", "type": "tile"}]}
                ]
            }]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs[0].entity_ids, vec!["climate.klima", "input_boolean.x"]);
    }

    #[test]
    fn handles_series_and_conditional_card_nesting() {
        let config = json!({
            "views": [{
                "title": "Charts",
                "cards": [
                    {
                        "type": "custom:apexcharts-card",
                        "series": [{"entity": "sensor.a"}, {"entity": "sensor.b"}]
                    },
                    {
                        "type": "conditional",
                        "card": {"type": "tile", "entity": "light.c"}
                    }
                ]
            }]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs[0].entity_ids, vec!["sensor.a", "sensor.b", "light.c"]);
    }

    #[test]
    fn badges_contribute_entities_and_duplicates_are_deduped() {
        let config = json!({
            "views": [{
                "title": "Home",
                "badges": ["person.a", {"entity": "person.a"}],
                "cards": [{"type": "entities", "entities": ["person.a"]}]
            }]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs[0].entity_ids, vec!["person.a"]);
    }

    #[test]
    fn view_name_falls_back_to_path_then_position() {
        let config = json!({
            "views": [
                {"path": "no-title", "cards": [{"entity": "light.a"}]},
                {"cards": [{"entity": "light.b"}]}
            ]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs[0].name, "no-title");
        assert_eq!(tabs[1].name, "View 2");
    }

    #[test]
    fn views_with_no_extractable_entities_are_dropped() {
        let config = json!({
            "views": [
                {"title": "Markdown only", "cards": [{"type": "markdown", "content": "hi"}]},
                {"title": "Has entity", "cards": [{"entity": "light.a"}]}
            ]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].name, "Has entity");
    }

    #[test]
    fn strategy_only_config_with_no_views_yields_no_tabs() {
        let config = json!({ "strategy": { "type": "original-states" } });
        assert!(extract_tabs(&config).is_empty());
    }
}
