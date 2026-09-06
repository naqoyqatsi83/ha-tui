//! Best-effort extraction of a Lovelace dashboard config (`lovelace/config`'s
//! raw JSON) into ha-tui's own tab/card model, used to mirror the HA web
//! UI's own dashboard - not just its entity groupings, but its panel
//! structure too.
//!
//! Card schemas are loosely typed and vary a lot by card type (including
//! third-party `custom:*` cards), so this deliberately doesn't model the
//! schema. The key structural rule mirrors how Lovelace actually renders a
//! view: each *top-level* entry in a view's `cards` (masonry layout) or
//! `sections` (modern grid layout) array is one visual panel, however much
//! nesting it contains inside (a "grid" of several mini sensor cards, a
//! "sections" section stacking a chart and history graphs, ...) - so each
//! one becomes exactly one `DashboardCard`, with every entity found
//! anywhere inside it (however deep) merged into that one card. Splitting
//! on every nested leaf card instead (the previous approach here) doesn't
//! match what the dashboard actually looks like: HA commonly groups
//! several small cards (e.g. a room's temperature + humidity) inside one
//! "grid" wrapper specifically so they render together as one panel.

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

            if let Some(Value::Array(items)) = view.get("badges") {
                for item in items {
                    if let Some(card) = single_card_from_badge(item) {
                        cards.push(card);
                    }
                }
            }

            // Modern "sections" layout takes precedence when present and
            // non-empty; masonry views use "cards" directly instead.
            let top_level = view
                .get("sections")
                .and_then(Value::as_array)
                .filter(|items| !items.is_empty())
                .or_else(|| view.get("cards").and_then(Value::as_array));

            if let Some(items) = top_level {
                for item in items {
                    if let Some(card) = merge_into_one_card(item) {
                        cards.push(card);
                    }
                }
            }

            DashboardTab { name, entity_ids: Vec::new(), cards }
        })
        .filter(|tab| !tab.cards.is_empty())
        .collect()
}

fn single_card_from_badge(item: &Value) -> Option<DashboardCard> {
    let mut entity_ids = Vec::new();
    let mut seen = HashSet::new();
    match item {
        Value::String(id) => push(id, &mut entity_ids, &mut seen),
        Value::Object(o) => {
            if let Some(Value::String(id)) = o.get("entity") {
                push(id, &mut entity_ids, &mut seen);
            }
        }
        _ => {}
    }
    (!entity_ids.is_empty()).then_some(DashboardCard { title: None, entity_ids })
}

/// Collects every entity referenced anywhere inside `value` (however
/// deeply nested) into one card, titled from whatever naming hint we can
/// find in the subtree. Returns `None` if no entities were found at all
/// (e.g. a markdown or iframe card).
fn merge_into_one_card(value: &Value) -> Option<DashboardCard> {
    let mut entity_ids = Vec::new();
    let mut seen = HashSet::new();
    collect_entities(value, &mut entity_ids, &mut seen);
    if entity_ids.is_empty() {
        return None;
    }
    Some(DashboardCard { title: find_title(value), entity_ids })
}

fn collect_entities(value: &Value, out: &mut Vec<String>, seen: &mut HashSet<String>) {
    let Value::Object(map) = value else { return };

    if let Some(Value::String(id)) = map.get("entity") {
        push(id, out, seen);
    }
    for key in ["entities", "series"] {
        if let Some(Value::Array(items)) = map.get(key) {
            collect_entity_array(items, out, seen);
        }
    }
    for key in ["cards", "sections"] {
        if let Some(Value::Array(items)) = map.get(key) {
            for item in items {
                collect_entities(item, out, seen);
            }
        }
    }
    // Used by e.g. "conditional" cards, which wrap a single nested card.
    if let Some(card) = map.get("card") {
        collect_entities(card, out, seen);
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

/// Best-effort label for a merged card, tried in order:
/// 1. The container's own `title`/`heading`/`name`.
/// 2. A nested `"type": "heading"` card's `heading` text (how the modern
///    sections layout commonly labels a section).
/// 3. The first leaf card's own `title`/`heading`/`name` found anywhere
///    inside (e.g. a "grid" of unnamed sensor cards picks up the first
///    one's name - an approximation, since the card may hold more than
///    just that one entity, but better than no label at all).
fn find_title(value: &Value) -> Option<String> {
    if let Value::Object(map) = value {
        if let Some(title) = own_title(map) {
            return Some(title);
        }
    }
    find_heading_card(value).or_else(|| find_first_named_leaf(value))
}

fn own_title(map: &serde_json::Map<String, Value>) -> Option<String> {
    ["title", "heading", "name"].iter().find_map(|key| map.get(*key).and_then(Value::as_str)).map(str::to_string)
}

fn find_heading_card(value: &Value) -> Option<String> {
    let Value::Object(map) = value else { return None };
    if map.get("type").and_then(Value::as_str) == Some("heading") {
        if let Some(h) = map.get("heading").and_then(Value::as_str) {
            return Some(h.to_string());
        }
    }
    for key in ["cards", "sections"] {
        if let Some(Value::Array(items)) = map.get(key) {
            for item in items {
                if let Some(t) = find_heading_card(item) {
                    return Some(t);
                }
            }
        }
    }
    map.get("card").and_then(find_heading_card)
}

fn find_first_named_leaf(value: &Value) -> Option<String> {
    let Value::Object(map) = value else { return None };
    let has_entities = map.contains_key("entity") || map.contains_key("entities") || map.contains_key("series");
    if has_entities {
        if let Some(title) = own_title(map) {
            return Some(title);
        }
    }
    for key in ["cards", "sections"] {
        if let Some(Value::Array(items)) = map.get(key) {
            for item in items {
                if let Some(t) = find_first_named_leaf(item) {
                    return Some(t);
                }
            }
        }
    }
    map.get("card").and_then(find_first_named_leaf)
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
    fn a_grid_of_related_mini_cards_merges_into_one_panel() {
        // The real-world case this fixes: HA commonly wraps a room's
        // temperature + humidity sensor cards in one "grid" so they render
        // together as a single panel - that grouping should carry through,
        // not get split back into two panels.
        let config = json!({
            "views": [{
                "title": "Home",
                "cards": [{
                    "type": "grid",
                    "cards": [
                        {"type": "sensor", "name": "Kitchen temperature", "entity": "sensor.kitchen_temp"},
                        {"type": "sensor", "name": "Kitchen humidity", "entity": "sensor.kitchen_humidity"}
                    ],
                    "columns": 2
                }]
            }]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs[0].cards.len(), 1);
        assert_eq!(tabs[0].cards[0].title.as_deref(), Some("Kitchen temperature"));
        assert_eq!(tabs[0].cards[0].entity_ids, vec!["sensor.kitchen_temp", "sensor.kitchen_humidity"]);
    }

    #[test]
    fn each_top_level_masonry_card_is_its_own_panel() {
        let config = json!({
            "views": [{
                "title": "Grid View",
                "cards": [
                    {"type": "sensor", "name": "Temp", "entity": "sensor.temp"},
                    {"type": "thermostat", "entity": "climate.klima"}
                ]
            }]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs[0].cards.len(), 2);
        assert_eq!(tabs[0].cards[0].title.as_deref(), Some("Temp"));
        assert_eq!(tabs[0].cards[1].entity_ids, vec!["climate.klima"]);
    }

    #[test]
    fn each_top_level_section_is_its_own_panel_titled_from_its_heading_card() {
        let config = json!({
            "views": [{
                "title": "Flood Sensors",
                "sections": [
                    {
                        "type": "grid",
                        "cards": [
                            {"type": "heading", "heading": "Kitchen", "heading_style": "title"},
                            {"type": "custom:apexcharts-card", "series": [{"entity": "sensor.temp"}, {"entity": "sensor.battery"}]},
                            {"type": "history-graph", "entities": [{"entity": "binary_sensor.flood"}]}
                        ]
                    },
                    {"type": "grid", "cards": [{"entity": "input_boolean.x", "type": "tile"}]}
                ]
            }]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs[0].cards.len(), 2);
        assert_eq!(tabs[0].cards[0].title.as_deref(), Some("Kitchen"));
        assert_eq!(tabs[0].cards[0].entity_ids, vec!["sensor.temp", "sensor.battery", "binary_sensor.flood"]);
        assert_eq!(tabs[0].cards[1].title, None);
        assert_eq!(tabs[0].cards[1].entity_ids, vec!["input_boolean.x"]);
    }

    #[test]
    fn conditional_card_wrapping_is_transparent() {
        let config = json!({
            "views": [{
                "title": "Charts",
                "cards": [{
                    "type": "conditional",
                    "card": {"type": "tile", "entity": "light.c"}
                }]
            }]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs[0].cards.len(), 1);
        assert_eq!(tabs[0].cards[0].entity_ids, vec!["light.c"]);
    }

    #[test]
    fn badges_become_their_own_untitled_cards_before_the_main_cards() {
        let config = json!({
            "views": [{
                "title": "Home",
                "badges": ["person.a", {"entity": "person.b"}],
                "cards": [{"type": "entities", "entities": ["person.c"]}]
            }]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs[0].cards.len(), 3);
        assert_eq!(tabs[0].cards[0].entity_ids, vec!["person.a"]);
        assert_eq!(tabs[0].cards[1].entity_ids, vec!["person.b"]);
        assert_eq!(tabs[0].cards[2].entity_ids, vec!["person.c"]);
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
