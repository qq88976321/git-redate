//! Key -> [`Action`] mapping.
//!
//! Pure and context-aware: text entry feeds the line buffer, a
//! confirmation prompt takes yes/no, and navigation drives editing.
//! Keeping this separate from [`crate::app`] lets the whole keymap be
//! unit-tested without a terminal. Bindings follow common TUI
//! conventions (lazygit/gitui/tig/k9s/vim): `j`/`k` + arrows to move,
//! `h`/`l` + arrows to pick a field, `+`/`-` and `Shift+arrows` (and
//! vim `Ctrl-A`/`Ctrl-X`) to adjust, `u` to reset (not `d`, which reads
//! as delete), `Space` to disclose, `Tab` to move between sub-fields.

use crate::lineedit::LineOp;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A semantic action produced from a key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
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
    ResetAll,
    // Text entry.
    BeginEdit,
    Char(char),
    Backspace,
    CommitEdit,
    CancelEdit,
    // Search (incremental jump).
    BeginSearch,
    Line(LineOp),
    CommitSearch,
    CancelSearch,
    NextMatch,
    PrevMatch,
    // Global.
    Undo,
    Redo,
    ToggleHelp,
    Write,
    WriteForce,
    Quit,
    QuitForce,
    // Confirmation prompt.
    ConfirmYes,
    ConfirmNo,
}

/// Which keymap applies, mirroring the app's mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Navigate,
    Editing,
    Confirm,
    Search,
}

fn is_ctrl(key: &KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(c)
}

/// Map a key to an action for the given context.
pub fn map(key: KeyEvent, ctx: Context) -> Action {
    // Ctrl-C is a hard abort in every context.
    if is_ctrl(&key, 'c') {
        return Action::QuitForce;
    }
    match ctx {
        Context::Editing => map_editing(key),
        Context::Confirm => map_confirm(key),
        Context::Search => map_search(key),
        Context::Navigate => map_navigate(key),
    }
}

fn map_editing(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char(c) => Action::Char(c),
        KeyCode::Backspace => Action::Backspace,
        KeyCode::Enter => Action::CommitEdit,
        KeyCode::Esc => Action::CancelEdit,
        _ => Action::None,
    }
}

fn map_confirm(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => Action::ConfirmYes,
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('q') | KeyCode::Esc => {
            Action::ConfirmNo
        }
        _ => Action::None,
    }
}

/// The search prompt: a readline-style line editor plus commit/cancel.
/// (Ctrl-C is caught by `map` before this, so it still hard-aborts.)
fn map_search(key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let line = |op| Action::Line(op);
    match key.code {
        KeyCode::Enter => Action::CommitSearch,
        KeyCode::Esc => Action::CancelSearch,
        KeyCode::Backspace => line(LineOp::Backspace),
        KeyCode::Delete => line(LineOp::Delete),
        KeyCode::Home => line(LineOp::Home),
        KeyCode::End => line(LineOp::End),
        KeyCode::Left if ctrl => line(LineOp::WordLeft),
        KeyCode::Right if ctrl => line(LineOp::WordRight),
        KeyCode::Left => line(LineOp::Left),
        KeyCode::Right => line(LineOp::Right),
        // Emacs/readline control keys for editing the query.
        KeyCode::Char(c) if ctrl => match c {
            'a' => line(LineOp::Home),
            'e' => line(LineOp::End),
            'b' => line(LineOp::Left),
            'f' => line(LineOp::Right),
            'w' => line(LineOp::KillWordBack),
            'u' => line(LineOp::KillToStart),
            'k' => line(LineOp::KillToEnd),
            _ => Action::None,
        },
        KeyCode::Char(c) => line(LineOp::Insert(c)),
        _ => Action::None,
    }
}

fn map_navigate(key: KeyEvent) -> Action {
    // vim-style numeric nudge.
    if is_ctrl(&key, 'a') {
        return Action::Increment;
    }
    if is_ctrl(&key, 'x') {
        return Action::Decrement;
    }
    if is_ctrl(&key, 'z') {
        return Action::Undo;
    }
    if is_ctrl(&key, 'r') {
        return Action::Redo;
    }

    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Up if shift => Action::Increment,
        KeyCode::Down if shift => Action::Decrement,
        KeyCode::Up | KeyCode::Char('k') => Action::PrevRow,
        KeyCode::Down | KeyCode::Char('j') => Action::NextRow,
        KeyCode::Left | KeyCode::Char('h') => Action::PrevComponent,
        KeyCode::Right | KeyCode::Char('l') => Action::NextComponent,
        KeyCode::Char('+') => Action::Increment,
        KeyCode::Char('-') => Action::Decrement,
        KeyCode::Char(' ') => Action::ToggleExpand,
        KeyCode::Tab | KeyCode::BackTab => Action::ToggleSubField,
        KeyCode::Char('s') => Action::ToggleMode,
        KeyCode::Char('c') => Action::CopyPrevious,
        KeyCode::Char('=') => Action::Distribute,
        KeyCode::Char('u') => Action::ResetRow,
        KeyCode::Char('U') => Action::ResetAll,
        KeyCode::Char('/') => Action::BeginSearch,
        KeyCode::Char('n') => Action::NextMatch,
        KeyCode::Char('N') => Action::PrevMatch,
        KeyCode::Char('e') | KeyCode::Enter => Action::BeginEdit,
        KeyCode::Char('?') | KeyCode::F(1) => Action::ToggleHelp,
        KeyCode::Char('w') => Action::Write,
        KeyCode::Char('W') => Action::WriteForce,
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
        KeyCode::Char('Q') => Action::QuitForce,
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
    fn nav(code: KeyCode) -> Action {
        map(key(code), Context::Navigate)
    }

    #[test]
    fn navigation_keys() {
        assert_eq!(nav(KeyCode::Up), Action::PrevRow);
        assert_eq!(nav(KeyCode::Char('j')), Action::NextRow);
        assert_eq!(nav(KeyCode::Left), Action::PrevComponent);
        assert_eq!(nav(KeyCode::Char('l')), Action::NextComponent);
    }

    #[test]
    fn adjust_variants_without_kj() {
        assert_eq!(nav(KeyCode::Char('+')), Action::Increment);
        assert_eq!(nav(KeyCode::Char('-')), Action::Decrement);
        assert_eq!(
            map(key_mod(KeyCode::Up, KeyModifiers::SHIFT), Context::Navigate),
            Action::Increment
        );
        assert_eq!(
            map(
                key_mod(KeyCode::Down, KeyModifiers::SHIFT),
                Context::Navigate
            ),
            Action::Decrement
        );
        assert_eq!(
            map(
                key_mod(KeyCode::Char('a'), KeyModifiers::CONTROL),
                Context::Navigate
            ),
            Action::Increment
        );
        assert_eq!(
            map(
                key_mod(KeyCode::Char('x'), KeyModifiers::CONTROL),
                Context::Navigate
            ),
            Action::Decrement
        );
        // K/J are no longer bound to adjust.
        assert_ne!(nav(KeyCode::Char('K')), Action::Increment);
        assert_ne!(nav(KeyCode::Char('J')), Action::Decrement);
    }

    #[test]
    fn disclosure_and_subfield() {
        assert_eq!(nav(KeyCode::Char(' ')), Action::ToggleExpand);
        assert_eq!(nav(KeyCode::Tab), Action::ToggleSubField);
        assert_eq!(nav(KeyCode::BackTab), Action::ToggleSubField);
    }

    #[test]
    fn reset_is_u_not_d() {
        assert_eq!(nav(KeyCode::Char('u')), Action::ResetRow);
        assert_eq!(nav(KeyCode::Char('d')), Action::None); // d is unbound
    }

    #[test]
    fn reset_all_is_capital_u() {
        assert_eq!(nav(KeyCode::Char('U')), Action::ResetAll);
    }

    #[test]
    fn undo_redo_are_ctrl_z_and_ctrl_r() {
        assert_eq!(
            map(
                key_mod(KeyCode::Char('z'), KeyModifiers::CONTROL),
                Context::Navigate
            ),
            Action::Undo
        );
        assert_eq!(
            map(
                key_mod(KeyCode::Char('r'), KeyModifiers::CONTROL),
                Context::Navigate
            ),
            Action::Redo
        );
    }

    #[test]
    fn write_and_quit_split() {
        assert_eq!(nav(KeyCode::Char('w')), Action::Write);
        assert_eq!(nav(KeyCode::Char('W')), Action::WriteForce);
        assert_eq!(nav(KeyCode::Char('q')), Action::Quit);
        assert_eq!(nav(KeyCode::Char('Q')), Action::QuitForce);
        assert_eq!(nav(KeyCode::Esc), Action::Quit);
        assert_eq!(
            map(
                key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
                Context::Navigate
            ),
            Action::QuitForce
        );
    }

    #[test]
    fn help_aliases() {
        assert_eq!(nav(KeyCode::Char('?')), Action::ToggleHelp);
        assert_eq!(nav(KeyCode::F(1)), Action::ToggleHelp);
    }

    #[test]
    fn editing_context_feeds_buffer() {
        assert_eq!(
            map(key(KeyCode::Char('2')), Context::Editing),
            Action::Char('2')
        );
        assert_eq!(
            map(key(KeyCode::Backspace), Context::Editing),
            Action::Backspace
        );
        assert_eq!(
            map(key(KeyCode::Enter), Context::Editing),
            Action::CommitEdit
        );
        assert_eq!(map(key(KeyCode::Esc), Context::Editing), Action::CancelEdit);
        // 'q' in text entry is a literal character, not quit.
        assert_eq!(
            map(key(KeyCode::Char('q')), Context::Editing),
            Action::Char('q')
        );
    }

    #[test]
    fn navigate_search_bindings() {
        assert_eq!(nav(KeyCode::Char('/')), Action::BeginSearch);
        assert_eq!(nav(KeyCode::Char('n')), Action::NextMatch);
        assert_eq!(nav(KeyCode::Char('N')), Action::PrevMatch);
    }

    #[test]
    fn search_context_edits_and_commits() {
        let s = |code| map(key(code), Context::Search);
        assert_eq!(s(KeyCode::Char('a')), Action::Line(LineOp::Insert('a')));
        assert_eq!(s(KeyCode::Backspace), Action::Line(LineOp::Backspace));
        assert_eq!(s(KeyCode::Delete), Action::Line(LineOp::Delete));
        assert_eq!(s(KeyCode::Left), Action::Line(LineOp::Left));
        assert_eq!(s(KeyCode::Home), Action::Line(LineOp::Home));
        assert_eq!(s(KeyCode::End), Action::Line(LineOp::End));
        assert_eq!(s(KeyCode::Enter), Action::CommitSearch);
        assert_eq!(s(KeyCode::Esc), Action::CancelSearch);
    }

    #[test]
    fn search_context_readline_control_keys() {
        let ctrl = |c| {
            map(
                key_mod(KeyCode::Char(c), KeyModifiers::CONTROL),
                Context::Search,
            )
        };
        assert_eq!(ctrl('u'), Action::Line(LineOp::KillToStart));
        assert_eq!(ctrl('w'), Action::Line(LineOp::KillWordBack));
        assert_eq!(ctrl('k'), Action::Line(LineOp::KillToEnd));
        assert_eq!(ctrl('a'), Action::Line(LineOp::Home));
        assert_eq!(ctrl('e'), Action::Line(LineOp::End));
        assert_eq!(
            map(
                key_mod(KeyCode::Left, KeyModifiers::CONTROL),
                Context::Search
            ),
            Action::Line(LineOp::WordLeft)
        );
        assert_eq!(
            map(
                key_mod(KeyCode::Right, KeyModifiers::CONTROL),
                Context::Search
            ),
            Action::Line(LineOp::WordRight)
        );
        // Ctrl-C still hard-aborts, even from the search prompt.
        assert_eq!(
            map(
                key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
                Context::Search
            ),
            Action::QuitForce
        );
    }

    #[test]
    fn confirm_context_takes_yes_no() {
        assert_eq!(
            map(key(KeyCode::Char('y')), Context::Confirm),
            Action::ConfirmYes
        );
        assert_eq!(
            map(key(KeyCode::Enter), Context::Confirm),
            Action::ConfirmYes
        );
        assert_eq!(
            map(key(KeyCode::Char('n')), Context::Confirm),
            Action::ConfirmNo
        );
        assert_eq!(map(key(KeyCode::Esc), Context::Confirm), Action::ConfirmNo);
        // Ctrl-C still hard-aborts from a prompt.
        assert_eq!(
            map(
                key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
                Context::Confirm
            ),
            Action::QuitForce
        );
    }
}
