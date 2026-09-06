pub mod action;
pub mod entity;
pub mod registry;

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use entity::Entity;
use registry::Registry;
use serde_json::json;

use crate::config::DashboardTab;
use crate::ha::{Command, StateObject};

/// One dashboard panel: an optional title and the entities it shows, in
/// order. Mirrors a Lovelace card (or, in auto-grouping mode, one domain's
/// entities within a room).
pub struct ResolvedCard<'a> {
    pub title: Option<String>,
    pub entities: Vec<&'a Entity>,
}

/// How long an optimistic UI update or a status message stays visible
/// before being cleared automatically (e.g. the service call's HA-side
/// result never arrived, or the user has had enough time to read it).
const PENDING_TIMEOUT: Duration = Duration::from_secs(5);
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a row stays flashed after its state changes.
const FLASH_DURATION: Duration = Duration::from_millis(600);
/// Rolling window length for live-updated sparkline history buffers.
const HISTORY_MAX_POINTS: usize = 60;
/// How long a tab switch's panel expand-in animation runs.
const TAB_TRANSITION_DURATION: Duration = Duration::from_millis(220);
/// Target animation frame pacing (~60fps) while a transition is running.
const ANIMATION_FRAME: Duration = Duration::from_millis(16);

/// A locally-applied guess at an entity's next display state, shown until
/// either a real `state_changed` event confirms it (cleared regardless of
/// whether the confirmed value matches - we trust HA once it answers) or
/// `PENDING_TIMEOUT` elapses (the call likely failed silently).
struct Pending {
    display_state: String,
    issued_at: Instant,
}

/// "climate" -> "Climate", "binary_sensor" -> "Binary Sensor". Used for
/// auto-generated card titles (domain names) in the dashboard-style grid.
fn titleize(domain: &str) -> String {
    domain
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Live filter/search state: `/` opens it with `editing: true` (raw key
/// presses append to `query`); Enter stops editing but keeps the filter
/// applied so normal navigation/control keys work on the narrowed list;
/// Esc clears it entirely, back to the room/domain view.
struct FilterState {
    query: String,
    editing: bool,
    selected: usize,
}

/// Holds the live entity map and the area/domain grouping derived from it,
/// plus which group/entity the UI currently has selected.
pub struct AppState {
    pub entities: HashMap<String, Entity>,
    pub registry: Registry,
    pub selected_group: usize,
    /// Which panel (card) is selected within the current tab.
    pub selected_card: usize,
    /// Which entity row is selected within `selected_card`.
    pub selected_row: usize,
    pub show_help: bool,
    /// entity_id of a graphed entity currently shown in the detail chart
    /// popup (opened via Enter on a graphed row), if any.
    detail_entity: Option<String>,
    dashboard: Vec<DashboardTab>,
    filter: Option<FilterState>,
    pending: HashMap<String, Pending>,
    status: Option<(String, Instant)>,
    graph_entity_ids: HashSet<String>,
    history: HashMap<String, VecDeque<f64>>,
    /// entity_id -> when its state last changed, for a brief highlight
    /// flash; pruned by `expire_stale` after `FLASH_DURATION`.
    flashes: HashMap<String, Instant>,
    /// When the current tab was switched to, for the panels' brief
    /// expand-in animation. `None` means no animation is (or was ever)
    /// in progress - `tab_transition_progress` treats that the same as
    /// "already finished" (fully expanded).
    tab_transition_started_at: Option<Instant>,
}

impl AppState {
    pub fn new(states: Vec<StateObject>, registry: Registry, dashboard: Vec<DashboardTab>) -> Self {
        let graph_entity_ids = dashboard
            .iter()
            .flat_map(DashboardTab::resolved_cards)
            .flat_map(|card| card.graph_entity_ids)
            .collect();

        let mut app = AppState {
            entities: HashMap::new(),
            registry,
            selected_group: 0,
            selected_card: 0,
            selected_row: 0,
            show_help: false,
            detail_entity: None,
            dashboard,
            filter: None,
            pending: HashMap::new(),
            status: None,
            graph_entity_ids,
            history: HashMap::new(),
            flashes: HashMap::new(),
            tab_transition_started_at: None,
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

        let changed = self.entities.get(&entity.entity_id).is_some_and(|old| old.state != entity.state);
        if changed {
            self.flashes.insert(entity.entity_id.clone(), Instant::now());
        }

        if self.graph_entity_ids.contains(&entity.entity_id) {
            if let Ok(value) = entity.state.parse::<f64>() {
                let buf = self.history.entry(entity.entity_id.clone()).or_default();
                buf.push_back(value);
                while buf.len() > HISTORY_MAX_POINTS {
                    buf.pop_front();
                }
            }
        }

        self.entities.insert(entity.entity_id.clone(), entity);
        self.clamp_selection();
    }

    /// Seeds (or refreshes, e.g. on reconnect) sparkline history buffers
    /// from a real `history_during_period` fetch - each entity's values
    /// replaced with the fetched trend, capped to the buffer's window.
    pub fn apply_history(&mut self, history: HashMap<String, Vec<f64>>) {
        for (entity_id, values) in history {
            let start = values.len().saturating_sub(HISTORY_MAX_POINTS);
            self.history.insert(entity_id, values[start..].iter().copied().collect());
        }
    }

    pub fn is_graphed(&self, entity_id: &str) -> bool {
        self.graph_entity_ids.contains(entity_id)
    }

    /// Recent values for `entity_id` normalized to 0-100 for
    /// `ratatui::widgets::Sparkline` (which only takes `u64`). A flat
    /// buffer (or fewer than 2 points) renders as a flat mid-height line
    /// rather than dividing by zero.
    pub fn sparkline_data(&self, entity_id: &str) -> Vec<u64> {
        let Some(buf) = self.history.get(entity_id) else {
            return Vec::new();
        };
        let min = buf.iter().copied().fold(f64::INFINITY, f64::min);
        let max = buf.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if max <= min {
            return buf.iter().map(|_| 50).collect();
        }
        buf.iter().map(|v| (((v - min) / (max - min)) * 100.0).round() as u64).collect()
    }

    /// Raw (index, value) points for `entity_id`'s history, for the
    /// detail chart - unlike `sparkline_data`, not normalized, since the
    /// chart draws its own real-valued Y axis.
    pub fn history_points(&self, entity_id: &str) -> Vec<(f64, f64)> {
        self.history
            .get(entity_id)
            .map(|buf| buf.iter().enumerate().map(|(i, v)| (i as f64, *v)).collect())
            .unwrap_or_default()
    }

    /// Whether `entity_id`'s state changed recently enough to still show
    /// the brief highlight flash.
    pub fn is_recently_changed(&self, entity_id: &str) -> bool {
        self.flashes.contains_key(entity_id)
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

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    pub fn close_help(&mut self) {
        self.show_help = false;
    }

    /// Enter on a graphed row: opens the detail chart popup for the
    /// selected entity. No-op if the selection isn't a graphed entity
    /// (the caller should fall back to toggling it instead).
    pub fn open_detail(&mut self) {
        let id = self.selected_entity().map(|e| e.entity_id.clone());
        if let Some(id) = id {
            if self.graph_entity_ids.contains(&id) {
                self.detail_entity = Some(id);
            }
        }
    }

    pub fn close_detail(&mut self) {
        self.detail_entity = None;
    }

    pub fn detail_entity(&self) -> Option<&str> {
        self.detail_entity.as_deref()
    }

    // ---- filter / search -------------------------------------------------

    pub fn is_filter_editing(&self) -> bool {
        self.filter.as_ref().is_some_and(|f| f.editing)
    }

    pub fn is_filtering(&self) -> bool {
        self.filter.is_some()
    }

    pub fn filter_query(&self) -> Option<&str> {
        self.filter.as_ref().map(|f| f.query.as_str())
    }

    /// `/`: opens the filter input, editing any already-active query.
    pub fn start_filter(&mut self) {
        let query = self.filter.as_ref().map(|f| f.query.clone()).unwrap_or_default();
        self.filter = Some(FilterState {
            query,
            editing: true,
            selected: 0,
        });
    }

    pub fn filter_push_char(&mut self, c: char) {
        if let Some(f) = &mut self.filter {
            f.query.push(c);
            f.selected = 0;
        }
    }

    pub fn filter_backspace(&mut self) {
        if let Some(f) = &mut self.filter {
            f.query.pop();
            f.selected = 0;
        }
    }

    /// Enter: stop capturing keys into the query, but keep the filter
    /// applied so movement/toggle/adjust keys act on the narrowed list.
    pub fn confirm_filter(&mut self) {
        if let Some(f) = &mut self.filter {
            f.editing = false;
        }
    }

    /// Esc: clear the filter entirely, back to the normal room/domain view.
    pub fn cancel_filter(&mut self) {
        self.filter = None;
    }

    /// All entities whose friendly name contains the current query
    /// (case-insensitive), across every group - name-sorted like a group.
    fn filtered_entities(&self) -> Vec<&Entity> {
        let query = self.filter.as_ref().map(|f| f.query.to_lowercase()).unwrap_or_default();
        let mut matches: Vec<&Entity> = self
            .entities
            .values()
            .filter(|e| e.friendly_name().to_lowercase().contains(&query))
            .collect();
        matches.sort_by(|a, b| (a.friendly_name(), &a.entity_id).cmp(&(b.friendly_name(), &b.entity_id)));
        matches
    }

    // ---- control -----------------------------------------------------

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

    /// Clears any pending optimistic update / status message / change
    /// flash that has outlived its timeout. Returns whether anything
    /// changed, so the render loop's periodic tick only redraws when it
    /// actually needs to.
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

        let before_flashes = self.flashes.len();
        self.flashes.retain(|_, at| at.elapsed() < FLASH_DURATION);
        let flashes_changed = self.flashes.len() != before_flashes;

        pending_changed || status_changed || flashes_changed
    }

    // ---- grouping / navigation -----------------------------------------

    /// Label for the left-hand tab panel: "Dashboard" when showing
    /// configured/imported tabs (manual `[[tab]]` or Lovelace import),
    /// "Rooms" when falling back to automatic room/domain grouping.
    pub fn tabs_label(&self) -> &'static str {
        if self.dashboard.is_empty() {
            "Rooms"
        } else {
            "Dashboard"
        }
    }

    /// Tabs with their cards, in display order. If the config defines
    /// explicit dashboard tabs, those are used verbatim (in their
    /// configured order, entity_ids not currently known to HA skipped) -
    /// each tab's `resolved_cards()` becomes its panels. Otherwise entities
    /// are grouped by area name where the registry knows one (falling back
    /// to domain) as the tab, alphabetically, with one card per domain
    /// within that tab. A card's own entities are always name-sorted
    /// (entity_id tie-break: `entities` is a HashMap mutated by every
    /// incoming state_changed event, so its iteration order isn't
    /// guaranteed stable between renders, which would otherwise let the
    /// index-based selection silently drift to a different entity between
    /// keypresses).
    pub fn tabs(&self) -> Vec<(String, Vec<ResolvedCard<'_>>)> {
        if !self.dashboard.is_empty() {
            return self
                .dashboard
                .iter()
                .map(|tab| {
                    let cards = tab
                        .resolved_cards()
                        .into_iter()
                        .map(|card| {
                            let entities: Vec<&Entity> =
                                card.entity_ids.iter().filter_map(|id| self.entities.get(id)).collect();
                            // A card with no label of its own and exactly one
                            // entity (e.g. a bare weather-forecast card) reads
                            // as unlabeled even though the dashboard shows
                            // one - fall back to that entity's own name.
                            let title = card.title.or_else(|| match entities.as_slice() {
                                [only] => Some(only.friendly_name().to_string()),
                                _ => None,
                            });
                            ResolvedCard { title, entities }
                        })
                        .collect();
                    (tab.name.clone(), cards)
                })
                .collect();
        }

        let mut rooms: std::collections::BTreeMap<String, Vec<&Entity>> = std::collections::BTreeMap::new();
        for entity in self.entities.values() {
            let room_name = self
                .registry
                .area_name_for(&entity.entity_id)
                .map(str::to_string)
                .unwrap_or_else(|| entity.domain.clone());
            rooms.entry(room_name).or_default().push(entity);
        }

        rooms
            .into_iter()
            .map(|(room_name, entities)| {
                let mut by_domain: std::collections::BTreeMap<String, Vec<&Entity>> = std::collections::BTreeMap::new();
                for entity in entities {
                    by_domain.entry(entity.domain.clone()).or_default().push(entity);
                }
                let cards = by_domain
                    .into_iter()
                    .map(|(domain, mut entities)| {
                        entities.sort_by(|a, b| (a.friendly_name(), &a.entity_id).cmp(&(b.friendly_name(), &b.entity_id)));
                        ResolvedCard {
                            title: Some(titleize(&domain)),
                            entities,
                        }
                    })
                    .collect();
                (room_name, cards)
            })
            .collect()
    }

    /// Entities grouped into named tabs, in display order - each tab's
    /// cards flattened into one ordered list, for navigation/selection
    /// purposes where card boundaries don't matter.
    pub fn grouped(&self) -> Vec<(String, Vec<&Entity>)> {
        self.tabs()
            .into_iter()
            .map(|(name, cards)| (name, cards.into_iter().flat_map(|c| c.entities).collect()))
            .collect()
    }

    pub fn group_names(&self) -> Vec<String> {
        self.grouped().into_iter().map(|(name, _)| name).collect()
    }

    pub fn selected_group_name(&self) -> Option<String> {
        self.group_names().into_iter().nth(self.selected_group)
    }

    /// Entities in the currently-selected group, name-sorted.
    pub fn selected_group_entities(&self) -> Vec<&Entity> {
        self.grouped()
            .into_iter()
            .nth(self.selected_group)
            .map(|(_, entities)| entities)
            .unwrap_or_default()
    }

    /// The selected tab's own cards (ignores any active filter) - the
    /// grid `selected_card`/`selected_row` navigate over.
    fn selected_tab_cards(&self) -> Vec<ResolvedCard<'_>> {
        self.tabs().into_iter().nth(self.selected_group).map(|(_, cards)| cards).unwrap_or_default()
    }

    /// The cards shown in the main panel: a single untitled "Search
    /// results" card while filtering, else the selected tab's cards.
    pub fn visible_cards(&self) -> Vec<ResolvedCard<'_>> {
        if self.filter.is_some() {
            return vec![ResolvedCard {
                title: None,
                entities: self.filtered_entities(),
            }];
        }
        self.selected_tab_cards()
    }

    /// The entity list currently shown in the main panel: the active
    /// filter's matches, or the selected group's entities.
    pub fn visible_entities(&self) -> Vec<&Entity> {
        if self.filter.is_some() {
            self.filtered_entities()
        } else {
            self.selected_group_entities()
        }
    }

    /// (card, row) into `visible_cards()` that's currently selected -
    /// always `(0, filter.selected)` while filtering, since filtering
    /// collapses to one flat card.
    pub fn selected_position(&self) -> (usize, usize) {
        match &self.filter {
            Some(f) => (0, f.selected),
            None => (self.selected_card, self.selected_row),
        }
    }

    /// The index into `visible_entities()` that's currently selected -
    /// either within the active filter's matches or the selected group.
    pub fn visible_selected_index(&self) -> usize {
        let (card, row) = self.selected_position();
        self.visible_cards().iter().take(card).map(|c| c.entities.len()).sum::<usize>() + row
    }

    pub fn selected_entity(&self) -> Option<&Entity> {
        let (card, row) = self.selected_position();
        self.visible_cards().into_iter().nth(card).and_then(|c| c.entities.into_iter().nth(row))
    }

    /// No-op while filtering (groups aren't meaningful for a global search).
    pub fn next_group(&mut self) {
        if self.filter.is_some() {
            return;
        }
        let count = self.grouped().len();
        if count <= 1 {
            return; // nothing to switch to
        }
        self.selected_group = (self.selected_group + 1) % count;
        self.selected_card = 0;
        self.selected_row = 0;
        self.tab_transition_started_at = Some(Instant::now());
    }

    pub fn prev_group(&mut self) {
        if self.filter.is_some() {
            return;
        }
        let count = self.grouped().len();
        if count <= 1 {
            return; // nothing to switch to
        }
        self.selected_group = (self.selected_group + count - 1) % count;
        self.selected_card = 0;
        self.selected_row = 0;
        self.tab_transition_started_at = Some(Instant::now());
    }

    /// 0.0 (just switched) to 1.0 (fully expanded / no transition in
    /// progress) for the current tab's panel expand-in animation.
    pub fn tab_transition_progress(&self) -> f32 {
        match self.tab_transition_started_at {
            Some(start) => {
                let elapsed = start.elapsed();
                if elapsed >= TAB_TRANSITION_DURATION {
                    1.0
                } else {
                    elapsed.as_secs_f32() / TAB_TRANSITION_DURATION.as_secs_f32()
                }
            }
            None => 1.0,
        }
    }

    /// How long the render loop should wait before the next animation
    /// frame, or `None` if nothing is animating (in which case it should
    /// just wait for the next real event instead of ticking).
    pub fn next_animation_delay(&self) -> Option<Duration> {
        match self.tab_transition_started_at {
            Some(start) if start.elapsed() < TAB_TRANSITION_DURATION => Some(ANIMATION_FRAME),
            _ => None,
        }
    }

    /// Moves within the selected panel's rows; at the top row, jumps to
    /// the bottom row of the panel directly above in the grid (same
    /// column, per `columns` - however many panels the UI is currently
    /// laying out per row, since that's a terminal-width-dependent
    /// rendering detail the app layer doesn't otherwise track).
    pub fn move_up(&mut self, columns: usize) {
        if let Some(f) = &mut self.filter {
            f.selected = f.selected.saturating_sub(1);
            return;
        }
        if self.selected_row > 0 {
            self.selected_row -= 1;
            return;
        }
        let columns = columns.max(1);
        if self.selected_card < columns {
            return; // already in the top grid row
        }
        let target = self.selected_card - columns;
        let target_len = self.selected_tab_cards().get(target).map(|c| c.entities.len());
        if let Some(len) = target_len {
            self.selected_card = target;
            self.selected_row = len.saturating_sub(1);
        }
    }

    /// Moves within the selected panel's rows; at the bottom (visible) row,
    /// jumps to the top row of the panel directly below in the grid.
    pub fn move_down(&mut self, columns: usize) {
        if self.filter.is_some() {
            let len = self.filtered_entities().len();
            if let Some(f) = &mut self.filter {
                if len > 0 {
                    f.selected = (f.selected + 1).min(len - 1);
                }
            }
            return;
        }
        let cards = self.selected_tab_cards();
        let Some(current_len) = cards.get(self.selected_card).map(|c| c.entities.len()) else {
            return;
        };
        if self.selected_row + 1 < current_len {
            self.selected_row += 1;
            return;
        }
        let target = self.selected_card + columns.max(1);
        if target < cards.len() {
            self.selected_card = target;
            self.selected_row = 0;
        }
    }

    /// Switches to the panel immediately to the left, same grid row.
    /// No-op while filtering (a search's results are a single flat card).
    pub fn move_left(&mut self, columns: usize) {
        if self.filter.is_some() || columns <= 1 {
            return;
        }
        let col = self.selected_card % columns;
        if col == 0 {
            return;
        }
        self.select_card(self.selected_card - 1);
    }

    /// Switches to the panel immediately to the right, same grid row.
    pub fn move_right(&mut self, columns: usize) {
        if self.filter.is_some() || columns <= 1 {
            return;
        }
        let card_count = self.selected_tab_cards().len();
        let col = self.selected_card % columns;
        let row_start = self.selected_card - col;
        let row_end = (row_start + columns).min(card_count);
        let target = self.selected_card + 1;
        if target < row_end {
            self.select_card(target);
        }
    }

    /// Selects `card`, keeping the same row index where it still fits.
    fn select_card(&mut self, card: usize) {
        let len = self.selected_tab_cards().get(card).map(|c| c.entities.len());
        if let Some(len) = len {
            self.selected_card = card;
            self.selected_row = self.selected_row.min(len.saturating_sub(1));
        }
    }

    /// Keeps selection in bounds after the entity map or filter changes
    /// size (an entity appearing/disappearing, a reconnect snapshot
    /// reshaping groups, a filter query narrowing further, ...). Must run
    /// after every mutation to the entity map.
    fn clamp_selection(&mut self) {
        if let Some(f) = &mut self.filter {
            let query = f.query.to_lowercase();
            let count = self
                .entities
                .values()
                .filter(|e| e.friendly_name().to_lowercase().contains(&query))
                .count();
            if count == 0 {
                f.selected = 0;
            } else if f.selected >= count {
                f.selected = count - 1;
            }
            return;
        }

        let (group, card, row) = {
            let tabs = self.tabs();
            let group_count = tabs.len();
            if group_count == 0 {
                (0, 0, 0)
            } else {
                let group = self.selected_group.min(group_count - 1);
                let cards = &tabs[group].1;
                let card_count = cards.len();
                if card_count == 0 {
                    (group, 0, 0)
                } else {
                    let card = self.selected_card.min(card_count - 1);
                    let entity_count = cards[card].entities.len();
                    let row = if entity_count == 0 { 0 } else { self.selected_row.min(entity_count - 1) };
                    (group, card, row)
                }
            }
        };
        self.selected_group = group;
        self.selected_card = card;
        self.selected_row = row;
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

    fn app(states: Vec<StateObject>, registry: Registry) -> AppState {
        AppState::new(states, registry, vec![])
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
        let a = app(
            vec![state("switch.tv_socket", "on"), state("sensor.unmapped", "42")],
            registry,
        );

        let groups: HashMap<_, _> = a.grouped().into_iter().collect();
        assert_eq!(groups.get("Living Room").map(Vec::len), Some(1));
        assert_eq!(groups.get("sensor").map(Vec::len), Some(1));
    }

    #[test]
    fn apply_state_updates_existing_entity_in_place() {
        let mut a = app(vec![state("switch.tv_socket", "off")], Registry::default());
        assert_eq!(a.entities["switch.tv_socket"].state, "off");

        a.apply_state(state("switch.tv_socket", "on"));
        assert_eq!(a.entities["switch.tv_socket"].state, "on");
        assert_eq!(a.entities.len(), 1);
    }

    #[test]
    fn remove_entity_drops_it_from_the_map() {
        let mut a = app(vec![state("switch.tv_socket", "on")], Registry::default());
        a.remove_entity("switch.tv_socket");
        assert!(a.entities.is_empty());
    }

    #[test]
    fn navigation_moves_within_and_across_groups() {
        let mut a = app(
            vec![
                state("light.a", "on"),
                state("light.b", "off"),
                state("switch.c", "on"),
            ],
            Registry::default(),
        );
        // groups (sorted): "light" (a, b), "switch" (c)
        assert_eq!(a.selected_group_name().as_deref(), Some("light"));
        assert_eq!(a.selected_entity().unwrap().entity_id, "light.a");

        a.move_down(1);
        assert_eq!(a.selected_entity().unwrap().entity_id, "light.b");

        a.move_down(1); // already at last entity in group, stays put
        assert_eq!(a.selected_entity().unwrap().entity_id, "light.b");

        a.next_group();
        assert_eq!(a.selected_group_name().as_deref(), Some("switch"));
        assert_eq!(a.selected_entity().unwrap().entity_id, "switch.c");

        a.prev_group();
        assert_eq!(a.selected_group_name().as_deref(), Some("light"));
        // selection resets to the top of the group on group change
        assert_eq!(a.selected_entity().unwrap().entity_id, "light.a");
    }

    #[test]
    fn selection_clamps_when_selected_entity_disappears() {
        let mut a = app(vec![state("light.a", "on"), state("light.b", "off")], Registry::default());
        a.move_down(1);
        assert_eq!(a.selected_row, 1);

        a.remove_entity("light.b");
        assert_eq!(a.selected_row, 0);
        assert_eq!(a.selected_entity().unwrap().entity_id, "light.a");
    }

    #[test]
    fn selection_clamps_when_all_entities_disappear() {
        let mut a = app(vec![state("light.a", "on")], Registry::default());
        a.remove_entity("light.a");
        assert_eq!(a.selected_group, 0);
        assert_eq!(a.selected_row, 0);
        assert!(a.selected_entity().is_none());
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
        let mut a = app(vec![state("light.a", "on")], Registry::default());

        let cmd = a.toggle_selected().expect("light should be toggleable");
        match cmd {
            Command::CallService { domain, service, target, .. } => {
                assert_eq!(domain, "light");
                assert_eq!(service, "toggle");
                assert_eq!(target, Some(json!({ "entity_id": "light.a" })));
            }
        }

        // Real state hasn't changed yet, but the display reflects the guess.
        let entity = a.entities["light.a"].clone();
        assert_eq!(a.display_state(&entity), "off");
        assert_eq!(a.entities["light.a"].state, "on");
    }

    #[test]
    fn toggle_selected_is_noop_for_unsupported_domain() {
        let mut a = app(vec![state("sensor.temp", "20")], Registry::default());
        assert!(a.toggle_selected().is_none());
    }

    #[test]
    fn adjust_selected_light_steps_brightness_and_clamps() {
        let mut a = app(
            vec![state_with_attrs("light.a", "on", json!({ "brightness": 240 }))],
            Registry::default(),
        );

        let cmd = a.adjust_selected(1).expect("light brightness should be adjustable");
        match cmd {
            Command::CallService { domain, service, service_data, .. } => {
                assert_eq!(domain, "light");
                assert_eq!(service, "turn_on");
                // 240 + 25 clamps to 255
                assert_eq!(service_data, Some(json!({ "brightness": 255 })));
            }
        }
        let entity = a.entities["light.a"].clone();
        assert_eq!(a.display_state(&entity), "on (brightness 255/255)");
    }

    #[test]
    fn adjust_selected_climate_steps_target_temperature() {
        let mut a = app(
            vec![state_with_attrs("climate.a", "heat", json!({ "temperature": 21.0 }))],
            Registry::default(),
        );

        let cmd = a.adjust_selected(-1).expect("climate target temp should be adjustable");
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
        let mut a = app(vec![state("switch.a", "off")], Registry::default());
        a.toggle_selected();
        let entity = a.entities["switch.a"].clone();
        assert_eq!(a.display_state(&entity), "on"); // optimistic

        // HA confirms - even though it happens to match the guess here,
        // reconciliation clears the pending entry unconditionally.
        a.apply_state(state("switch.a", "on"));
        let entity = a.entities["switch.a"].clone();
        assert_eq!(a.display_state(&entity), "on");
        assert!(a.pending.is_empty());
    }

    #[test]
    fn command_failed_sets_a_status_message() {
        let mut a = app(vec![state("switch.a", "on")], Registry::default());
        assert!(a.status_message().is_none());
        a.command_failed("entity not found");
        assert_eq!(a.status_message(), Some("error: entity not found"));
    }

    #[test]
    fn dashboard_tabs_override_auto_grouping_and_keep_configured_order() {
        let a = AppState::new(
            vec![state("light.a", "on"), state("switch.b", "off"), state("sensor.c", "1")],
            Registry::default(),
            vec![
                DashboardTab::new("Second", vec!["switch.b".into()]),
                DashboardTab::new("First", vec!["light.a".into(), "sensor.c".into()]),
            ],
        );

        let groups = a.grouped();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "Second");
        assert_eq!(groups[0].1.iter().map(|e| &e.entity_id).collect::<Vec<_>>(), vec!["switch.b"]);
        assert_eq!(groups[1].0, "First");
        assert_eq!(
            groups[1].1.iter().map(|e| &e.entity_id).collect::<Vec<_>>(),
            vec!["light.a", "sensor.c"]
        );
    }

    #[test]
    fn dashboard_tab_skips_entity_ids_ha_does_not_currently_know_about() {
        let a = AppState::new(
            vec![state("light.a", "on")],
            Registry::default(),
            vec![DashboardTab::new("Tab", vec!["light.a".into(), "light.missing".into()])],
        );
        let groups = a.grouped();
        assert_eq!(groups[0].1.len(), 1);
        assert_eq!(groups[0].1[0].entity_id, "light.a");
    }

    #[test]
    fn filter_narrows_across_all_groups_by_friendly_name() {
        let mut a = app(
            vec![
                state("light.kitchen_lamp", "on"),
                state("switch.kitchen_socket", "off"),
                state("sensor.bedroom_temp", "20"),
            ],
            Registry::default(),
        );
        a.start_filter();
        assert!(a.is_filter_editing());
        for c in "kitchen".chars() {
            a.filter_push_char(c);
        }
        let mut ids: Vec<_> = a.visible_entities().into_iter().map(|e| e.entity_id.clone()).collect();
        ids.sort();
        assert_eq!(ids, vec!["light.kitchen_lamp", "switch.kitchen_socket"]);

        a.confirm_filter();
        assert!(!a.is_filter_editing());
        assert!(a.is_filtering());
    }

    #[test]
    fn filter_backspace_and_cancel() {
        let mut a = app(vec![state("light.kitchen_lamp", "on")], Registry::default());
        a.start_filter();
        a.filter_push_char('x');
        a.filter_push_char('y');
        a.filter_backspace();
        assert_eq!(a.filter_query(), Some("x"));

        a.cancel_filter();
        assert!(!a.is_filtering());
        assert_eq!(a.filter_query(), None);
    }

    #[test]
    fn filter_selection_clamps_as_query_narrows() {
        let mut a = app(
            vec![state("light.a_lamp", "on"), state("light.b_lamp", "on")],
            Registry::default(),
        );
        a.start_filter();
        for c in "lamp".chars() {
            a.filter_push_char(c);
        }
        a.move_down(1);
        assert_eq!(a.selected_entity().unwrap().entity_id, "light.b_lamp");

        a.filter_push_char('x'); // no entity matches "lampx"
        assert!(a.selected_entity().is_none());
    }

    #[test]
    fn auto_grouping_splits_a_room_into_one_card_per_domain() {
        let registry = Registry::build(
            vec![AreaEntry {
                area_id: "living_room".into(),
                name: "Living Room".into(),
            }],
            vec![],
            vec![
                EntityRegistryEntry {
                    entity_id: "light.a".into(),
                    device_id: None,
                    area_id: Some("living_room".into()),
                },
                EntityRegistryEntry {
                    entity_id: "switch.b".into(),
                    device_id: None,
                    area_id: Some("living_room".into()),
                },
            ],
        );
        let a = app(vec![state("light.a", "on"), state("switch.b", "off")], registry);

        let tabs = a.tabs();
        let (name, cards) = tabs.iter().find(|(n, _)| n == "Living Room").unwrap();
        assert_eq!(name, "Living Room");
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].title.as_deref(), Some("Light"));
        assert_eq!(cards[0].entities[0].entity_id, "light.a");
        assert_eq!(cards[1].title.as_deref(), Some("Switch"));
        assert_eq!(cards[1].entities[0].entity_id, "switch.b");
    }

    fn grid_of_four_single_entity_cards() -> AppState {
        let mut tab = DashboardTab::new("Grid", vec![]);
        tab.cards = ["a", "b", "c", "d"]
            .iter()
            .map(|n| crate::config::DashboardCard {
                title: Some((*n).to_string()),
                entity_ids: vec![format!("light.{n}")],
                ..Default::default()
            })
            .collect();
        AppState::new(
            vec![
                state("light.a", "on"),
                state("light.b", "on"),
                state("light.c", "on"),
                state("light.d", "on"),
            ],
            Registry::default(),
            vec![tab],
        )
    }

    #[test]
    fn move_right_and_left_switch_between_panels_in_the_same_grid_row() {
        // 2x2 grid: [a b] / [c d]
        let mut a = grid_of_four_single_entity_cards();
        assert_eq!(a.selected_entity().unwrap().entity_id, "light.a");

        a.move_right(2);
        assert_eq!(a.selected_entity().unwrap().entity_id, "light.b");

        // already rightmost in its grid row - no-op
        a.move_right(2);
        assert_eq!(a.selected_entity().unwrap().entity_id, "light.b");

        a.move_left(2);
        assert_eq!(a.selected_entity().unwrap().entity_id, "light.a");

        // already leftmost - no-op
        a.move_left(2);
        assert_eq!(a.selected_entity().unwrap().entity_id, "light.a");
    }

    #[test]
    fn move_down_and_up_cross_grid_rows_via_the_same_column() {
        // 2x2 grid: [a b] / [c d]
        let mut a = grid_of_four_single_entity_cards();
        a.move_right(2); // -> b (row 0, col 1)
        a.move_down(2); // -> d (row 1, col 1): same column, one entity each so it jumps straight to the next panel
        assert_eq!(a.selected_entity().unwrap().entity_id, "light.d");

        a.move_up(2); // back up to b
        assert_eq!(a.selected_entity().unwrap().entity_id, "light.b");

        // top row - no panel above, no-op
        a.move_up(2);
        assert_eq!(a.selected_entity().unwrap().entity_id, "light.b");
    }

    #[test]
    fn left_right_are_noops_with_a_single_column() {
        let mut a = grid_of_four_single_entity_cards();
        a.move_right(1);
        assert_eq!(a.selected_entity().unwrap().entity_id, "light.a");
        a.move_left(1);
        assert_eq!(a.selected_entity().unwrap().entity_id, "light.a");
    }

    #[test]
    fn dashboard_cards_carry_titles_through_to_visible_cards() {
        let mut tab = DashboardTab::new("Living Room", vec![]);
        tab.cards = vec![
            crate::config::DashboardCard {
                title: Some("Lights".into()),
                entity_ids: vec!["light.a".into()],
                ..Default::default()
            },
            crate::config::DashboardCard {
                title: None,
                entity_ids: vec!["light.b".into(), "sensor.c".into()],
                ..Default::default()
            },
        ];
        let a = AppState::new(
            vec![state("light.a", "on"), state("light.b", "on"), state("sensor.c", "1")],
            Registry::default(),
            vec![tab],
        );

        let cards = a.visible_cards();
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].title.as_deref(), Some("Lights"));
        assert_eq!(cards[0].entities[0].entity_id, "light.a");
        // Untitled card with more than one entity stays untitled - no
        // single entity to reasonably fall back to.
        assert_eq!(cards[1].title, None);
    }

    #[test]
    fn untitled_single_entity_card_falls_back_to_the_entitys_own_name() {
        let mut tab = DashboardTab::new("Weather", vec![]);
        tab.cards = vec![crate::config::DashboardCard {
            title: None,
            entity_ids: vec!["weather.home".into()],
            ..Default::default()
        }];
        let a = AppState::new(
            vec![state_with_attrs("weather.home", "sunny", json!({ "friendly_name": "Forecast Home" }))],
            Registry::default(),
            vec![tab],
        );

        let cards = a.visible_cards();
        assert_eq!(cards[0].title.as_deref(), Some("Forecast Home"));
    }

    #[test]
    fn filtering_collapses_to_a_single_untitled_card() {
        let mut a = app(vec![state("light.kitchen_lamp", "on"), state("light.other", "on")], Registry::default());
        a.start_filter();
        for c in "kitchen".chars() {
            a.filter_push_char(c);
        }
        let cards = a.visible_cards();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].title, None);
        assert_eq!(cards[0].entities.len(), 1);
        assert_eq!(cards[0].entities[0].entity_id, "light.kitchen_lamp");
    }

    #[test]
    fn help_toggles() {
        let mut a = app(vec![], Registry::default());
        assert!(!a.show_help);
        a.toggle_help();
        assert!(a.show_help);
        a.close_help();
        assert!(!a.show_help);
    }

    #[test]
    fn open_detail_opens_only_for_a_graphed_selection() {
        let mut tab = DashboardTab::new("Home", vec![]);
        tab.cards = vec![crate::config::DashboardCard {
            title: None,
            entity_ids: vec!["sensor.temp".into()],
            graph_entity_ids: vec!["sensor.temp".into()],
        }];
        let mut a = AppState::new(vec![state("sensor.temp", "20")], Registry::default(), vec![tab]);

        assert_eq!(a.detail_entity(), None);
        a.open_detail();
        assert_eq!(a.detail_entity(), Some("sensor.temp"));
        a.close_detail();
        assert_eq!(a.detail_entity(), None);
    }

    #[test]
    fn open_detail_is_a_noop_for_a_non_graphed_selection() {
        let mut a = app(vec![state("light.a", "on")], Registry::default());
        a.open_detail();
        assert_eq!(a.detail_entity(), None);
    }

    #[test]
    fn history_points_are_index_value_pairs_in_order() {
        let mut a = app_with_graphed_sensor();
        let mut history = HashMap::new();
        history.insert("sensor.temp".to_string(), vec![10.0, 20.0, 30.0]);
        a.apply_history(history);

        assert_eq!(a.history_points("sensor.temp"), vec![(0.0, 10.0), (1.0, 20.0), (2.0, 30.0)]);
        assert_eq!(a.history_points("sensor.unknown"), vec![]);
    }

    #[test]
    fn tab_switch_starts_at_zero_progress_and_animation_delay() {
        let mut a = app(
            vec![state("light.a", "on"), state("switch.b", "on")],
            Registry::default(),
        );
        // No transition has happened yet - fully "expanded".
        assert_eq!(a.tab_transition_progress(), 1.0);
        assert_eq!(a.next_animation_delay(), None);

        a.next_group();
        // Real elapsed time since Instant::now(), so a tiny epsilon rather
        // than exactly 0.0 - still well short of "in progress" (< 1.0) and
        // still animating.
        assert!(a.tab_transition_progress() < 0.1);
        assert_eq!(a.next_animation_delay(), Some(Duration::from_millis(16)));
    }

    #[test]
    fn no_tabs_to_switch_between_does_not_start_an_animation() {
        let mut a = app(vec![state("light.a", "on")], Registry::default());
        a.next_group(); // only one group - no-op
        assert_eq!(a.tab_transition_progress(), 1.0);
        assert_eq!(a.next_animation_delay(), None);
    }

    fn app_with_graphed_sensor() -> AppState {
        let mut tab = DashboardTab::new("Home", vec![]);
        tab.cards = vec![crate::config::DashboardCard {
            title: None,
            entity_ids: vec!["sensor.temp".into()],
            graph_entity_ids: vec!["sensor.temp".into()],
        }];
        AppState::new(vec![state("sensor.temp", "20")], Registry::default(), vec![tab])
    }

    #[test]
    fn is_graphed_reflects_the_dashboard_cards_graph_entity_ids() {
        let a = app_with_graphed_sensor();
        assert!(a.is_graphed("sensor.temp"));
        assert!(!a.is_graphed("sensor.other"));
    }

    #[test]
    fn apply_history_seeds_the_sparkline_buffer() {
        let mut a = app_with_graphed_sensor();
        let mut history = HashMap::new();
        history.insert("sensor.temp".to_string(), vec![10.0, 20.0, 30.0]);
        a.apply_history(history);

        let data = a.sparkline_data("sensor.temp");
        assert_eq!(data, vec![0, 50, 100]); // normalized 10..30 to 0..100
    }

    #[test]
    fn sparkline_data_is_empty_for_an_ungraphed_or_unseeded_entity() {
        let a = app_with_graphed_sensor();
        assert!(a.sparkline_data("sensor.temp").is_empty()); // no history applied yet
        assert!(a.sparkline_data("sensor.other").is_empty());
    }

    #[test]
    fn live_state_changes_append_to_the_graphed_entitys_history() {
        let mut a = app_with_graphed_sensor();
        a.apply_state(state("sensor.temp", "25"));
        a.apply_state(state("sensor.temp", "30"));
        // Both numeric updates appended; normalized last point is the max.
        assert_eq!(a.sparkline_data("sensor.temp"), vec![0, 100]);
    }

    #[test]
    fn state_change_flashes_the_entity_briefly() {
        let mut a = app(vec![state("switch.a", "off")], Registry::default());
        assert!(!a.is_recently_changed("switch.a"));

        a.apply_state(state("switch.a", "on"));
        assert!(a.is_recently_changed("switch.a"));

        // Applying the same state again isn't a change, so no new flash.
        a.apply_state(state("switch.a", "on"));
        assert!(a.is_recently_changed("switch.a"));
    }
}
