//! Key -> [`Action`] mapping.
//!
//! Pure and mode-aware: in text-entry mode keys feed the line buffer;
//! otherwise they drive navigation and editing. Keeping this separate
//! from [`crate::app`] lets the whole keymap be unit-tested without a
//! terminal.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A semantic action produced from a key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Nothing bound to this key.
    None,
    // Navigation.
    PrevRow,
    NextRow,
    PrevComponent,
    NextComponent,
    // Editing the focused component.
    Increment,
    Decrement,
    // Row / mode toggles.
    ToggleExpand,
    ToggleSubField,
    ToggleMode,
    CopyPrevious,
    Distribute,
    ResetRow,
    // Text entry.
    BeginEdit,
    Char(char),
    Backspace,
    CommitEdit,
    CancelEdit,
    // Global.
    ToggleHelp,
    Write,
    Quit,
}

/// Map a key to an action. `editing` selects the text-entry keymap.
pub fn map(key: KeyEvent, editing: bool) -> Action {
    if editing {
        return match key.code {
            KeyCode::Char(c) => Action::Char(c),
            KeyCode::Backspace => Action::Backspace,
            KeyCode::Enter => Action::CommitEdit,
            KeyCode::Esc => Action::CancelEdit,
            _ => Action::None,
        };
    }

    // Ctrl-C always quits.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Action::Quit;
    }

    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Up if shift => Action::Increment,
        KeyCode::Down if shift => Action::Decrement,
        KeyCode::Up | KeyCode::Char('k') => Action::PrevRow,
        KeyCode::Down | KeyCode::Char('j') => Action::NextRow,
        KeyCode::Left | KeyCode::Char('h') => Action::PrevComponent,
        KeyCode::Right | KeyCode::Char('l') => Action::NextComponent,
        KeyCode::Char('+') | KeyCode::Char('K') => Action::Increment,
        KeyCode::Char('-') | KeyCode::Char('J') => Action::Decrement,
        KeyCode::Tab => Action::ToggleExpand,
        KeyCode::Char('t') => Action::ToggleSubField,
        KeyCode::Char('s') => Action::ToggleMode,
        KeyCode::Char('c') => Action::CopyPrevious,
        KeyCode::Char('=') => Action::Distribute,
        KeyCode::Char('d') => Action::ResetRow,
        KeyCode::Char('e') | KeyCode::Enter => Action::BeginEdit,
        KeyCode::Char('?') => Action::ToggleHelp,
        KeyCode::Char('w') => Action::Write,
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn key_mod(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    #[test]
    fn navigation_keys() {
        assert_eq!(map(key(KeyCode::Up), false), Action::PrevRow);
        assert_eq!(map(key(KeyCode::Char('j')), false), Action::NextRow);
        assert_eq!(map(key(KeyCode::Left), false), Action::PrevComponent);
        assert_eq!(map(key(KeyCode::Char('l')), false), Action::NextComponent);
    }

    #[test]
    fn increment_variants() {
        assert_eq!(map(key(KeyCode::Char('+')), false), Action::Increment);
        assert_eq!(map(key(KeyCode::Char('-')), false), Action::Decrement);
        assert_eq!(
            map(key_mod(KeyCode::Up, KeyModifiers::SHIFT), false),
            Action::Increment
        );
        assert_eq!(
            map(key_mod(KeyCode::Down, KeyModifiers::SHIFT), false),
            Action::Decrement
        );
    }

    #[test]
    fn toggles_and_globals() {
        assert_eq!(map(key(KeyCode::Tab), false), Action::ToggleExpand);
        assert_eq!(map(key(KeyCode::Char('t')), false), Action::ToggleSubField);
        assert_eq!(map(key(KeyCode::Char('s')), false), Action::ToggleMode);
        assert_eq!(map(key(KeyCode::Char('=')), false), Action::Distribute);
        assert_eq!(map(key(KeyCode::Char('w')), false), Action::Write);
        assert_eq!(map(key(KeyCode::Char('q')), false), Action::Quit);
        assert_eq!(map(key(KeyCode::Esc), false), Action::Quit);
    }

    #[test]
    fn ctrl_c_quits() {
        assert_eq!(
            map(key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL), false),
            Action::Quit
        );
    }

    #[test]
    fn text_mode_feeds_the_buffer() {
        assert_eq!(map(key(KeyCode::Char('2')), true), Action::Char('2'));
        assert_eq!(map(key(KeyCode::Backspace), true), Action::Backspace);
        assert_eq!(map(key(KeyCode::Enter), true), Action::CommitEdit);
        assert_eq!(map(key(KeyCode::Esc), true), Action::CancelEdit);
    }

    #[test]
    fn c_is_copy_only_outside_text_mode() {
        // Plain 'c' copies; Ctrl-C quits; in text mode 'c' is a char.
        assert_eq!(map(key(KeyCode::Char('c')), false), Action::CopyPrevious);
        assert_eq!(map(key(KeyCode::Char('c')), true), Action::Char('c'));
    }
}
