use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

use super::protocol::{AreaEntry, DeviceEntry, EntityRegistryEntry, EventPayload, Incoming, Outgoing, StateObject};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// A single, authenticated connection to a Home Assistant instance.
/// Not resilient by itself - see [`connect_with_backoff`] for reconnect.
pub struct HaConnection {
    socket: WsStream,
    next_id: AtomicU64,
    /// `Event` messages that arrived while `await_result` was waiting for
    /// an unrelated request's ack (HA can interleave a subscription's
    /// event notifications with any other request's result on the same
    /// socket). Queued here instead of dropped, and drained by
    /// `read_incoming` before it reads the socket again, so the run()
    /// loop's event subscriber never misses a `state_changed` that
    /// happened to arrive mid-`call_service`.
    pending_events: VecDeque<Incoming>,
}

impl HaConnection {
    /// Connects, performs the HA auth handshake, and returns a ready
    /// connection. `insecure_skip_verify` disables TLS cert validation,
    /// meant only for trusted local instances with self-signed certs.
    pub async fn connect(base_url: &str, token: &str, insecure_skip_verify: bool) -> Result<Self> {
        let ws_url = to_ws_url(base_url)?;

        let connector = if ws_url.starts_with("wss://") {
            let tls = native_tls::TlsConnector::builder()
                .danger_accept_invalid_certs(insecure_skip_verify)
                .build()
                .context("failed to build TLS connector")?;
            Some(Connector::NativeTls(tls))
        } else {
            None
        };

        let (socket, _response) =
            tokio_tungstenite::connect_async_tls_with_config(&ws_url, None, false, connector)
                .await
                .with_context(|| format!("failed to connect to {ws_url}"))?;

        let mut conn = HaConnection {
            socket,
            next_id: AtomicU64::new(1),
            pending_events: VecDeque::new(),
        };
        conn.authenticate(token).await?;
        Ok(conn)
    }

    async fn authenticate(&mut self, token: &str) -> Result<()> {
        match self.read_from_socket().await?.context("connection closed during handshake")? {
            Incoming::AuthRequired { .. } => {}
            other => bail!("expected auth_required, got {other:?}"),
        }

        self.send_raw(&Outgoing::Auth {
            access_token: token.to_string(),
        })
        .await?;

        match self.read_from_socket().await?.context("connection closed during auth")? {
            Incoming::AuthOk { .. } => Ok(()),
            Incoming::AuthInvalid { message } => {
                bail!("HA rejected the access token: {}", message.unwrap_or_default())
            }
            other => bail!("expected auth_ok/auth_invalid, got {other:?}"),
        }
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn send_raw(&mut self, msg: &Outgoing) -> Result<()> {
        let text = serde_json::to_string(msg)?;
        self.socket.send(Message::Text(text)).await?;
        Ok(())
    }

    /// Reads the next non-ping/pong/close frame as an [`Incoming`] message.
    /// Returns `Ok(None)` if the stream ended. Serves any event queued by
    /// `await_result` before reading the socket again, so events are
    /// delivered in the order they actually arrived on the wire.
    ///
    /// Only for the outer consumer (the `run()` event loop) - `await_result`
    /// must use `read_from_socket` directly, never this, or a stray event it
    /// just queued would be immediately handed straight back to it here,
    /// looping forever without ever polling the socket again.
    pub async fn read_incoming(&mut self) -> Result<Option<Incoming>> {
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(Some(event));
        }
        self.read_from_socket().await
    }

    /// Reads the next non-ping/pong/close frame straight from the socket,
    /// bypassing `pending_events`.
    async fn read_from_socket(&mut self) -> Result<Option<Incoming>> {
        loop {
            let Some(msg) = self.socket.next().await else {
                return Ok(None);
            };
            match msg? {
                Message::Text(text) => {
                    let parsed: Incoming = serde_json::from_str(&text)
                        .with_context(|| format!("failed to parse HA message: {text}"))?;
                    return Ok(Some(parsed));
                }
                Message::Close(_) => return Ok(None),
                _ => continue,
            }
        }
    }

    /// Fetches the full current state snapshot.
    pub async fn get_states(&mut self) -> Result<Vec<StateObject>> {
        let id = self.next_id();
        self.send_raw(&Outgoing::GetStates { id }).await?;
        let result = self.await_result(id).await?;
        let states: Vec<StateObject> = serde_json::from_value(result.unwrap_or(Value::Null))
            .context("failed to parse get_states result")?;
        Ok(states)
    }

    /// Fetches the area registry (rooms/areas configured in HA).
    pub async fn area_registry(&mut self) -> Result<Vec<AreaEntry>> {
        let id = self.next_id();
        self.send_raw(&Outgoing::AreaRegistryList { id }).await?;
        let result = self.await_result(id).await?;
        serde_json::from_value(result.unwrap_or(Value::Null)).context("failed to parse area_registry result")
    }

    /// Fetches the device registry (used to resolve an entity's area via
    /// its device, when the entity itself has no direct area assignment).
    pub async fn device_registry(&mut self) -> Result<Vec<DeviceEntry>> {
        let id = self.next_id();
        self.send_raw(&Outgoing::DeviceRegistryList { id }).await?;
        let result = self.await_result(id).await?;
        serde_json::from_value(result.unwrap_or(Value::Null)).context("failed to parse device_registry result")
    }

    /// Fetches the entity registry (entity -> device/area linkage).
    pub async fn entity_registry(&mut self) -> Result<Vec<EntityRegistryEntry>> {
        let id = self.next_id();
        self.send_raw(&Outgoing::EntityRegistryList { id }).await?;
        let result = self.await_result(id).await?;
        serde_json::from_value(result.unwrap_or(Value::Null)).context("failed to parse entity_registry result")
    }

    /// Subscribes to events of the given type (or all events if `None`).
    /// Returns the subscription's message id (events on this subscription
    /// arrive as `Incoming::Event` with a matching `id`).
    pub async fn subscribe_events(&mut self, event_type: Option<&str>) -> Result<u64> {
        let id = self.next_id();
        self.send_raw(&Outgoing::SubscribeEvents {
            id,
            event_type: event_type.map(str::to_string),
        })
        .await?;
        self.await_result(id).await?;
        Ok(id)
    }

    /// Calls a HA service, e.g. domain="light", service="toggle".
    pub async fn call_service(
        &mut self,
        domain: &str,
        service: &str,
        service_data: Option<Value>,
        target: Option<Value>,
    ) -> Result<()> {
        let id = self.next_id();
        self.send_raw(&Outgoing::CallService {
            id,
            domain: domain.to_string(),
            service: service.to_string(),
            service_data,
            target,
        })
        .await?;
        self.await_result(id).await?;
        Ok(())
    }

    /// Waits for the `result` message matching `id`. Any `Event` message
    /// that arrives first is queued (not dropped - see `pending_events`)
    /// for the next `read_incoming` call to deliver.
    async fn await_result(&mut self, id: u64) -> Result<Option<Value>> {
        loop {
            match self.read_from_socket().await?.context("connection closed waiting for result")? {
                Incoming::Result {
                    id: rid,
                    success,
                    result,
                    error,
                } if rid == id => {
                    if success {
                        return Ok(result);
                    }
                    let msg = error
                        .and_then(|e| e.message)
                        .unwrap_or_else(|| "unknown error".to_string());
                    bail!("HA command {id} failed: {msg}");
                }
                event @ Incoming::Event { .. } => self.pending_events.push_back(event),
                _ => continue,
            }
        }
    }
}

fn to_ws_url(base_url: &str) -> Result<String> {
    let url = base_url.trim_end_matches('/');
    let ws = if let Some(rest) = url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if url.starts_with("ws://") || url.starts_with("wss://") {
        url.to_string()
    } else {
        return Err(anyhow!("ha_url must start with http://, https://, ws:// or wss://"));
    };
    Ok(format!("{ws}/api/websocket"))
}

/// Connects with exponential backoff (1s, 2s, 4s, ... capped at 30s),
/// retrying forever until a connection + auth succeeds. Intended for the
/// long-running app; a one-shot caller (like the dump_states example)
/// should just use [`HaConnection::connect`] directly.
pub async fn connect_with_backoff(base_url: &str, token: &str, insecure_skip_verify: bool) -> HaConnection {
    let mut backoff = Duration::from_secs(1);
    loop {
        match HaConnection::connect(base_url, token, insecure_skip_verify).await {
            Ok(conn) => return conn,
            Err(err) => {
                tracing::warn!(error = %err, backoff_secs = backoff.as_secs(), "HA connection failed, retrying");
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, MAX_BACKOFF);
            }
        }
    }
}

/// Helper for callers that want a typed view of a `state_changed` event.
pub fn as_state_changed(event: &EventPayload) -> Option<super::protocol::StateChangedData> {
    if event.event_type.as_deref() != Some("state_changed") {
        return None;
    }
    let data = event.data.clone()?;
    serde_json::from_value(data).ok()
}

/// Updates the WS task pushes to the app as they happen.
#[derive(Debug)]
pub enum WsEvent {
    /// The full state + registry snapshot fetched right after each
    /// successful (re)connect. The app should replace its entire state
    /// with this, since a reconnect may have missed events.
    Snapshot {
        states: Vec<StateObject>,
        areas: Vec<AreaEntry>,
        devices: Vec<DeviceEntry>,
        entities: Vec<EntityRegistryEntry>,
    },
    StateChanged(super::protocol::StateChangedData),
    /// A `Command` the app sent (e.g. a toggle from a keypress) came back
    /// as a HA error result.
    CommandFailed { message: String },
}

/// Requests the app sends to the WS task.
#[derive(Debug)]
pub enum Command {
    CallService {
        domain: String,
        service: String,
        service_data: Option<Value>,
        target: Option<Value>,
    },
}

/// Owns the HA connection for the lifetime of the app: connects (with
/// reconnect/backoff), fetches a fresh snapshot on every (re)connect,
/// forwards `state_changed` events, and executes `Command`s the app sends
/// back (e.g. service calls from user input). Runs until `cmd_rx` is
/// dropped (the app shutting down) or `event_tx` has no more receivers.
pub async fn run(
    base_url: String,
    token: String,
    insecure_skip_verify: bool,
    event_tx: tokio::sync::mpsc::UnboundedSender<WsEvent>,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<Command>,
) {
    'reconnect: loop {
        let mut conn = connect_with_backoff(&base_url, &token, insecure_skip_verify).await;
        tracing::info!("connected to HA");

        let snapshot = async {
            let states = conn.get_states().await?;
            let areas = conn.area_registry().await?;
            let devices = conn.device_registry().await?;
            let entities = conn.entity_registry().await?;
            conn.subscribe_events(Some("state_changed")).await?;
            Ok::<_, anyhow::Error>((states, areas, devices, entities))
        };

        let (states, areas, devices, entities) = match snapshot.await {
            Ok(snapshot) => snapshot,
            Err(err) => {
                tracing::warn!(error = %err, "failed to fetch initial snapshot, reconnecting");
                continue 'reconnect;
            }
        };

        if event_tx
            .send(WsEvent::Snapshot {
                states,
                areas,
                devices,
                entities,
            })
            .is_err()
        {
            return; // app has shut down
        }

        loop {
            tokio::select! {
                incoming = conn.read_incoming() => {
                    match incoming {
                        Ok(Some(Incoming::Event { event, .. })) => {
                            if let Some(change) = as_state_changed(&event) {
                                tracing::debug!(entity_id = %change.entity_id, new_state = ?change.new_state.as_ref().map(|s| &s.state), "state_changed");
                                if event_tx.send(WsEvent::StateChanged(change)).is_err() {
                                    return;
                                }
                            }
                        }
                        Ok(Some(_)) => {}
                        Ok(None) => {
                            tracing::warn!("HA connection closed, reconnecting");
                            continue 'reconnect;
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "HA connection error, reconnecting");
                            continue 'reconnect;
                        }
                    }
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(Command::CallService { domain, service, service_data, target }) => {
                            if let Err(err) = conn.call_service(&domain, &service, service_data, target).await {
                                tracing::warn!(error = %err, domain, service, "call_service failed");
                                let _ = event_tx.send(WsEvent::CommandFailed { message: err.to_string() });
                            }
                        }
                        None => return, // app has shut down
                    }
                }
            }
        }
    }
}
