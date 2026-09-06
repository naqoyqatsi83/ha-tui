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
        let entities = states
            .into_iter()
            .map(|s| {
                let entity = Entity::from_state(s);
                (entity.entity_id.clone(), entity)
            })
            .collect();

        AppState {
            entities,
            registry,
            selected_group: 0,
            selected_entity: 0,
        }
    }

    /// Applies a `state_changed` update (new state replaces the old).
    pub fn apply_state(&mut self, state: StateObject) {
        let entity = Entity::from_state(state);
        self.entities.insert(entity.entity_id.clone(), entity);
    }

    /// Removes an entity, e.g. when a `state_changed` event carries a
    /// `None` `new_state` (entity removed from HA).
    pub fn remove_entity(&mut self, entity_id: &str) {
        self.entities.remove(entity_id);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use registry::{AreaEntry, EntityRegistryEntry};

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
}
