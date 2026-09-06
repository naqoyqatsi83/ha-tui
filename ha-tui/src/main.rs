mod config;
mod logging;
mod terminal;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use ratatui::widgets::{Block, Borders, Paragraph};

#[tokio::main]
async fn main() -> Result<()> {
    let _log_guard = logging::init()?;

    let config_path = config::default_config_path()?;
    let config = config::Config::load(&config_path)?;
    tracing::info!(url = %config.ha_url, "loaded config");

    let (_guard, mut tui) = terminal::TerminalGuard::init()?;

    loop {
        tui.draw(|frame| {
            let block = Paragraph::new("ha-tui — press 'q' to quit")
                .block(Block::default().borders(Borders::ALL).title("ha-tui"));
            frame.render_widget(block, frame.area());
        })?;

        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    break;
                }
            }
        }
    }

    Ok(())
}
