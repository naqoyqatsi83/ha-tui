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
    let (_cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Command>();
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

    loop {
        match &app {
            Some(app) => {
                tui.draw(|frame| ui::draw(frame, app))?;
            }
            None => {
                tui.draw(ui::draw_connecting)?;
            }
        }

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
                    None => break, // WS task ended (shouldn't happen; it retries forever)
                }
            }
            action = action_rx.recv() => {
                match action {
                    Some(Action::Quit) | None => break,
                    Some(Action::MoveUp) => {
                        if let Some(app) = &mut app { app.move_up(); }
                    }
                    Some(Action::MoveDown) => {
                        if let Some(app) = &mut app { app.move_down(); }
                    }
                    Some(Action::NextGroup) => {
                        if let Some(app) = &mut app { app.next_group(); }
                    }
                    Some(Action::PrevGroup) => {
                        if let Some(app) = &mut app { app.prev_group(); }
                    }
                }
            }
        }
    }

    Ok(())
}
