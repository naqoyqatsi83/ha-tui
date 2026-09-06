pub mod action;
pub mod entity;
pub mod registry;

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use entity::Entity;
use registry::Registry;
use serde_json::json;

use crate::ha::{Command, StateObject};

/// How long an optimistic UI update or a status message stays visible
/// before being cleared automatically (e.g. the service call's HA-side
/// result never arrived, or the user has had enough time to read it).
const PENDING_TIMEOUT: Duration = Duration::from_secs(5);
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);

/// A locally-applied guess at an entity's next display state, shown until
/// either a real `state_changed` event confirms it (cleared regardless of
/// whether the confirmed value matches - we trust HA once it answers) or
/// `PENDING_TIMEOUT` elapses (the call likely failed silently).
struct Pending {
    display_state: String,
    issued_at: Instant,
}

/// Holds the live entity map and the area/domain grouping derived from it,
/// plus which group/entity the UI currently has selected.
pub struct AppState {
    pub entities: HashMap<String, Entity>,
    pub registry: Registry,
    pub selected_group: usize,
    pub selected_entity: usize,
    pending: HashMap<String, Pending>,
    status: Option<(String, Instant)>,
}

impl AppState {
    pub fn new(states: Vec<StateObject>, registry: Registry) -> Self {
        let mut app = AppState {
            entities: HashMap::new(),
            registry,
            selected_group: 0,
            selected_entity: 0,
            pending: HashMap::new(),
            status: None,
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
        self.pending.remove(&entity.entity_id);
        self.entities.insert(entity.entity_id.clone(), entity);
        self.clamp_selection();
    }

    /// Removes an entity, e.g. when a `state_changed` event carries a
    /// `None` `new_state` (entity removed from HA).
    pub fn remove_entity(&mut self, entity_id: &str) {
        self.entities.remove(entity_id);
        self.pending.remove(entity_id);
        self.clamp_selection();
    }

    /// The text to show for an entity's state: its pending optimistic
    /// value if a service call is in flight, else its real display state.
    pub fn display_state(&self, entity: &Entity) -> String {
        match self.pending.get(&entity.entity_id) {
            Some(pending) => pending.display_state.clone(),
            None => entity.display_state(),
        }
    }

    pub fn status_message(&self) -> Option<&str> {
        self.status.as_ref().map(|(message, _)| message.as_str())
    }

    fn set_status(&mut self, message: impl Into<String>) {
        self.status = Some((message.into(), Instant::now()));
    }

    /// Toggles the selected entity if it's a light or switch, returning the
    /// `Command` to send to the WS task. Applies the optimistic flip
    /// immediately so the UI reflects it before HA confirms.
    pub fn toggle_selected(&mut self) -> Option<Command> {
        let entity = self.selected_entity()?;
        if entity.domain != "light" && entity.domain != "switch" {
            return None;
        }
        let entity_id = entity.entity_id.clone();
        let domain = entity.domain.clone();
        let new_state = if entity.state == "on" { "off" } else { "on" }.to_string();

        self.pending.insert(
            entity_id.clone(),
            Pending {
                display_state: new_state,
                issued_at: Instant::now(),
            },
        );
        self.set_status(format!("toggling {entity_id}..."));

        Some(Command::CallService {
            domain: domain.clone(),
            service: "toggle".to_string(),
            service_data: None,
            target: Some(json!({ "entity_id": entity_id })),
        })
    }

    /// `direction` is +1 (`Increase`) or -1 (`Decrease`); interpreted per
    /// the selected entity's domain (light brightness, climate target
    /// temperature). Returns `None` for domains with no adjustment.
    pub fn adjust_selected(&mut self, direction: i64) -> Option<Command> {
        let entity = self.selected_entity()?;
        let entity_id = entity.entity_id.clone();

        match entity.domain.as_str() {
            "light" => {
                const STEP: i64 = 25;
                let current = entity.as_light()?.brightness().unwrap_or(0) as i64;
                let new_brightness = (current + direction * STEP).clamp(0, 255);

                self.pending.insert(
                    entity_id.clone(),
                    Pending {
                        display_state: format!("on (brightness {new_brightness}/255)"),
                        issued_at: Instant::now(),
                    },
                );
                self.set_status(format!("setting {entity_id} brightness to {new_brightness}..."));

                Some(Command::CallService {
                    domain: "light".to_string(),
                    service: "turn_on".to_string(),
                    service_data: Some(json!({ "brightness": new_brightness })),
                    target: Some(json!({ "entity_id": entity_id })),
                })
            }
            "climate" => {
                const STEP: f64 = 0.5;
                let current_target = entity.as_climate()?.target_temperature().unwrap_or(20.0);
                let new_target = current_target + direction as f64 * STEP;

                self.pending.insert(
                    entity_id.clone(),
                    Pending {
                        display_state: format!("target {new_target:.1}°"),
                        issued_at: Instant::now(),
                    },
                );
                self.set_status(format!("setting {entity_id} target temp to {new_target:.1}..."));

                Some(Command::CallService {
                    domain: "climate".to_string(),
                    service: "set_temperature".to_string(),
                    service_data: Some(json!({ "temperature": new_target })),
                    target: Some(json!({ "entity_id": entity_id })),
                })
            }
            _ => None,
        }
    }

    /// Called when the WS task reports a failed service call.
    pub fn command_failed(&mut self, message: &str) {
        self.set_status(format!("error: {message}"));
    }

    /// Clears any pending optimistic update / status message that has
    /// outlived its timeout. Returns whether anything changed, so the
    /// render loop's periodic tick only redraws when it actually needs to.
    pub fn expire_stale(&mut self) -> bool {
        let before = self.pending.len();
        self.pending.retain(|_, p| p.issued_at.elapsed() < PENDING_TIMEOUT);
        let pending_changed = self.pending.len() != before;

        let status_changed = match &self.status {
            Some((_, at)) if at.elapsed() >= STATUS_TIMEOUT => {
                self.status = None;
                true
            }
            _ => false,
        };

        pending_changed || status_changed
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
            // Tie-break on entity_id so ordering is fully deterministic even
            // when two entities share a friendly_name - `entities` is a
            // HashMap mutated by every incoming state_changed event, so its
            // iteration order (the sort's tie-break otherwise) isn't
            // guaranteed stable between renders, which would let the
            // index-based selection silently point at a different entity.
            group.sort_by(|a, b| (a.friendly_name(), &a.entity_id).cmp(&(b.friendly_name(), &b.entity_id)));
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

    fn state_with_attrs(entity_id: &str, state: &str, attributes: serde_json::Value) -> StateObject {
        serde_json::from_value(serde_json::json!({
            "entity_id": entity_id,
            "state": state,
            "attributes": attributes,
            "last_updated": null,
            "last_changed": null,
        }))
        .unwrap()
    }

    #[test]
    fn toggle_selected_light_sends_toggle_and_shows_optimistic_state() {
        let mut app = AppState::new(vec![state("light.a", "on")], Registry::default());

        let cmd = app.toggle_selected().expect("light should be toggleable");
        match cmd {
            Command::CallService { domain, service, target, .. } => {
                assert_eq!(domain, "light");
                assert_eq!(service, "toggle");
                assert_eq!(target, Some(json!({ "entity_id": "light.a" })));
            }
        }

        // Real state hasn't changed yet, but the display reflects the guess.
        let entity = app.entities["light.a"].clone();
        assert_eq!(app.display_state(&entity), "off");
        assert_eq!(app.entities["light.a"].state, "on");
    }

    #[test]
    fn toggle_selected_is_noop_for_unsupported_domain() {
        let mut app = AppState::new(vec![state("sensor.temp", "20")], Registry::default());
        assert!(app.toggle_selected().is_none());
    }

    #[test]
    fn adjust_selected_light_steps_brightness_and_clamps() {
        let mut app = AppState::new(
            vec![state_with_attrs("light.a", "on", json!({ "brightness": 240 }))],
            Registry::default(),
        );

        let cmd = app.adjust_selected(1).expect("light brightness should be adjustable");
        match cmd {
            Command::CallService { domain, service, service_data, .. } => {
                assert_eq!(domain, "light");
                assert_eq!(service, "turn_on");
                // 240 + 25 clamps to 255
                assert_eq!(service_data, Some(json!({ "brightness": 255 })));
            }
        }
        let entity = app.entities["light.a"].clone();
        assert_eq!(app.display_state(&entity), "on (brightness 255/255)");
    }

    #[test]
    fn adjust_selected_climate_steps_target_temperature() {
        let mut app = AppState::new(
            vec![state_with_attrs("climate.a", "heat", json!({ "temperature": 21.0 }))],
            Registry::default(),
        );

        let cmd = app.adjust_selected(-1).expect("climate target temp should be adjustable");
        match cmd {
            Command::CallService { domain, service, service_data, .. } => {
                assert_eq!(domain, "climate");
                assert_eq!(service, "set_temperature");
                assert_eq!(service_data, Some(json!({ "temperature": 20.5 })));
            }
        }
    }

    #[test]
    fn real_update_clears_pending_optimistic_state() {
        let mut app = AppState::new(vec![state("switch.a", "off")], Registry::default());
        app.toggle_selected();
        let entity = app.entities["switch.a"].clone();
        assert_eq!(app.display_state(&entity), "on"); // optimistic

        // HA confirms - even though it happens to match the guess here,
        // reconciliation clears the pending entry unconditionally.
        app.apply_state(state("switch.a", "on"));
        let entity = app.entities["switch.a"].clone();
        assert_eq!(app.display_state(&entity), "on");
        assert!(app.pending.is_empty());
    }

    #[test]
    fn command_failed_sets_a_status_message() {
        let mut app = AppState::new(vec![state("switch.a", "on")], Registry::default());
        assert!(app.status_message().is_none());
        app.command_failed("entity not found");
        assert_eq!(app.status_message(), Some("error: entity not found"));
    }
}
