//! Best-effort extraction of a Lovelace dashboard config (`lovelace/config`'s
//! raw JSON) into ha-tui's own tab/card model, used to mirror the HA web
//! UI's own dashboard - not just its entity groupings, but its panel
//! structure too.
//!
//! Card schemas are loosely typed and vary a lot by card type (including
//! third-party `custom:*` cards), so this deliberately doesn't model the
//! schema - a "card" for our purposes is just any object that directly
//! references entities (`entity`, `entities`, or `series`); containers
//! (`cards`, `sections`, `badges`, `card`) are walked but don't become
//! panels themselves. Unrecognized card shapes are silently skipped rather
//! than erroring - a partial dashboard import is far more useful than none.

use std::collections::HashSet;

use serde_json::Value;

use crate::config::{DashboardCard, DashboardTab};

/// One tab per view, named after the view's `title` (falling back to its
/// `path`, then a positional name). Views that yield no cards at all (e.g.
/// a purely markdown/iframe view) are dropped.
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

            let mut cards = Vec::new();
            collect_cards(view, &mut cards);

            DashboardTab { name, entity_ids: Vec::new(), cards }
        })
        .filter(|tab| !tab.cards.is_empty())
        .collect()
}

fn collect_cards(value: &Value, out: &mut Vec<DashboardCard>) {
    let Value::Object(map) = value else { return };

    let mut entity_ids = Vec::new();
    let mut seen = HashSet::new();
    if let Some(Value::String(id)) = map.get("entity") {
        push(id, &mut entity_ids, &mut seen);
    }
    for key in ["entities", "series"] {
        if let Some(Value::Array(items)) = map.get(key) {
            collect_entity_array(items, &mut entity_ids, &mut seen);
        }
    }
    if !entity_ids.is_empty() {
        let title = ["title", "heading", "name"]
            .iter()
            .find_map(|key| map.get(*key).and_then(Value::as_str))
            .map(str::to_string);
        out.push(DashboardCard { title, entity_ids });
    }

    for key in ["cards", "sections", "badges"] {
        if let Some(Value::Array(items)) = map.get(key) {
            for item in items {
                match item {
                    Value::Object(_) => collect_cards(item, out),
                    // Legacy badges shorthand: a bare entity_id string,
                    // bucketed as its own untitled single-entity card.
                    Value::String(id) => {
                        let mut ids = Vec::new();
                        let mut seen = HashSet::new();
                        push(id, &mut ids, &mut seen);
                        if !ids.is_empty() {
                            out.push(DashboardCard { title: None, entity_ids: ids });
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // Used by e.g. "conditional" cards, which wrap a single nested card.
    if let Some(card) = map.get("card") {
        collect_cards(card, out);
    }
}

fn collect_entity_array(items: &[Value], out: &mut Vec<String>, seen: &mut HashSet<String>) {
    for item in items {
        match item {
            // Legacy entities-card shorthand: a plain entity_id string.
            Value::String(id) => push(id, out, seen),
            Value::Object(o) => {
                if let Some(Value::String(id)) = o.get("entity") {
                    push(id, out, seen);
                }
            }
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
    fn extracts_a_single_card_from_a_flat_entities_card() {
        let config = json!({
            "views": [{
                "title": "Living Room",
                "cards": [{
                    "type": "entities",
                    "title": "Living Room",
                    "entities": ["light.a", {"entity": "switch.b"}]
                }]
            }]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].name, "Living Room");
        assert_eq!(tabs[0].cards.len(), 1);
        assert_eq!(tabs[0].cards[0].title.as_deref(), Some("Living Room"));
        assert_eq!(tabs[0].cards[0].entity_ids, vec!["light.a", "switch.b"]);
    }

    #[test]
    fn grid_nesting_produces_one_card_per_leaf_card() {
        let config = json!({
            "views": [{
                "title": "Grid View",
                "cards": [{
                    "type": "grid",
                    "cards": [
                        {"type": "sensor", "name": "Temp", "entity": "sensor.temp"},
                        {"type": "thermostat", "entity": "climate.klima"}
                    ]
                }]
            }]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs[0].cards.len(), 2);
        assert_eq!(tabs[0].cards[0].title.as_deref(), Some("Temp"));
        assert_eq!(tabs[0].cards[0].entity_ids, vec!["sensor.temp"]);
        assert_eq!(tabs[0].cards[1].title, None);
        assert_eq!(tabs[0].cards[1].entity_ids, vec!["climate.klima"]);
    }

    #[test]
    fn sections_layout_produces_one_card_per_leaf_card() {
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
        assert_eq!(tabs[0].cards.len(), 2);
        assert_eq!(tabs[0].cards[0].entity_ids, vec!["climate.klima"]);
        assert_eq!(tabs[0].cards[1].entity_ids, vec!["input_boolean.x"]);
    }

    #[test]
    fn handles_series_and_conditional_card_nesting() {
        let config = json!({
            "views": [{
                "title": "Charts",
                "cards": [
                    {
                        "type": "custom:apexcharts-card",
                        "header": {"title": "Chart"},
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
        assert_eq!(tabs[0].cards.len(), 2);
        assert_eq!(tabs[0].cards[0].entity_ids, vec!["sensor.a", "sensor.b"]);
        assert_eq!(tabs[0].cards[1].entity_ids, vec!["light.c"]);
    }

    #[test]
    fn badges_become_their_own_untitled_cards() {
        let config = json!({
            "views": [{
                "title": "Home",
                "badges": ["person.a", {"entity": "person.b"}],
                "cards": [{"type": "entities", "entities": ["person.a"]}]
            }]
        });
        let tabs = extract_tabs(&config);
        // "cards" is walked before "badges" (fixed key order), so the
        // entities card comes first, then the two badge cards.
        assert_eq!(tabs[0].cards.len(), 3);
        assert_eq!(tabs[0].cards[0].entity_ids, vec!["person.a"]); // from "cards"
        assert_eq!(tabs[0].cards[1].entity_ids, vec!["person.a"]); // badge string
        assert_eq!(tabs[0].cards[2].entity_ids, vec!["person.b"]); // badge object
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
    fn views_with_no_extractable_cards_are_dropped() {
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
