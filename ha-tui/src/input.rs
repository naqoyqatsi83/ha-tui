use crossterm::event::{self, Event};
use tokio::sync::mpsc::UnboundedSender;

use crate::app::action::Action;

/// Spawns a dedicated OS thread that blocks on `crossterm::event::read()`
/// and forwards mapped key events as `Action`s. A thread (not a tokio
/// task) because `event::read()` is a blocking syscall with no async
/// variant available without pulling in the `event-stream` feature.
pub fn spawn(tx: UnboundedSender<Action>) {
    std::thread::spawn(move || loop {
        match event::read() {
            Ok(Event::Key(key)) => {
                if let Some(action) = Action::from_key(key) {
                    let is_quit = action == Action::Quit;
                    if tx.send(action).is_err() || is_quit {
                        return;
                    }
                }
            }
            Ok(_) => {}
            Err(_) => return,
        }
    });
}
