use std::time::Duration;

use anyhow::Result;
use crossterm::event::KeyEvent;
use ha_tui::app::action::{Action, FilterAction};
use ha_tui::app::registry::Registry;
use ha_tui::app::AppState;
use ha_tui::ha::{Command, WsEvent};
use ha_tui::{config, ha, input, logging, terminal, ui};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    let _log_guard = logging::init()?;

    let config_path = config::resolve_config_path(std::env::args().skip(1))?;
    let config = config::Config::load(&config_path)?;
    tracing::info!(url = %config.ha_url, "loaded config");

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<WsEvent>();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Command>();
    let (key_tx, mut key_rx) = mpsc::unbounded_channel::<KeyEvent>();

    tokio::spawn(ha::run(
        config.ha_url.clone(),
        config.ha_token.clone(),
        config.insecure_skip_verify,
        config.import_lovelace,
        event_tx,
        cmd_rx,
    ));
    input::spawn(key_tx);

    let (_guard, mut tui) = terminal::TerminalGuard::init()?;

    let mut app: Option<AppState> = None;
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
                    Some(WsEvent::Snapshot { states, areas, devices, entities, lovelace_tabs }) => {
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
            key = key_rx.recv() => {
                let Some(key) = key else { break };
                let Some(app) = &mut app else {
                    continue; // no snapshot yet; ignore input, nothing to redraw
                };

                if app.show_help {
                    // Any key dismisses the help overlay.
                    app.close_help();
                } else if app.is_filter_editing() {
                    match FilterAction::from_key(key) {
                        Some(FilterAction::Push(c)) => app.filter_push_char(c),
                        Some(FilterAction::Backspace) => app.filter_backspace(),
                        Some(FilterAction::Confirm) => app.confirm_filter(),
                        Some(FilterAction::Cancel) => app.cancel_filter(),
                        None => dirty = false,
                    }
                } else {
                    match Action::from_key(key) {
                        Some(Action::Quit) => break,
                        Some(Action::MoveUp) => app.move_up(),
                        Some(Action::MoveDown) => app.move_down(),
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
                        Some(Action::Toggle) => match app.toggle_selected() {
                            Some(cmd) => { let _ = cmd_tx.send(cmd); }
                            None => dirty = false,
                        },
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
            _ = expiry_tick.tick() => {
                dirty = match &mut app {
                    Some(app) => app.expire_stale(),
                    None => false,
                };
            }
        }

        if dirty {
            match &app {
                Some(app) => {
                    tui.draw(|frame| ui::draw(frame, app))?;
                }
                None => {
                    tui.draw(ui::draw_connecting)?;
                }
            }
        }
    }

    Ok(())
}
