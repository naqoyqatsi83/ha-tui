use std::collections::HashMap;

use crate::ha::{AreaEntry, DeviceEntry, EntityRegistryEntry};

/// Caches the area/device/entity registries fetched once at startup, and
/// resolves each entity to a room name: its own `area_id` if set, else its
/// device's `area_id`, else no area (caller falls back to domain grouping).
#[derive(Debug, Clone, Default)]
pub struct Registry {
    areas: HashMap<String, AreaEntry>,
    entity_area: HashMap<String, String>,
}

impl Registry {
    pub fn build(areas: Vec<AreaEntry>, devices: Vec<DeviceEntry>, entities: Vec<EntityRegistryEntry>) -> Self {
        let device_area: HashMap<String, String> = devices
            .into_iter()
            .filter_map(|d| d.area_id.map(|area_id| (d.id, area_id)))
            .collect();

        let entity_area = entities
            .into_iter()
            .filter_map(|e| {
                let area_id = e
                    .area_id
                    .or_else(|| e.device_id.as_ref().and_then(|id| device_area.get(id).cloned()))?;
                Some((e.entity_id, area_id))
            })
            .collect();

        let areas = areas.into_iter().map(|a| (a.area_id.clone(), a)).collect();

        Registry { areas, entity_area }
    }

    /// The room/area name for an entity, if it (or its device) is assigned one.
    pub fn area_name_for(&self, entity_id: &str) -> Option<&str> {
        let area_id = self.entity_area.get(entity_id)?;
        self.areas.get(area_id).map(|a| a.name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn areas() -> Vec<AreaEntry> {
        vec![
            AreaEntry {
                area_id: "living_room".into(),
                name: "Living Room".into(),
            },
            AreaEntry {
                area_id: "bedroom".into(),
                name: "Bedroom".into(),
            },
        ]
    }

    #[test]
    fn entity_with_direct_area_id_resolves() {
        let registry = Registry::build(
            areas(),
            vec![],
            vec![EntityRegistryEntry {
                entity_id: "light.lamp".into(),
                device_id: None,
                area_id: Some("bedroom".into()),
            }],
        );
        assert_eq!(registry.area_name_for("light.lamp"), Some("Bedroom"));
    }

    #[test]
    fn entity_inherits_area_from_its_device() {
        let registry = Registry::build(
            areas(),
            vec![DeviceEntry {
                id: "dev1".into(),
                area_id: Some("living_room".into()),
            }],
            vec![EntityRegistryEntry {
                entity_id: "switch.tv_socket".into(),
                device_id: Some("dev1".into()),
                area_id: None,
            }],
        );
        assert_eq!(registry.area_name_for("switch.tv_socket"), Some("Living Room"));
    }

    #[test]
    fn direct_area_id_takes_precedence_over_device_area() {
        let registry = Registry::build(
            areas(),
            vec![DeviceEntry {
                id: "dev1".into(),
                area_id: Some("living_room".into()),
            }],
            vec![EntityRegistryEntry {
                entity_id: "sensor.x".into(),
                device_id: Some("dev1".into()),
                area_id: Some("bedroom".into()),
            }],
        );
        assert_eq!(registry.area_name_for("sensor.x"), Some("Bedroom"));
    }

    #[test]
    fn entity_with_no_area_anywhere_resolves_to_none() {
        let registry = Registry::build(
            areas(),
            vec![DeviceEntry {
                id: "dev1".into(),
                area_id: None,
            }],
            vec![EntityRegistryEntry {
                entity_id: "sensor.sun_next_dawn".into(),
                device_id: Some("dev1".into()),
                area_id: None,
            }],
        );
        assert_eq!(registry.area_name_for("sensor.sun_next_dawn"), None);
    }

    #[test]
    fn unknown_entity_resolves_to_none() {
        let registry = Registry::build(areas(), vec![], vec![]);
        assert_eq!(registry.area_name_for("sensor.not_in_registry"), None);
    }
}
