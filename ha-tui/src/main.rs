use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use ha_tui::app::action::{Action, FilterAction};
use ha_tui::app::registry::Registry;
use ha_tui::app::AppState;
use ha_tui::ha::{Command, WsEvent};
use ha_tui::input::InputEvent;
use ha_tui::{config, ha, input, logging, terminal, ui};
use ratatui::layout::Position;
use tokio::sync::mpsc;

/// Two left-clicks on the same row within this long of each other count
/// as a double-click. Standard desktop double-click intervals run roughly
/// 300-500ms; 400ms splits the difference.
const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(400);

/// Enter/Space on a row, or a double-click landing on one (after `select`
/// has already moved there): opens the detail chart for a graphed entity,
/// otherwise toggles it. Shared so keyboard and mouse activation can never
/// disagree about what "activating" a row does. Returns whether anything
/// actually happened, for the caller's dirty-redraw tracking.
fn activate_selected(app: &mut AppState, cmd_tx: &mpsc::UnboundedSender<Command>) -> bool {
    let graphed = app.selected_entity().is_some_and(|e| app.is_graphed(&e.entity_id));
    if graphed {
        app.open_detail();
        true
    } else {
        match app.toggle_selected() {
            Some(cmd) => {
                let _ = cmd_tx.send(cmd);
                true
            }
            None => false,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let _log_guard = logging::init()?;

    let config_path = config::resolve_config_path(std::env::args().skip(1))?;
    let config = config::Config::load(&config_path)?;
    tracing::info!(url = %config.ha_url, "loaded config");

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<WsEvent>();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Command>();
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<InputEvent>();

    tokio::spawn(ha::run(
        config.ha_url.clone(),
        config.ha_token.clone(),
        config.insecure_skip_verify,
        config.import_lovelace,
        event_tx,
        cmd_rx,
    ));
    input::spawn(input_tx);

    let (_guard, mut tui) = terminal::TerminalGuard::init()?;

    let mut app: Option<AppState> = None;
    // Screen positions of the tabs/entity rows drawn in the most recent
    // frame, for mapping a mouse click back to a tab index or (card, row)
    // selection - stale between draws, but a click always lands after the
    // frame it's clicking on, so it's always in sync with what's actually
    // on screen.
    let mut hits = ui::DrawHits::default();
    // (when, card, row) of the last left-click that landed on a row, for
    // double-click detection - a second click on the *same row* (not
    // necessarily the same cell within it) within `DOUBLE_CLICK_WINDOW`
    // activates it instead of just selecting it.
    let mut last_row_click: Option<(Instant, usize, usize)> = None;
    // F2 toggles this off so the terminal's own text selection/copy works
    // again - while on, the terminal hands every click to us instead.
    let mut mouse_enabled = true;
    // Only fires often enough to expire stale optimistic updates / status
    // messages (both on a 5s timeout) - not a general redraw tick.
    let mut expiry_tick = tokio::time::interval(Duration::from_millis(500));

    tui.draw(ui::draw_connecting)?;

    loop {
        // Whether this iteration's event actually changed anything visible
        // - only then do we pay for a redraw.
        let mut dirty = true;

        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Some(WsEvent::Snapshot { states, areas, devices, entities, lovelace_tabs, history }) => {
                        let registry = Registry::build(areas, devices, entities);
                        // Manual [[tab]] config wins if present; otherwise use
                        // the imported Lovelace tabs when there are any;
                        // otherwise AppState falls back to auto grouping.
                        let dashboard = if !config.dashboard.is_empty() {
                            config.dashboard.clone()
                        } else {
                            lovelace_tabs
                        };
                        match &mut app {
                            Some(app) => app.replace_snapshot(states, registry),
                            None => app = Some(AppState::new(states, registry, dashboard)),
                        }
                        if let Some(app) = &mut app {
                            app.apply_history(history);
                        }
                    }
                    Some(WsEvent::StateChanged(change)) => {
                        if let Some(app) = &mut app {
                            match change.new_state {
                                Some(state) => app.apply_state(state),
                                None => app.remove_entity(&change.entity_id),
                            }
                        }
                    }
                    Some(WsEvent::CommandFailed { message }) => {
                        if let Some(app) = &mut app {
                            app.command_failed(&message);
                        }
                    }
                    None => break, // WS task ended (shouldn't happen; it retries forever)
                }
            }
            event = input_rx.recv() => {
                let Some(event) = event else { break };

                // Works in any mode (help/detail/filter/connecting) and
                // regardless of what's selected - it's a terminal-level
                // mode switch, not an app action, so it's handled before
                // the "no snapshot yet" early-out below.
                let is_mouse_toggle =
                    matches!(event, InputEvent::Key(key) if key.kind == KeyEventKind::Press && key.code == KeyCode::F(2));

                if is_mouse_toggle {
                    mouse_enabled = !mouse_enabled;
                    if let Err(err) = terminal::set_mouse_capture(mouse_enabled) {
                        tracing::warn!(%err, "failed to toggle mouse capture");
                    }
                    match &mut app {
                        Some(app) => app.set_status(if mouse_enabled {
                            "Mouse mode on (F2 to turn off for terminal text selection/copy)"
                        } else {
                            "Mouse mode off - terminal text selection/copy enabled (F2 to turn back on)"
                        }),
                        None => dirty = false, // nothing drawn yet to show the status on
                    }
                } else if let Some(app) = &mut app {
                    match event {
                        InputEvent::Key(key) => {
                            if app.show_help {
                                // Any key dismisses the help overlay.
                                app.close_help();
                            } else if app.detail_entity().is_some() {
                                // Any key dismisses the detail chart popup.
                                app.close_detail();
                            } else if app.is_filter_editing() {
                                match FilterAction::from_key(key) {
                                    Some(FilterAction::Push(c)) => app.filter_push_char(c),
                                    Some(FilterAction::Backspace) => app.filter_backspace(),
                                    Some(FilterAction::Confirm) => app.confirm_filter(),
                                    Some(FilterAction::Cancel) => app.cancel_filter(),
                                    None => dirty = false,
                                }
                            } else {
                                // Column count must match what the card grid
                                // actually rendered (a terminal-width-dependent
                                // layout detail AppState doesn't otherwise
                                // track) so left/right and the up/down
                                // panel-jump land on the right neighbor.
                                let columns = ui::cards::columns_for(tui.size().map(|s| s.width).unwrap_or(80), app.visible_cards().len());
                                match Action::from_key(key) {
                                    Some(Action::Quit) => break,
                                    Some(Action::MoveUp) => app.move_up(columns),
                                    Some(Action::MoveDown) => app.move_down(columns),
                                    Some(Action::MoveLeft) => app.move_left(columns),
                                    Some(Action::MoveRight) => app.move_right(columns),
                                    Some(Action::NextGroup) => app.next_group(),
                                    Some(Action::PrevGroup) => app.prev_group(),
                                    Some(Action::StartFilter) => app.start_filter(),
                                    Some(Action::ClearFilter) => {
                                        if app.is_filtering() {
                                            app.cancel_filter();
                                        } else {
                                            dirty = false;
                                        }
                                    }
                                    Some(Action::ShowHelp) => app.toggle_help(),
                                    Some(Action::Toggle) => {
                                        dirty = activate_selected(app, &cmd_tx);
                                    }
                                    Some(Action::Increase) => match app.adjust_selected(1) {
                                        Some(cmd) => { let _ = cmd_tx.send(cmd); }
                                        None => dirty = false,
                                    },
                                    Some(Action::Decrease) => match app.adjust_selected(-1) {
                                        Some(cmd) => { let _ = cmd_tx.send(cmd); }
                                        None => dirty = false,
                                    },
                                    None => dirty = false,
                                }
                            }
                        }
                        InputEvent::Mouse(mouse) => {
                            // Crossterm also reports drag/move/release as
                            // distinct `MouseEventKind`s; only a fresh left
                            // press and scroll ticks are meaningful here.
                            // Treating every kind as "any input" (like the key
                            // handler's overlay-dismiss does) would mean the
                            // *release* of the very click that just opened the
                            // detail popup immediately closes it again.
                            let is_click = matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left));
                            let is_scroll = matches!(mouse.kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown);

                            if !is_click && !is_scroll {
                                dirty = false;
                            } else if app.show_help {
                                app.close_help();
                            } else if app.detail_entity().is_some() {
                                app.close_detail();
                            } else if app.is_filter_editing() {
                                // The filter text box has no mouse affordances.
                                dirty = false;
                            } else if is_scroll {
                                let columns = ui::cards::columns_for(tui.size().map(|s| s.width).unwrap_or(80), app.visible_cards().len());
                                match mouse.kind {
                                    MouseEventKind::ScrollUp => app.move_up(columns),
                                    MouseEventKind::ScrollDown => app.move_down(columns),
                                    _ => unreachable!("is_scroll only matches these two kinds"),
                                }
                            } else {
                                let pos = Position { x: mouse.column, y: mouse.row };
                                if let Some(tab) = hits.tabs.iter().find(|t| t.area.contains(pos)) {
                                    // Dashboard/room switching: a single click
                                    // acts immediately, no double-click needed.
                                    app.select_group(tab.index);
                                } else if let Some(hit) = hits.rows.iter().find(|h| h.area.contains(pos)) {
                                    app.select(hit.card, hit.row);
                                    let is_double_click = last_row_click
                                        .is_some_and(|(at, card, row)| card == hit.card && row == hit.row && at.elapsed() < DOUBLE_CLICK_WINDOW);
                                    if is_double_click {
                                        dirty = activate_selected(app, &cmd_tx);
                                        last_row_click = None;
                                    } else {
                                        last_row_click = Some((Instant::now(), hit.card, hit.row));
                                    }
                                } else {
                                    dirty = false;
                                }
                            }
                        }
                    }
                } else {
                    // No snapshot yet (still on the "Connecting..." screen)
                    // - still quittable via keyboard, everything else
                    // (including all mouse activity) is a no-op.
                    if matches!(event, InputEvent::Key(key) if Action::from_key(key) == Some(Action::Quit)) {
                        break;
                    }
                    dirty = false;
                }
            }
            _ = expiry_tick.tick() => {
                dirty = match &mut app {
                    Some(app) => app.expire_stale(),
                    None => false,
                };
            }
            // Paces the tab-switch expand animation (~60fps) while one is
            // running; resolves and stays pending forever otherwise, so an
            // idle app never wakes up for this on its own.
            _ = async {
                match app.as_ref().and_then(AppState::next_animation_delay) {
                    Some(delay) => tokio::time::sleep(delay).await,
                    None => std::future::pending().await,
                }
            } => {
                dirty = true;
            }
        }

        if dirty {
            match &app {
                Some(app) => {
                    tui.draw(|frame| hits = ui::draw(frame, app))?;
                }
                None => {
                    hits = ui::DrawHits::default();
                    tui.draw(ui::draw_connecting)?;
                }
            }
        }
    }

    Ok(())
}
