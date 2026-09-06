pub mod client;
pub mod protocol;

pub use client::{as_state_changed, connect_with_backoff, run, Command, HaConnection, WsEvent};
pub use protocol::{AreaEntry, DeviceEntry, EntityRegistryEntry, EventPayload, Incoming, StateChangedData, StateObject};
