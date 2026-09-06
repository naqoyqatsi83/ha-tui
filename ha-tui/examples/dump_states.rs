//! Phase 1 smoke test: connects to HA, prints the initial state snapshot,
//! then prints each `state_changed` event as it arrives.
//!
//! Reads config the same way the main app does:
//! `~/.config/ha-tui/config.toml`.

use anyhow::Result;
use ha_tui::config::{default_config_path, Config};
use ha_tui::ha::{as_state_changed, HaConnection, Incoming};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_writer(std::io::stderr).init();

    let config_path = default_config_path()?;
    let config = Config::load(&config_path)?;

    let mut conn = HaConnection::connect(&config.ha_url, &config.ha_token, config.insecure_skip_verify).await?;
    println!("connected and authenticated to {}", config.ha_url);

    let states = conn.get_states().await?;
    println!("--- initial state snapshot ({} entities) ---", states.len());
    for state in &states {
        println!("{:<40} {}", state.entity_id, state.state);
    }

    conn.subscribe_events(Some("state_changed")).await?;
    println!("--- subscribed, watching for state_changed events (Ctrl+C to stop) ---");

    loop {
        match conn.read_incoming().await? {
            Some(Incoming::Event { event, .. }) => {
                if let Some(change) = as_state_changed(&event) {
                    let old = change.old_state.as_ref().map(|s| s.state.as_str()).unwrap_or("-");
                    let new = change.new_state.as_ref().map(|s| s.state.as_str()).unwrap_or("-");
                    println!("{:<40} {old} -> {new}", change.entity_id);
                }
            }
            Some(_) => continue,
            None => {
                println!("connection closed");
                break;
            }
        }
    }

    Ok(())
}
