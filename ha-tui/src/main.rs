use std::time::Duration;

use anyhow::Result;
use crossterm::event::{MouseButton, MouseEventKind};
use ha_tui::app::action::{Action, FilterAction};
use ha_tui::app::registry::Registry;
use ha_tui::app::AppState;
use ha_tui::ha::{Command, WsEvent};
use ha_tui::input::InputEvent;
use ha_tui::{config, ha, input, logging, terminal, ui};
use ratatui::layout::Position;
use tokio::sync::mpsc;

/// Enter/Space on a row, or a mouse click landing on one (after `select`
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
    // Screen positions of the entity rows drawn in the most recent frame,
    // for mapping a mouse click back to a (card, row) selection - stale
    // between draws, but a click always lands after the frame it's
    // clicking on, so it's always in sync with what's actually on screen.
    let mut hits: Vec<ui::RowHit> = Vec::new();
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
                let Some(app) = &mut app else {
                    // No snapshot yet (still on the "Connecting..." screen)
                    // - still quittable via keyboard, everything else
                    // (including all mouse activity) is a no-op.
                    if matches!(event, InputEvent::Key(key) if Action::from_key(key) == Some(Action::Quit)) {
                        break;
                    }
                    continue;
                };

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
                        if app.show_help {
                            // Any click (or scroll) dismisses the help
                            // overlay, mirroring "any key" for keyboard.
                            app.close_help();
                        } else if app.detail_entity().is_some() {
                            app.close_detail();
                        } else if app.is_filter_editing() {
                            // The filter text box has no mouse affordances.
                            dirty = false;
                        } else {
                            let columns = ui::cards::columns_for(tui.size().map(|s| s.width).unwrap_or(80), app.visible_cards().len());
                            match mouse.kind {
                                MouseEventKind::ScrollUp => app.move_up(columns),
                                MouseEventKind::ScrollDown => app.move_down(columns),
                                MouseEventKind::Down(MouseButton::Left) => {
                                    let pos = Position { x: mouse.column, y: mouse.row };
                                    match hits.iter().find(|hit| hit.area.contains(pos)) {
                                        Some(hit) => {
                                            app.select(hit.card, hit.row);
                                            dirty = activate_selected(app, &cmd_tx);
                                        }
                                        None => dirty = false,
                                    }
                                }
                                _ => dirty = false,
                            }
                        }
                    }
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
                    hits.clear();
                    tui.draw(ui::draw_connecting)?;
                }
            }
        }
    }

    Ok(())
}
