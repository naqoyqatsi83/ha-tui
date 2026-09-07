use std::io::{self, Stdout};

use anyhow::Result;
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Puts the terminal into raw mode + alternate screen + mouse capture, and
/// installs a panic hook that restores the terminal before the default
/// panic handler runs. The returned guard restores the terminal on normal
/// drop as well, so a clean `?`-propagated exit also leaves the terminal
/// usable.
pub struct TerminalGuard;

impl TerminalGuard {
    pub fn init() -> Result<(Self, Tui)> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

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
    execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
    Ok(())
}

/// Turns the terminal's mouse reporting on or off at runtime (bound to a
/// hotkey in `main`) - with it off, clicks and drags go back to the
/// terminal's own text selection instead of being captured as app input, so
/// the user can copy text out of the TUI.
pub fn set_mouse_capture(enabled: bool) -> Result<()> {
    if enabled {
        execute!(io::stdout(), EnableMouseCapture)?;
    } else {
        execute!(io::stdout(), DisableMouseCapture)?;
    }
    Ok(())
}
