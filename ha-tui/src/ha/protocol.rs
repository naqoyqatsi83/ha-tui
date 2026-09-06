use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Messages sent from HA to us over the WS connection.
/// Kept permissive (`Other` fallback, `Option` fields) since payload shape
/// varies across HA versions/integrations and we must never panic on it.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum Incoming {
    #[serde(rename = "auth_required")]
    AuthRequired { ha_version: Option<String> },
    #[serde(rename = "auth_ok")]
    AuthOk { ha_version: Option<String> },
    #[serde(rename = "auth_invalid")]
    AuthInvalid { message: Option<String> },
    #[serde(rename = "result")]
    Result {
        id: u64,
        success: bool,
        #[serde(default)]
        result: Option<Value>,
        #[serde(default)]
        error: Option<ResultError>,
    },
    #[serde(rename = "event")]
    Event { id: u64, event: EventPayload },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResultError {
    pub code: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EventPayload {
    pub event_type: Option<String>,
    #[serde(default)]
    pub data: Option<Value>,
}

/// A parsed `state_changed` event's `data` field.
#[derive(Debug, Clone, Deserialize)]
pub struct StateChangedData {
    pub entity_id: String,
    pub old_state: Option<StateObject>,
    pub new_state: Option<StateObject>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StateObject {
    pub entity_id: String,
    pub state: String,
    #[serde(default)]
    pub attributes: Value,
    pub last_updated: Option<String>,
    pub last_changed: Option<String>,
}

/// Registry payloads deliberately only pull the handful of fields we use -
/// HA's registries carry many more (labels, config_entry_id, ...) that we
/// don't need and shouldn't break deserialization over if they change.
#[derive(Debug, Clone, Deserialize)]
pub struct AreaEntry {
    pub area_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceEntry {
    pub id: String,
    #[serde(default)]
    pub area_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EntityRegistryEntry {
    pub entity_id: String,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub area_id: Option<String>,
}

/// Messages we send to HA. Each has an `id` assigned by the client's
/// monotonic counter, except `auth` which precedes id assignment.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum Outgoing {
    #[serde(rename = "auth")]
    Auth { access_token: String },
    #[serde(rename = "subscribe_events")]
    SubscribeEvents {
        id: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        event_type: Option<String>,
    },
    #[serde(rename = "get_states")]
    GetStates { id: u64 },
    #[serde(rename = "config/area_registry/list")]
    AreaRegistryList { id: u64 },
    #[serde(rename = "config/device_registry/list")]
    DeviceRegistryList { id: u64 },
    #[serde(rename = "config/entity_registry/list")]
    EntityRegistryList { id: u64 },
    /// Fetches the default Lovelace dashboard's config (views/cards), used
    /// to mirror the web UI's own dashboard layout as ha-tui's tabs.
    #[serde(rename = "lovelace/config")]
    LovelaceConfig { id: u64 },
    /// Fetches historical states for a set of entities, used to seed
    /// sparkline graphs with a real trend rather than starting flat.
    #[serde(rename = "history/history_during_period")]
    HistoryDuringPeriod {
        id: u64,
        start_time: String,
        entity_ids: Vec<String>,
        minimal_response: bool,
        no_attributes: bool,
    },
    #[serde(rename = "call_service")]
    CallService {
        id: u64,
        domain: String,
        service: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        service_data: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<Value>,
    },
}
