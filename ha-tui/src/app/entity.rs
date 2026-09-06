use serde_json::Value;

use crate::ha::StateObject;

/// A live entity: its identity, current state string, and raw attributes.
/// Domain-specific fields are accessed through the typed views below
/// (`as_light`, `as_switch`, ...) rather than duplicated onto this struct,
/// since most attributes are only meaningful for one domain.
#[derive(Debug, Clone)]
pub struct Entity {
    pub entity_id: String,
    pub domain: String,
    pub state: String,
    pub attributes: Value,
    pub last_updated: Option<String>,
}

impl Entity {
    pub fn from_state(state: StateObject) -> Self {
        Entity {
            domain: domain_of(&state.entity_id),
            entity_id: state.entity_id,
            state: state.state,
            attributes: state.attributes,
            last_updated: state.last_updated,
        }
    }

    pub fn friendly_name(&self) -> &str {
        self.attributes
            .get("friendly_name")
            .and_then(Value::as_str)
            .unwrap_or(&self.entity_id)
    }

    pub fn is_unavailable(&self) -> bool {
        matches!(self.state.as_str(), "unavailable" | "unknown")
    }

    pub fn as_light(&self) -> Option<LightView<'_>> {
        (self.domain == "light").then_some(LightView(self))
    }

    pub fn as_switch(&self) -> Option<SwitchView<'_>> {
        (self.domain == "switch").then_some(SwitchView(self))
    }

    pub fn as_climate(&self) -> Option<ClimateView<'_>> {
        (self.domain == "climate").then_some(ClimateView(self))
    }

    pub fn as_sensor(&self) -> Option<SensorView<'_>> {
        (self.domain == "sensor").then_some(SensorView(self))
    }

    /// Human-readable state text for the entity list, with domain-specific
    /// detail (brightness, target/current temperature, unit) where we have
    /// a typed view for it. `AppState::display_state` overrides this with
    /// a pending optimistic value when one is in flight.
    pub fn display_state(&self) -> String {
        if let Some(light) = self.as_light() {
            return match (light.is_on(), light.brightness()) {
                (true, Some(b)) => format!("on (brightness {b}/255)"),
                (true, None) => "on".to_string(),
                (false, _) => "off".to_string(),
            };
        }
        if let Some(climate) = self.as_climate() {
            let current = climate
                .current_temperature()
                .map(|t| format!("{t:.1}°"))
                .unwrap_or_else(|| "-".to_string());
            let target = climate
                .target_temperature()
                .map(|t| format!("{t:.1}°"))
                .unwrap_or_else(|| "-".to_string());
            return format!("{}  cur:{current} target:{target}", climate.hvac_mode());
        }
        if let Some(switch) = self.as_switch() {
            return if switch.is_on() { "on".to_string() } else { "off".to_string() };
        }
        if let Some(sensor) = self.as_sensor() {
            let value = round_numeric(sensor.value());
            return match sensor.unit() {
                Some(unit) => format!("{value} {unit}"),
                None => value,
            };
        }
        self.state.clone()
    }
}

/// Sensors often report far more decimal precision than is useful on a
/// narrow terminal panel (e.g. "30.8631578947368"); round to 2 places when
/// the value is numeric, otherwise pass it through unchanged (most sensor
/// states aren't numbers at all - "idle", timestamps, ...).
fn round_numeric(raw: &str) -> String {
    match raw.parse::<f64>() {
        Ok(n) => format!("{:.2}", (n * 100.0).round() / 100.0)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string(),
        Err(_) => raw.to_string(),
    }
}

pub fn domain_of(entity_id: &str) -> String {
    entity_id.split('.').next().unwrap_or_default().to_string()
}

fn f64_attr(entity: &Entity, key: &str) -> Option<f64> {
    entity.attributes.get(key).and_then(Value::as_f64)
}

pub struct LightView<'a>(&'a Entity);

impl LightView<'_> {
    pub fn is_on(&self) -> bool {
        self.0.state == "on"
    }

    pub fn brightness(&self) -> Option<u8> {
        self.0
            .attributes
            .get("brightness")
            .and_then(Value::as_u64)
            .map(|v| v as u8)
    }

    pub fn color_temp(&self) -> Option<u32> {
        self.0
            .attributes
            .get("color_temp")
            .and_then(Value::as_u64)
            .map(|v| v as u32)
    }
}

pub struct SwitchView<'a>(&'a Entity);

impl SwitchView<'_> {
    pub fn is_on(&self) -> bool {
        self.0.state == "on"
    }
}

pub struct ClimateView<'a>(&'a Entity);

impl ClimateView<'_> {
    /// HA models the climate entity's state as its hvac_mode ("off",
    /// "cool", "heat", ...).
    pub fn hvac_mode(&self) -> &str {
        &self.0.state
    }

    pub fn current_temperature(&self) -> Option<f64> {
        f64_attr(self.0, "current_temperature")
    }

    pub fn target_temperature(&self) -> Option<f64> {
        f64_attr(self.0, "temperature")
    }
}

pub struct SensorView<'a>(&'a Entity);

impl SensorView<'_> {
    pub fn value(&self) -> &str {
        &self.0.state
    }

    pub fn unit(&self) -> Option<&str> {
        self.0.attributes.get("unit_of_measurement").and_then(Value::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Sample payloads captured live from Phase 1's dump_states example
    // against a real HA instance (Sept 2026), covering the domains we
    // render specially.

    fn state_from_json(json: &str) -> StateObject {
        serde_json::from_str(json).expect("sample payload should deserialize")
    }

    #[test]
    fn light_view_reads_onoff_state() {
        let state = state_from_json(
            r#"{
                "entity_id": "light.lumi_v3_c9e7_light",
                "state": "off",
                "attributes": {
                    "color_mode": null,
                    "friendly_name": "Mi Control Hub Light",
                    "supported_color_modes": ["onoff"],
                    "supported_features": 0
                },
                "last_updated": "2026-09-06T04:29:40.175467+00:00",
                "last_changed": "2026-09-06T04:29:40.175467+00:00"
            }"#,
        );
        let entity = Entity::from_state(state);
        assert_eq!(entity.domain, "light");
        assert_eq!(entity.friendly_name(), "Mi Control Hub Light");
        let light = entity.as_light().expect("should be a light view");
        assert!(!light.is_on());
        assert_eq!(light.brightness(), None);
        assert!(entity.as_switch().is_none());
    }

    #[test]
    fn switch_view_reads_onoff_state() {
        let state = state_from_json(
            r#"{
                "entity_id": "switch.tv_socket",
                "state": "on",
                "attributes": { "friendly_name": "TV Socket" },
                "last_updated": "2026-09-03T20:57:24.887959+00:00",
                "last_changed": "2026-09-03T20:57:24.887959+00:00"
            }"#,
        );
        let entity = Entity::from_state(state);
        let switch = entity.as_switch().expect("should be a switch view");
        assert!(switch.is_on());
    }

    #[test]
    fn climate_view_reads_temperatures_and_mode() {
        let state = state_from_json(
            r#"{
                "entity_id": "climate.klima",
                "state": "off",
                "attributes": {
                    "current_temperature": 29,
                    "friendly_name": "Klima",
                    "hvac_modes": ["off", "cool", "dry", "fan_only", "auto", "heat"],
                    "max_temp": 35,
                    "min_temp": 7,
                    "temperature": 23
                },
                "last_updated": "2026-09-06T06:49:42.966360+00:00",
                "last_changed": "2026-09-06T06:49:42.966360+00:00"
            }"#,
        );
        let entity = Entity::from_state(state);
        let climate = entity.as_climate().expect("should be a climate view");
        assert_eq!(climate.hvac_mode(), "off");
        assert_eq!(climate.current_temperature(), Some(29.0));
        assert_eq!(climate.target_temperature(), Some(23.0));
    }

    #[test]
    fn sensor_view_reads_value_and_unit() {
        let state = state_from_json(
            r#"{
                "entity_id": "sensor.sm_a546b_battery_level",
                "state": "91",
                "attributes": {
                    "device_class": "battery",
                    "friendly_name": "SM-A546B Battery level",
                    "icon": "mdi:battery-90",
                    "state_class": "measurement",
                    "unit_of_measurement": "%"
                },
                "last_updated": "2026-09-06T12:10:21.306473+00:00",
                "last_changed": "2026-09-06T12:10:21.306473+00:00"
            }"#,
        );
        let entity = Entity::from_state(state);
        let sensor = entity.as_sensor().expect("should be a sensor view");
        assert_eq!(sensor.value(), "91");
        assert_eq!(sensor.unit(), Some("%"));
    }

    #[test]
    fn sensor_without_unit_falls_back_gracefully() {
        let state = state_from_json(
            r#"{
                "entity_id": "sensor.internet_latency_clean",
                "state": "52.253",
                "attributes": { "friendly_name": "Internet Latency Clean", "unit_of_measurement": "ms" },
                "last_updated": "2026-09-06T12:37:48.282799+00:00",
                "last_changed": "2026-09-06T12:37:48.282799+00:00"
            }"#,
        );
        let entity = Entity::from_state(state);
        assert_eq!(entity.as_sensor().unwrap().unit(), Some("ms"));
    }

    #[test]
    fn sensor_display_state_rounds_long_decimals_to_two_places() {
        let state = state_from_json(
            r#"{
                "entity_id": "sensor.workroom_ble_temperature",
                "state": "30.8631578947368",
                "attributes": { "unit_of_measurement": "°C" },
                "last_updated": null,
                "last_changed": null
            }"#,
        );
        let entity = Entity::from_state(state);
        assert_eq!(entity.display_state(), "30.86 °C");
    }

    #[test]
    fn sensor_display_state_trims_trailing_zeros_and_keeps_non_numeric_states() {
        let whole = Entity::from_state(state_from_json(
            r#"{"entity_id": "sensor.a", "state": "41.00", "attributes": {"unit_of_measurement": "%"}, "last_updated": null, "last_changed": null}"#,
        ));
        assert_eq!(whole.display_state(), "41 %");

        let non_numeric = Entity::from_state(state_from_json(
            r#"{"entity_id": "sensor.b", "state": "idle", "attributes": {}, "last_updated": null, "last_changed": null}"#,
        ));
        assert_eq!(non_numeric.display_state(), "idle");
    }

    #[test]
    fn unknown_domain_falls_back_to_entity_id_and_raw_state() {
        let state = state_from_json(
            r#"{
                "entity_id": "person.peter_guspan",
                "state": "home",
                "attributes": {},
                "last_updated": null,
                "last_changed": null
            }"#,
        );
        let entity = Entity::from_state(state);
        assert_eq!(entity.domain, "person");
        assert_eq!(entity.friendly_name(), "person.peter_guspan");
        assert!(entity.as_light().is_none());
        assert!(entity.as_switch().is_none());
        assert!(entity.as_climate().is_none());
        assert!(entity.as_sensor().is_none());
    }

    #[test]
    fn unavailable_and_unknown_states_are_flagged() {
        let unavailable = Entity::from_state(state_from_json(
            r#"{"entity_id": "media_player.tv_1", "state": "unavailable", "attributes": {}, "last_updated": null, "last_changed": null}"#,
        ));
        let unknown = Entity::from_state(state_from_json(
            r#"{"entity_id": "sensor.shmu_temperature", "state": "unknown", "attributes": {}, "last_updated": null, "last_changed": null}"#,
        ));
        let normal = Entity::from_state(state_from_json(
            r#"{"entity_id": "switch.tv_socket", "state": "on", "attributes": {}, "last_updated": null, "last_changed": null}"#,
        ));
        assert!(unavailable.is_unavailable());
        assert!(unknown.is_unavailable());
        assert!(!normal.is_unavailable());
    }
}
