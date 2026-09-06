use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

/// User-input intents, decoupled from the raw crossterm key so the render
/// loop's `match` doesn't need to know about keybindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    MoveUp,
    MoveDown,
    NextGroup,
    PrevGroup,
    /// Enter/Space on a light or switch row: call its `toggle` service.
    Toggle,
    /// `+`: light -> brightness up a step, climate -> target temp up a step.
    Increase,
    /// `-`: light -> brightness down a step, climate -> target temp down a step.
    Decrease,
}

impl Action {
    pub fn from_key(key: KeyEvent) -> Option<Action> {
        // Some terminals report both press and release; only act on press
        // to avoid double-firing.
        if key.kind != KeyEventKind::Press {
            return None;
        }
        match key.code {
            KeyCode::Char('q') => Some(Action::Quit),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::MoveUp),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::MoveDown),
            KeyCode::Tab => Some(Action::NextGroup),
            KeyCode::BackTab => Some(Action::PrevGroup),
            KeyCode::Enter | KeyCode::Char(' ') => Some(Action::Toggle),
            KeyCode::Char('+') | KeyCode::Char('=') => Some(Action::Increase),
            KeyCode::Char('-') => Some(Action::Decrease),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn maps_known_keys() {
        assert_eq!(Action::from_key(press(KeyCode::Char('q'))), Some(Action::Quit));
        assert_eq!(Action::from_key(press(KeyCode::Char('j'))), Some(Action::MoveDown));
        assert_eq!(Action::from_key(press(KeyCode::Char('k'))), Some(Action::MoveUp));
        assert_eq!(Action::from_key(press(KeyCode::Down)), Some(Action::MoveDown));
        assert_eq!(Action::from_key(press(KeyCode::Up)), Some(Action::MoveUp));
        assert_eq!(Action::from_key(press(KeyCode::Tab)), Some(Action::NextGroup));
        assert_eq!(Action::from_key(press(KeyCode::BackTab)), Some(Action::PrevGroup));
        assert_eq!(Action::from_key(press(KeyCode::Enter)), Some(Action::Toggle));
        assert_eq!(Action::from_key(press(KeyCode::Char(' '))), Some(Action::Toggle));
        assert_eq!(Action::from_key(press(KeyCode::Char('+'))), Some(Action::Increase));
        assert_eq!(Action::from_key(press(KeyCode::Char('='))), Some(Action::Increase));
        assert_eq!(Action::from_key(press(KeyCode::Char('-'))), Some(Action::Decrease));
    }

    #[test]
    fn ignores_unknown_keys_and_non_press_events() {
        assert_eq!(Action::from_key(press(KeyCode::Char('x'))), None);
        let mut release = press(KeyCode::Char('q'));
        release.kind = KeyEventKind::Release;
        assert_eq!(Action::from_key(release), None);
    }
}
