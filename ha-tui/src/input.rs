use crossterm::event::{self, Event, KeyEvent};
use tokio::sync::mpsc::UnboundedSender;

/// Spawns a dedicated OS thread that blocks on `crossterm::event::read()`
/// and forwards raw key events. A thread (not a tokio task) because
/// `event::read()` is a blocking syscall with no async variant available
/// without pulling in the `event-stream` feature.
///
/// Keys are forwarded raw (not pre-interpreted into an `Action`) because
/// interpretation depends on the app's current mode (normal navigation vs.
/// filter-input editing vs. the help overlay swallowing the next key),
/// which only the render loop knows.
pub fn spawn(tx: UnboundedSender<KeyEvent>) {
    std::thread::spawn(move || loop {
        match event::read() {
            Ok(Event::Key(key)) => {
                if tx.send(key).is_err() {
                    return;
                }
            }
            Ok(_) => {}
            Err(_) => return,
        }
    });
}
