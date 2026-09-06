use std::time::Duration;

use anyhow::Result;
use ha_tui::app::action::Action;
use ha_tui::app::registry::Registry;
use ha_tui::app::AppState;
use ha_tui::ha::{Command, WsEvent};
use ha_tui::{config, ha, input, logging, terminal, ui};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    let _log_guard = logging::init()?;

    let config_path = config::default_config_path()?;
    let config = config::Config::load(&config_path)?;
    tracing::info!(url = %config.ha_url, "loaded config");

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<WsEvent>();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Command>();
    let (action_tx, mut action_rx) = mpsc::unbounded_channel::<Action>();

    tokio::spawn(ha::run(
        config.ha_url.clone(),
        config.ha_token.clone(),
        config.insecure_skip_verify,
        event_tx,
        cmd_rx,
    ));
    input::spawn(action_tx);

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
                    Some(WsEvent::Snapshot { states, areas, devices, entities }) => {
                        let registry = Registry::build(areas, devices, entities);
                        match &mut app {
                            Some(app) => app.replace_snapshot(states, registry),
                            None => app = Some(AppState::new(states, registry)),
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
            action = action_rx.recv() => {
                match (&mut app, action) {
                    (_, Some(Action::Quit)) | (_, None) => break,
                    (None, _) => dirty = false, // no snapshot yet; ignore input
                    (Some(app), Some(Action::MoveUp)) => app.move_up(),
                    (Some(app), Some(Action::MoveDown)) => app.move_down(),
                    (Some(app), Some(Action::NextGroup)) => app.next_group(),
                    (Some(app), Some(Action::PrevGroup)) => app.prev_group(),
                    (Some(app), Some(Action::Toggle)) => {
                        match app.toggle_selected() {
                            Some(cmd) => { let _ = cmd_tx.send(cmd); }
                            None => dirty = false,
                        }
                    }
                    (Some(app), Some(Action::Increase)) => {
                        match app.adjust_selected(1) {
                            Some(cmd) => { let _ = cmd_tx.send(cmd); }
                            None => dirty = false,
                        }
                    }
                    (Some(app), Some(Action::Decrease)) => {
                        match app.adjust_selected(-1) {
                            Some(cmd) => { let _ = cmd_tx.send(cmd); }
                            None => dirty = false,
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
