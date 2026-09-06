use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

/// User-input intents for normal (non-filter-editing) mode, decoupled from
/// the raw crossterm key so the render loop's `match` doesn't need to know
/// about keybindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    MoveUp,
    MoveDown,
    /// Switch to the panel to the left, same grid row.
    MoveLeft,
    /// Switch to the panel to the right, same grid row.
    MoveRight,
    NextGroup,
    PrevGroup,
    /// Enter/Space on a light or switch row: call its `toggle` service.
    Toggle,
    /// `+`: light -> brightness up a step, climate -> target temp up a step.
    Increase,
    /// `-`: light -> brightness down a step, climate -> target temp down a step.
    Decrease,
    /// `/`: open the filter/search input.
    StartFilter,
    /// Esc: clear an active (confirmed) filter, if any.
    ClearFilter,
    /// `?`: toggle the help overlay.
    ShowHelp,
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
            KeyCode::Left | KeyCode::Char('h') => Some(Action::MoveLeft),
            KeyCode::Right | KeyCode::Char('l') => Some(Action::MoveRight),
            KeyCode::Tab => Some(Action::NextGroup),
            KeyCode::BackTab => Some(Action::PrevGroup),
            KeyCode::Enter | KeyCode::Char(' ') => Some(Action::Toggle),
            KeyCode::Char('+') | KeyCode::Char('=') => Some(Action::Increase),
            KeyCode::Char('-') => Some(Action::Decrease),
            KeyCode::Char('/') => Some(Action::StartFilter),
            KeyCode::Esc => Some(Action::ClearFilter),
            KeyCode::Char('?') => Some(Action::ShowHelp),
            _ => None,
        }
    }
}

/// Key interpretation while the filter input has focus (after `/`, before
/// Enter/Esc) - every printable character is query text here, not a
/// keybinding, so this is deliberately a separate mapping from [`Action`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterAction {
    Push(char),
    Backspace,
    /// Enter: stop editing, keep the filter applied.
    Confirm,
    /// Esc: clear the filter entirely.
    Cancel,
}

impl FilterAction {
    pub fn from_key(key: KeyEvent) -> Option<FilterAction> {
        if key.kind != KeyEventKind::Press {
            return None;
        }
        match key.code {
            KeyCode::Char(c) => Some(FilterAction::Push(c)),
            KeyCode::Backspace => Some(FilterAction::Backspace),
            KeyCode::Enter => Some(FilterAction::Confirm),
            KeyCode::Esc => Some(FilterAction::Cancel),
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
        assert_eq!(Action::from_key(press(KeyCode::Char('h'))), Some(Action::MoveLeft));
        assert_eq!(Action::from_key(press(KeyCode::Char('l'))), Some(Action::MoveRight));
        assert_eq!(Action::from_key(press(KeyCode::Left)), Some(Action::MoveLeft));
        assert_eq!(Action::from_key(press(KeyCode::Right)), Some(Action::MoveRight));
        assert_eq!(Action::from_key(press(KeyCode::Tab)), Some(Action::NextGroup));
        assert_eq!(Action::from_key(press(KeyCode::BackTab)), Some(Action::PrevGroup));
        assert_eq!(Action::from_key(press(KeyCode::Enter)), Some(Action::Toggle));
        assert_eq!(Action::from_key(press(KeyCode::Char(' '))), Some(Action::Toggle));
        assert_eq!(Action::from_key(press(KeyCode::Char('+'))), Some(Action::Increase));
        assert_eq!(Action::from_key(press(KeyCode::Char('='))), Some(Action::Increase));
        assert_eq!(Action::from_key(press(KeyCode::Char('-'))), Some(Action::Decrease));
        assert_eq!(Action::from_key(press(KeyCode::Char('/'))), Some(Action::StartFilter));
        assert_eq!(Action::from_key(press(KeyCode::Esc)), Some(Action::ClearFilter));
        assert_eq!(Action::from_key(press(KeyCode::Char('?'))), Some(Action::ShowHelp));
    }

    #[test]
    fn ignores_unknown_keys_and_non_press_events() {
        assert_eq!(Action::from_key(press(KeyCode::Char('x'))), None);
        let mut release = press(KeyCode::Char('q'));
        release.kind = KeyEventKind::Release;
        assert_eq!(Action::from_key(release), None);
    }

    #[test]
    fn filter_action_maps_printable_chars_and_controls() {
        assert_eq!(FilterAction::from_key(press(KeyCode::Char('a'))), Some(FilterAction::Push('a')));
        assert_eq!(FilterAction::from_key(press(KeyCode::Char(' '))), Some(FilterAction::Push(' ')));
        assert_eq!(FilterAction::from_key(press(KeyCode::Backspace)), Some(FilterAction::Backspace));
        assert_eq!(FilterAction::from_key(press(KeyCode::Enter)), Some(FilterAction::Confirm));
        assert_eq!(FilterAction::from_key(press(KeyCode::Esc)), Some(FilterAction::Cancel));
    }
}
