use std::io::{self, Stdout};

use anyhow::Result;
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Puts the terminal into raw mode + alternate screen, and installs a panic
/// hook that restores the terminal before the default panic handler runs.
/// The returned guard restores the terminal on normal drop as well, so a
/// clean `?`-propagated exit also leaves the terminal usable.
pub struct TerminalGuard;

impl TerminalGuard {
    pub fn init() -> Result<(Self, Tui)> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;

        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = restore_terminal();
            default_hook(info);
        }));

        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok((TerminalGuard, terminal))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = restore_terminal();
    }
}

fn restore_terminal() -> Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}
