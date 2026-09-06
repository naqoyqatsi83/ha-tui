pub mod action;
pub mod entity;
pub mod registry;

use std::collections::{BTreeMap, HashMap};

use entity::Entity;
use registry::Registry;

use crate::ha::StateObject;

/// Holds the live entity map and the area/domain grouping derived from it,
/// plus which group/entity the UI currently has selected.
pub struct AppState {
    pub entities: HashMap<String, Entity>,
    pub registry: Registry,
    pub selected_group: usize,
    pub selected_entity: usize,
}

impl AppState {
    pub fn new(states: Vec<StateObject>, registry: Registry) -> Self {
        let mut app = AppState {
            entities: HashMap::new(),
            registry,
            selected_group: 0,
            selected_entity: 0,
        };
        app.entities = states
            .into_iter()
            .map(|s| {
                let entity = Entity::from_state(s);
                (entity.entity_id.clone(), entity)
            })
            .collect();
        app.clamp_selection();
        app
    }

    /// Replaces the entire entity map and registry, e.g. after a
    /// reconnect's fresh snapshot (don't assume no events were missed
    /// while disconnected). Selection is clamped, not reset, so a
    /// reconnect doesn't jar the user back to the top of the list.
    pub fn replace_snapshot(&mut self, states: Vec<StateObject>, registry: Registry) {
        self.entities = states
            .into_iter()
            .map(|s| {
                let entity = Entity::from_state(s);
                (entity.entity_id.clone(), entity)
            })
            .collect();
        self.registry = registry;
        self.clamp_selection();
    }

    /// Applies a `state_changed` update (new state replaces the old).
    pub fn apply_state(&mut self, state: StateObject) {
        let entity = Entity::from_state(state);
        self.entities.insert(entity.entity_id.clone(), entity);
        self.clamp_selection();
    }

    /// Removes an entity, e.g. when a `state_changed` event carries a
    /// `None` `new_state` (entity removed from HA).
    pub fn remove_entity(&mut self, entity_id: &str) {
        self.entities.remove(entity_id);
        self.clamp_selection();
    }

    /// Groups entities by area name where the registry knows one, falling
    /// back to grouping by domain otherwise. Groups and their entities are
    /// both name-sorted for stable, predictable UI ordering.
    pub fn grouped(&self) -> BTreeMap<String, Vec<&Entity>> {
        let mut groups: BTreeMap<String, Vec<&Entity>> = BTreeMap::new();
        for entity in self.entities.values() {
            let group_name = self
                .registry
                .area_name_for(&entity.entity_id)
                .map(str::to_string)
                .unwrap_or_else(|| entity.domain.clone());
            groups.entry(group_name).or_default().push(entity);
        }
        for group in groups.values_mut() {
            group.sort_by(|a, b| a.friendly_name().cmp(b.friendly_name()));
        }
        groups
    }

    pub fn group_names(&self) -> Vec<String> {
        self.grouped().into_keys().collect()
    }

    pub fn selected_group_name(&self) -> Option<String> {
        self.group_names().into_iter().nth(self.selected_group)
    }

    /// Entities in the currently-selected group, name-sorted.
    pub fn selected_group_entities(&self) -> Vec<&Entity> {
        let groups = self.grouped();
        self.selected_group_name()
            .and_then(|name| groups.get(&name).cloned())
            .unwrap_or_default()
    }

    pub fn selected_entity(&self) -> Option<&Entity> {
        self.selected_group_entities().into_iter().nth(self.selected_entity)
    }

    pub fn next_group(&mut self) {
        let count = self.grouped().len();
        if count == 0 {
            return;
        }
        self.selected_group = (self.selected_group + 1) % count;
        self.selected_entity = 0;
    }

    pub fn prev_group(&mut self) {
        let count = self.grouped().len();
        if count == 0 {
            return;
        }
        self.selected_group = (self.selected_group + count - 1) % count;
        self.selected_entity = 0;
    }

    pub fn move_down(&mut self) {
        let len = self.selected_group_entities().len();
        if len == 0 {
            return;
        }
        self.selected_entity = (self.selected_entity + 1).min(len - 1);
    }

    pub fn move_up(&mut self) {
        self.selected_entity = self.selected_entity.saturating_sub(1);
    }

    /// Keeps `selected_group`/`selected_entity` in bounds after the entity
    /// map changes size (an entity appearing/disappearing, a reconnect
    /// snapshot reshaping groups, ...). Must run after every mutation.
    fn clamp_selection(&mut self) {
        let group_lens: Vec<usize> = self.grouped().values().map(Vec::len).collect();
        let group_count = group_lens.len();
        if group_count == 0 {
            self.selected_group = 0;
            self.selected_entity = 0;
            return;
        }
        if self.selected_group >= group_count {
            self.selected_group = group_count - 1;
        }
        let entity_count = group_lens[self.selected_group];
        if entity_count == 0 {
            self.selected_entity = 0;
        } else if self.selected_entity >= entity_count {
            self.selected_entity = entity_count - 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ha::{AreaEntry, EntityRegistryEntry};

    fn state(entity_id: &str, state: &str) -> StateObject {
        serde_json::from_value(serde_json::json!({
            "entity_id": entity_id,
            "state": state,
            "attributes": {},
            "last_updated": null,
            "last_changed": null,
        }))
        .unwrap()
    }

    #[test]
    fn groups_by_area_when_known_and_by_domain_otherwise() {
        let registry = Registry::build(
            vec![AreaEntry {
                area_id: "living_room".into(),
                name: "Living Room".into(),
            }],
            vec![],
            vec![EntityRegistryEntry {
                entity_id: "switch.tv_socket".into(),
                device_id: None,
                area_id: Some("living_room".into()),
            }],
        );
        let app = AppState::new(
            vec![state("switch.tv_socket", "on"), state("sensor.unmapped", "42")],
            registry,
        );

        let groups = app.grouped();
        assert_eq!(groups.get("Living Room").map(Vec::len), Some(1));
        assert_eq!(groups.get("sensor").map(Vec::len), Some(1));
    }

    #[test]
    fn apply_state_updates_existing_entity_in_place() {
        let mut app = AppState::new(vec![state("switch.tv_socket", "off")], Registry::default());
        assert_eq!(app.entities["switch.tv_socket"].state, "off");

        app.apply_state(state("switch.tv_socket", "on"));
        assert_eq!(app.entities["switch.tv_socket"].state, "on");
        assert_eq!(app.entities.len(), 1);
    }

    #[test]
    fn remove_entity_drops_it_from_the_map() {
        let mut app = AppState::new(vec![state("switch.tv_socket", "on")], Registry::default());
        app.remove_entity("switch.tv_socket");
        assert!(app.entities.is_empty());
    }

    #[test]
    fn navigation_moves_within_and_across_groups() {
        let mut app = AppState::new(
            vec![
                state("light.a", "on"),
                state("light.b", "off"),
                state("switch.c", "on"),
            ],
            Registry::default(),
        );
        // groups (sorted): "light" (a, b), "switch" (c)
        assert_eq!(app.selected_group_name().as_deref(), Some("light"));
        assert_eq!(app.selected_entity().unwrap().entity_id, "light.a");

        app.move_down();
        assert_eq!(app.selected_entity().unwrap().entity_id, "light.b");

        app.move_down(); // already at last entity in group, stays put
        assert_eq!(app.selected_entity().unwrap().entity_id, "light.b");

        app.next_group();
        assert_eq!(app.selected_group_name().as_deref(), Some("switch"));
        assert_eq!(app.selected_entity().unwrap().entity_id, "switch.c");

        app.prev_group();
        assert_eq!(app.selected_group_name().as_deref(), Some("light"));
        // selection resets to the top of the group on group change
        assert_eq!(app.selected_entity().unwrap().entity_id, "light.a");
    }

    #[test]
    fn selection_clamps_when_selected_entity_disappears() {
        let mut app = AppState::new(vec![state("light.a", "on"), state("light.b", "off")], Registry::default());
        app.move_down();
        assert_eq!(app.selected_entity, 1);

        app.remove_entity("light.b");
        assert_eq!(app.selected_entity, 0);
        assert_eq!(app.selected_entity().unwrap().entity_id, "light.a");
    }

    #[test]
    fn selection_clamps_when_all_entities_disappear() {
        let mut app = AppState::new(vec![state("light.a", "on")], Registry::default());
        app.remove_entity("light.a");
        assert_eq!(app.selected_group, 0);
        assert_eq!(app.selected_entity, 0);
        assert!(app.selected_entity().is_none());
    }
}
