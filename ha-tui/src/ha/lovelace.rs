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

/// Card types that plot a history graph regardless of a "graph" key -
/// checked against a card's own `type`, suffix-matched for `custom:*`
/// third-party chart cards.
fn type_wants_graph(card_type: &str) -> bool {
    matches!(card_type, "history-graph" | "sensor-graph")
        || card_type.ends_with("apexcharts-card")
        || card_type.ends_with("mini-graph-card")
}

/// Whether a specific card object itself (not its children) wants a graph:
/// either its own `type`, or a `"graph"` key set to something other than
/// "none" (how the built-in mini "sensor" card enables its sparkline).
fn card_wants_graph(map: &serde_json::Map<String, Value>) -> bool {
    if let Some(t) = map.get("type").and_then(Value::as_str) {
        if type_wants_graph(t) {
            return true;
        }
    }
    matches!(map.get("graph").and_then(Value::as_str), Some(g) if g != "none")
}

#[derive(Default)]
struct Collected {
    entity_ids: Vec<String>,
    graph_entity_ids: Vec<String>,
    seen: HashSet<String>,
}

impl Collected {
    fn push(&mut self, id: &str, wants_graph: bool) {
        if id.contains('.') && self.seen.insert(id.to_string()) {
            self.entity_ids.push(id.to_string());
            if wants_graph {
                self.graph_entity_ids.push(id.to_string());
            }
        }
    }
}

fn single_card_from_badge(item: &Value) -> Option<DashboardCard> {
    let mut collected = Collected::default();
    match item {
        Value::String(id) => collected.push(id, false),
        Value::Object(o) => {
            if let Some(Value::String(id)) = o.get("entity") {
                collected.push(id, false);
            }
        }
        _ => {}
    }
    (!collected.entity_ids.is_empty()).then_some(DashboardCard {
        title: None,
        entity_ids: collected.entity_ids,
        graph_entity_ids: collected.graph_entity_ids,
    })
}

/// Collects every entity referenced anywhere inside `value` (however
/// deeply nested) into one card, titled from whatever naming hint we can
/// find in the subtree. Returns `None` if no entities were found at all
/// (e.g. a markdown or iframe card).
fn merge_into_one_card(value: &Value) -> Option<DashboardCard> {
    let mut collected = Collected::default();
    collect_entities(value, &mut collected);
    if collected.entity_ids.is_empty() {
        return None;
    }
    Some(DashboardCard {
        title: find_title(value),
        entity_ids: collected.entity_ids,
        graph_entity_ids: collected.graph_entity_ids,
    })
}

fn collect_entities(value: &Value, collected: &mut Collected) {
    let Value::Object(map) = value else { return };
    let wants_graph = card_wants_graph(map);

    if let Some(Value::String(id)) = map.get("entity") {
        collected.push(id, wants_graph);
    }
    for key in ["entities", "series"] {
        if let Some(Value::Array(items)) = map.get(key) {
            collect_entity_array(items, collected, wants_graph);
        }
    }
    for key in ["cards", "sections"] {
        if let Some(Value::Array(items)) = map.get(key) {
            for item in items {
                collect_entities(item, collected);
            }
        }
    }
    // Used by e.g. "conditional" cards, which wrap a single nested card.
    if let Some(card) = map.get("card") {
        collect_entities(card, collected);
    }
}

fn collect_entity_array(items: &[Value], collected: &mut Collected, wants_graph: bool) {
    for item in items {
        match item {
            // Legacy entities-card shorthand: a plain entity_id string.
            Value::String(id) => collected.push(id, wants_graph),
            Value::Object(o) => {
                if let Some(Value::String(id)) = o.get("entity") {
                    collected.push(id, wants_graph);
                }
            }
            _ => {}
        }
    }
}

/// Best-effort label for a merged card, tried in order:
/// 1. The container's own `title`/`heading`/`name` (including nested under
///    `header.title`, how e.g. `custom:apexcharts-card` names itself).
/// 2. A nested `"type": "heading"` card's `heading` text (how the modern
///    sections layout commonly labels a section).
/// 3. The common leading words shared by every leaf card's own name found
///    anywhere inside (e.g. a "grid" of "Kitchen temperature" +
///    "Kitchen humidity" mini cards picks up "Kitchen" - the single-leaf
///    case naturally reduces to just that leaf's own name).
fn find_title(value: &Value) -> Option<String> {
    if let Value::Object(map) = value {
        if let Some(title) = own_title(map) {
            return Some(title);
        }
    }
    find_heading_card(value).or_else(|| common_leaf_title(value))
}

fn own_title(map: &serde_json::Map<String, Value>) -> Option<String> {
    ["title", "heading", "name"]
        .iter()
        .find_map(|key| map.get(*key).and_then(Value::as_str))
        .or_else(|| map.get("header").and_then(|h| h.get("title")).and_then(Value::as_str))
        .map(str::to_string)
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

/// Every leaf card's own name found anywhere inside `value`, reduced to
/// the leading words they all share ("Kitchen temperature" + "Kitchen
/// humidity" -> "Kitchen"). A single leaf's name passes through unchanged;
/// no common leading word at all (or no named leaves) gives `None`.
fn common_leaf_title(value: &Value) -> Option<String> {
    let mut names = Vec::new();
    collect_leaf_names(value, &mut names);
    common_word_prefix(&names)
}

fn collect_leaf_names(value: &Value, out: &mut Vec<String>) {
    let Value::Object(map) = value else { return };
    let has_entities = map.contains_key("entity") || map.contains_key("entities") || map.contains_key("series");
    if has_entities {
        if let Some(title) = own_title(map) {
            out.push(title);
        }
    }
    for key in ["cards", "sections"] {
        if let Some(Value::Array(items)) = map.get(key) {
            for item in items {
                collect_leaf_names(item, out);
            }
        }
    }
    if let Some(card) = map.get("card") {
        collect_leaf_names(card, out);
    }
}

fn common_word_prefix(names: &[String]) -> Option<String> {
    let word_lists: Vec<Vec<&str>> = names.iter().map(|n| n.split_whitespace().collect()).collect();
    let min_len = word_lists.iter().map(Vec::len).min()?;

    let mut common = Vec::new();
    for i in 0..min_len {
        let word = word_lists[0][i];
        if word_lists.iter().all(|words| words[i] == word) {
            common.push(word);
        } else {
            break;
        }
    }

    (!common.is_empty()).then(|| common.join(" "))
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
    fn sensor_cards_with_a_graph_key_are_flagged_for_sparklines() {
        let config = json!({
            "views": [{
                "title": "Home",
                "cards": [{
                    "type": "grid",
                    "cards": [
                        {"type": "sensor", "name": "Kitchen temp", "entity": "sensor.temp", "graph": "line"},
                        {"type": "sensor", "name": "Kitchen humidity", "entity": "sensor.humidity"}
                    ]
                }]
            }]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs[0].cards[0].entity_ids, vec!["sensor.temp", "sensor.humidity"]);
        assert_eq!(tabs[0].cards[0].graph_entity_ids, vec!["sensor.temp"]);
    }

    #[test]
    fn history_graph_and_apexcharts_cards_flag_all_their_entities() {
        let config = json!({
            "views": [{
                "title": "Flood",
                "cards": [
                    {"type": "history-graph", "entities": ["binary_sensor.flood"]},
                    {"type": "custom:apexcharts-card", "series": [{"entity": "sensor.a"}, {"entity": "sensor.b"}]}
                ]
            }]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs[0].cards[0].graph_entity_ids, vec!["binary_sensor.flood"]);
        assert_eq!(tabs[0].cards[1].graph_entity_ids, vec!["sensor.a", "sensor.b"]);
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
        // Common leading words across the merged leaves' own names, not
        // just the first one's full name - "Kitchen", not "Kitchen
        // temperature" (misleading once the card also shows humidity).
        assert_eq!(tabs[0].cards[0].title.as_deref(), Some("Kitchen"));
        assert_eq!(tabs[0].cards[0].entity_ids, vec!["sensor.kitchen_temp", "sensor.kitchen_humidity"]);
    }

    #[test]
    fn leaves_with_no_common_leading_word_leave_the_card_untitled() {
        let config = json!({
            "views": [{
                "title": "Home",
                "cards": [{
                    "type": "grid",
                    "cards": [
                        {"type": "sensor", "name": "Kitchen temperature", "entity": "sensor.a"},
                        {"type": "sensor", "name": "Living room humidity", "entity": "sensor.b"}
                    ]
                }]
            }]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs[0].cards[0].title, None);
    }

    #[test]
    fn a_card_title_nested_under_header_is_found() {
        // How custom:apexcharts-card names itself - a plain top-level
        // "title"/"heading"/"name" check misses this entirely.
        let config = json!({
            "views": [{
                "title": "Home Detailed",
                "cards": [{
                    "type": "custom:apexcharts-card",
                    "header": { "show": true, "title": "Kitchen" },
                    "series": [{"entity": "sensor.temp"}, {"entity": "sensor.humidity"}, {"entity": "sensor.battery"}]
                }]
            }]
        });
        let tabs = extract_tabs(&config);
        assert_eq!(tabs[0].cards[0].title.as_deref(), Some("Kitchen"));
        assert_eq!(tabs[0].cards[0].entity_ids, vec!["sensor.temp", "sensor.humidity", "sensor.battery"]);
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
