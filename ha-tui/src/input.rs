use crossterm::event::{self, Event, KeyEvent, MouseEvent};
use tokio::sync::mpsc::UnboundedSender;

/// A raw terminal input event, narrowed to the two kinds the render loop
/// acts on (key presses and mouse activity) - everything else `event::read`
/// can report (resize, focus, paste) is dropped at the source.
#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
}

/// Spawns a dedicated OS thread that blocks on `crossterm::event::read()`
/// and forwards key and mouse events. A thread (not a tokio task) because
/// `event::read()` is a blocking syscall with no async variant available
/// without pulling in the `event-stream` feature.
///
/// Events are forwarded raw (not pre-interpreted into an `Action`) because
/// interpretation depends on the app's current mode (normal navigation vs.
/// filter-input editing vs. the help overlay swallowing the next
/// key/click), which only the render loop knows.
pub fn spawn(tx: UnboundedSender<InputEvent>) {
    std::thread::spawn(move || loop {
        match event::read() {
            Ok(Event::Key(key)) => {
                if tx.send(InputEvent::Key(key)).is_err() {
                    return;
                }
            }
            Ok(Event::Mouse(mouse)) => {
                if tx.send(InputEvent::Mouse(mouse)).is_err() {
                    return;
                }
            }
            Ok(_) => {}
            Err(_) => return,
        }
    });
}
