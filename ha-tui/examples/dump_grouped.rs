//! Phase 2 smoke test: fetches states + registries from HA and prints
//! entities grouped by room/domain, to sanity-check the grouping logic
//! against a real instance.

use anyhow::Result;
use ha_tui::app::registry::Registry;
use ha_tui::app::AppState;
use ha_tui::config::{default_config_path, Config};
use ha_tui::ha::HaConnection;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::load(&default_config_path()?)?;
    let mut conn = HaConnection::connect(&config.ha_url, &config.ha_token, config.insecure_skip_verify).await?;

    let states = conn.get_states().await?;
    let areas = conn.area_registry().await?;
    let devices = conn.device_registry().await?;
    let entities = conn.entity_registry().await?;

    let registry = Registry::build(areas, devices, entities);
    let app = AppState::new(states, registry, vec![]);

    for (group, entities) in app.grouped() {
        println!("== {group} ({}) ==", entities.len());
        for entity in entities.iter().take(3) {
            println!("  {:<40} {}", entity.friendly_name(), entity.state);
        }
    }

    Ok(())
}
