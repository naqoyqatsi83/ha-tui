# Project Plan: `ha-tui` — A Rust TUI Client for Home Assistant

## Goal
Build a terminal UI application that connects to a Home Assistant instance over its WebSocket API, displays entities grouped by room/domain, updates in real time as states change, and lets the user control common entities (lights, switches, climate) with keyboard shortcuts.

## Tech stack
- **Runtime**: `tokio`
- **TUI**: `ratatui` + `crossterm`
- **HA connection**: `tokio-tungstenite` (raw WebSocket) — build our own thin client rather than depending on `hass-rs`, so we control reconnect/backoff and message shapes precisely
- **Serialization**: `serde`, `serde_json`
- **Config**: `serde` + `toml`, config file at `~/.config/ha-tui/config.toml`
- **Error handling**: `anyhow` for app-level, `thiserror` for library-level errors
- **Logging**: `tracing` + `tracing-subscriber`, logging to a file (never stdout, since stdout is the TUI)

## Architecture overview
Three concurrent pieces communicating over `tokio::sync::mpsc` / `watch` channels:
1. **WebSocket task** — owns the connection to HA, authenticates, subscribes to `state_changed` events, sends `call_service` commands, handles reconnect with exponential backoff.
2. **Input task** — polls `crossterm` events (keyboard) and forwards them as app-level `Action`s.
3. **Render loop (main task)** — owns application state (`HashMap<EntityId, Entity>`), applies incoming state-change events, applies user actions (which may also send commands back to the WS task), and redraws only when something changed.

Data flow: `WS task --state updates--> App state <--actions-- Input task`, and `App --commands--> WS task` for calling services.

## Milestones

### Phase 0 — Project scaffolding
- `cargo new ha-tui`
- Add dependencies, set up `tracing` to log to `~/.local/share/ha-tui/app.log`
- Config loading: HA base URL, long-lived access token, optional path overrides
- Basic `main.rs` that sets up the terminal (raw mode, alternate screen) and tears it down cleanly on exit/panic (use a panic hook + `Drop` guard so a crash doesn't leave the terminal broken)

**Definition of done**: `cargo run` opens a blank ratatui screen, `q` quits cleanly, config file is read and validated with a clear error if the token/URL is missing.

### Phase 1 — HA WebSocket client (library module, e.g. `src/ha/mod.rs`)
- Implement the HA WS auth handshake:
  1. Connect to `ws://<host>/api/websocket`
  2. Receive `auth_required`
  3. Send `{"type": "auth", "access_token": "..."}`
  4. Receive `auth_ok` or `auth_invalid`
- Implement message ID counter (HA requires incrementing `id` per request)
- Implement `subscribe_events` (type `state_changed`)
- Implement `get_states` (initial full snapshot on connect)
- Implement `call_service` (domain, service, entity_id, optional service_data)
- Define typed `HaEvent` / `HaMessage` enums via `serde` with `#[serde(tag = "type")]` where applicable
- Reconnect logic: on disconnect, backoff (1s, 2s, 4s... capped), re-auth, re-subscribe, re-fetch full state snapshot (don't assume you didn't miss events)

**Definition of done**: a small standalone example/bin (`examples/dump_states.rs`) connects, prints the initial state list, then prints each state_changed event as it happens.

### Phase 2 — Application state & domain modeling
- `Entity` struct: `entity_id`, `state: String`, `attributes: serde_json::Value`, `last_updated`
- Thin typed views over raw attributes for domains you actually render specially:
  - `light` (on/off, brightness, color_temp)
  - `switch` (on/off)
  - `climate` (current_temperature, target temp, hvac_mode)
  - `sensor` (value + unit_of_measurement)
  - fallback: render `entity_id` + raw `state` for anything else
- Grouping: by `area`/room if available (needs `config/area_registry` and `config/entity_registry` WS commands — fetch these once at startup and cache), otherwise fall back to grouping by domain
- `AppState` struct holding the entity map + registries + currently selected entity/tab

**Definition of done**: unit tests that deserialize sample HA JSON payloads (captured from Phase 1's dump tool) into `Entity` and the typed domain views correctly.

### Phase 3 — Basic UI (read-only)
- Layout: left-side tab list (rooms or domains), main panel: scrollable list of entities in the selected group, each row showing name + current state
- Keybindings: arrow keys / `j`/`k` to move selection, `Tab`/`Shift+Tab` to switch groups, `q` to quit
- Redraw-on-change: use a `watch` channel or dirty flag so the render loop doesn't spin — only redraw on new WS event or input event (use `tokio::select!` with a tick fallback for clock/relative-time displays if needed)

**Definition of done**: launching the app shows live entity states from your real HA instance, updating within ~1s of a change made elsewhere (e.g. via the HA app).

### Phase 4 — Control (write actions)
- `Enter`/`Space` on a `light`/`switch` row toggles it (`call_service` light.toggle / switch.toggle)
- For `light`, a secondary key (e.g. `+`/`-`) adjusts brightness by a step
- For `climate`, `+`/`-` adjusts target temperature
- Optimistic UI update on keypress, reconciled against the real state_changed event when it arrives (don't just trust the optimistic value forever — clear it once the real update lands, and time it out after ~5s in case the call failed)
- Visible error/status line for failed service calls (e.g. HA returned an error result for that command id)

**Definition of done**: toggling a real light/switch from the TUI actually flips it, and the UI reflects both the optimistic state and the confirmed state correctly.

### Phase 5 — Polish
- Search/filter entities by name (`/` to start filtering, like `less`/`fzf`)
- Config for a custom dashboard layout (explicit list of entity_ids per named tab, overriding the auto room-grouping)
- Color coding by state (on = highlighted, unavailable = dimmed/red)
- Help overlay (`?`) listing keybindings
- Graceful handling of `unavailable`/`unknown` states
- `--config <path>` CLI flag, sensible default config path

### Phase 6 (optional, later) — Nice-to-haves
- Notifications/toasts for HA `persistent_notification` entities
- History/graph view for `sensor` entities (sparkline via ratatui's `Sparkline` widget) using HA's `history/history_during_period` WS command
- Mouse support (ratatui/crossterm support mouse events — click to toggle)
- Packaging: `cargo install --path .`, maybe a `.deb`/AUR package later

## Suggested file structure
```
ha-tui/
  Cargo.toml
  src/
    main.rs          # terminal setup/teardown, event loop wiring
    config.rs         # config file loading + validation
    ha/
      mod.rs          # public client API (connect, subscribe, call_service)
      protocol.rs      # serde types for HA WS messages
      client.rs        # the actual WS task + reconnect logic
    app/
      mod.rs           # AppState, update logic
      entity.rs         # Entity + typed domain views
      registry.rs       # area/entity registry cache
    ui/
      mod.rs           # top-level draw() dispatch
      list.rs           # entity list widget
      tabs.rs           # room/domain tab bar
      help.rs           # help overlay
  examples/
    dump_states.rs     # Phase 1 smoke-test tool
```

## Notes for Claude Code when executing this plan
- Work phase by phase; get each phase's "definition of done" working and committed before moving to the next.
- Use a real (or test) Home Assistant instance for manual verification — a long-lived access token needs to be created by the person under HA's user profile settings; ask for the HA URL and confirm a token is available before Phase 1 rather than mocking indefinitely.
- Keep the WS protocol types in `protocol.rs` permissive (`#[serde(other)]` fallback variants, `Option` fields) — HA's payloads vary by integration and version, and the app should not panic on an unexpected shape.
- Never block the render loop on network I/O — all HA calls happen in the WS task, communicated via channels.
- Write the panic/terminal-restore hook early (Phase 0) so debugging later phases doesn't leave the terminal in a broken state after a crash.
