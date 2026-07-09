//! Interactive editor state machine.
//!
//! `App` holds the edit buffer and cursor; [`App::handle`] applies an
//! [`Action`] as a pure state transition (no terminal, no gix), so the
//! whole interaction is unit-testable. `main` owns the render/read loop
//! and, on `w`, hands `App::commits` to the rewrite step.

use crate::cli::EditMode;
use crate::datetime::{self, Component, Stamp};
use crate::input::Action;
use crate::model::{self, EditableCommit, Target};

/// Which of a commit's two timestamps is focused (when expanded).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubField {
    Author,
    Committer,
}

/// What a confirmation prompt will do if accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmKind {
    Write,
    Quit,
}

/// Top-level interaction mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Navigate,
    /// Typing an absolute date into the focused field.
    Editing {
        buffer: String,
    },
    /// Awaiting y/N for a write or a discard-and-quit.
    Confirm {
        kind: ConfirmKind,
    },
}

/// The datetime fields the cursor can move through, collapsed vs
/// expanded (the offset is only editable in the expanded view).
const COLLAPSED: [Component; 5] = [
    Component::Year,
    Component::Month,
    Component::Day,
    Component::Hour,
    Component::Minute,
];
const EXPANDED: [Component; 6] = [
    Component::Year,
    Component::Month,
    Component::Day,
    Component::Hour,
    Component::Minute,
    Component::Offset,
];

/// The interactive editor state.
pub struct App {
    pub commits: Vec<EditableCommit>,
    pub selected: usize,
    pub component: Component,
    pub sub: SubField,
    pub edit_mode: EditMode,
    pub mode: Mode,
    pub dry_run: bool,
    pub message: Option<String>,
    pub show_help: bool,
    pub quit: bool,
    pub write_requested: bool,
}

impl App {
    /// Build the editor over the loaded commits.
    pub fn new(
        commits: Vec<EditableCommit>,
        edit_mode: EditMode,
        dry_run: bool,
        separate: bool,
    ) -> Self {
        let mut app = App {
            commits,
            selected: 0,
            component: Component::Minute,
            sub: SubField::Author,
            edit_mode,
            mode: Mode::Navigate,
            dry_run,
            message: None,
            show_help: false,
            quit: false,
            write_requested: false,
        };
        if separate {
            for c in &mut app.commits {
                c.expanded = true;
            }
        }
        app
    }

    pub fn is_editing(&self) -> bool {
        matches!(self.mode, Mode::Editing { .. })
    }

    /// The input keymap that applies to the current mode.
    pub fn context(&self) -> crate::input::Context {
        match self.mode {
            Mode::Editing { .. } => crate::input::Context::Editing,
            Mode::Confirm { .. } => crate::input::Context::Confirm,
            Mode::Navigate => crate::input::Context::Navigate,
        }
    }

    fn expanded(&self) -> bool {
        self.commits.get(self.selected).is_some_and(|c| c.expanded)
    }

    fn components(&self) -> &'static [Component] {
        if self.expanded() {
            &EXPANDED
        } else {
            &COLLAPSED
        }
    }

    /// Which timestamp(s) an edit targets given the current view.
    pub fn target(&self) -> Target {
        if self.expanded() {
            match self.sub {
                SubField::Author => Target::Author,
                SubField::Committer => Target::Committer,
            }
        } else {
            Target::Both
        }
    }

    /// The stamp currently under the cursor (for text-edit seeding).
    pub fn focused_stamp(&self) -> Option<Stamp> {
        let c = self.commits.get(self.selected)?;
        Some(if self.expanded() && self.sub == SubField::Committer {
            c.committer
        } else {
            c.author
        })
    }

    fn cascade(&self) -> bool {
        self.edit_mode == EditMode::Shift
    }

    /// Apply an action as a state transition.
    pub fn handle(&mut self, action: Action) {
        self.message = None;
        // A hard abort (Ctrl-C) quits immediately from any mode.
        if action == Action::QuitForce {
            self.quit = true;
            return;
        }
        match self.mode {
            Mode::Editing { .. } => self.handle_editing(action),
            Mode::Confirm { .. } => self.handle_confirm(action),
            Mode::Navigate => self.handle_navigate(action),
        }
    }

    fn handle_navigate(&mut self, action: Action) {
        match action {
            Action::PrevRow => self.selected = self.selected.saturating_sub(1),
            Action::NextRow => {
                if self.selected + 1 < self.commits.len() {
                    self.selected += 1;
                }
            }
            Action::PrevComponent => self.move_component(-1),
            Action::NextComponent => self.move_component(1),
            Action::Increment => self.bump(1),
            Action::Decrement => self.bump(-1),
            Action::ToggleExpand => self.toggle_expand(),
            Action::ToggleSubField => {
                if self.expanded() {
                    self.sub = match self.sub {
                        SubField::Author => SubField::Committer,
                        SubField::Committer => SubField::Author,
                    };
                }
            }
            Action::ToggleMode => {
                self.edit_mode = match self.edit_mode {
                    EditMode::Single => EditMode::Shift,
                    EditMode::Shift => EditMode::Single,
                };
                self.message = Some(format!("mode: {}", self.edit_mode));
            }
            Action::CopyPrevious => {
                model::copy_from_previous(&mut self.commits, self.selected);
            }
            Action::Distribute => {
                model::distribute(&mut self.commits);
                self.message = Some("distributed evenly".to_string());
            }
            Action::ResetRow => model::reset(&mut self.commits, self.selected),
            Action::BeginEdit => self.begin_edit(),
            Action::ToggleHelp => self.show_help = !self.show_help,
            Action::Write => self.request_write(false),
            Action::WriteForce => self.request_write(true),
            Action::Quit => self.request_quit(false),
            _ => {}
        }
    }

    /// `w` prompts before writing when there are edits; `W` (force) and
    /// a no-op (nothing changed) write straight through.
    fn request_write(&mut self, force: bool) {
        if force || !model::any_changed(&self.commits) {
            self.write_requested = true;
            self.quit = true;
        } else {
            self.mode = Mode::Confirm {
                kind: ConfirmKind::Write,
            };
        }
    }

    /// `q`/`Esc` prompt before discarding edits; `Q` (force) and a clean
    /// tree quit straight through.
    fn request_quit(&mut self, force: bool) {
        if force || !model::any_changed(&self.commits) {
            self.quit = true;
        } else {
            self.mode = Mode::Confirm {
                kind: ConfirmKind::Quit,
            };
        }
    }

    fn handle_confirm(&mut self, action: Action) {
        let kind = match &self.mode {
            Mode::Confirm { kind } => *kind,
            _ => return,
        };
        match action {
            Action::ConfirmYes => match kind {
                ConfirmKind::Write => {
                    self.write_requested = true;
                    self.quit = true;
                }
                ConfirmKind::Quit => self.quit = true,
            },
            Action::ConfirmNo => self.mode = Mode::Navigate,
            _ => {}
        }
    }

    fn handle_editing(&mut self, action: Action) {
        // Take the buffer out to satisfy the borrow checker on commit.
        let Mode::Editing { buffer } = &mut self.mode else {
            return;
        };
        match action {
            Action::Char(c) => buffer.push(c),
            Action::Backspace => {
                buffer.pop();
            }
            Action::CancelEdit => self.mode = Mode::Navigate,
            Action::CommitEdit => {
                let text = buffer.clone();
                self.commit_edit(&text);
            }
            _ => {}
        }
    }

    fn commit_edit(&mut self, text: &str) {
        let offset = self.focused_stamp().map(|s| s.offset).unwrap_or(0);
        match datetime::parse_in_offset(text, offset) {
            Ok(stamp) => {
                let target = self.target();
                let cascade = self.cascade();
                model::set(&mut self.commits, self.selected, target, stamp, cascade);
                self.mode = Mode::Navigate;
            }
            Err(e) => {
                self.message = Some(e.to_string());
                // Stay in edit mode so the user can fix the text.
            }
        }
    }

    fn begin_edit(&mut self) {
        if let Some(stamp) = self.focused_stamp() {
            self.mode = Mode::Editing {
                buffer: datetime::format(stamp),
            };
        }
    }

    fn move_component(&mut self, delta: i32) {
        let comps = self.components();
        let idx = comps.iter().position(|&c| c == self.component).unwrap_or(0);
        let len = comps.len() as i32;
        let next = (idx as i32 + delta).rem_euclid(len) as usize;
        self.component = comps[next];
    }

    fn bump(&mut self, steps: i64) {
        let target = self.target();
        let component = self.component;
        let cascade = self.cascade();
        model::bump(
            &mut self.commits,
            self.selected,
            target,
            component,
            steps,
            cascade,
        );
    }

    fn toggle_expand(&mut self) {
        if let Some(c) = self.commits.get_mut(self.selected) {
            c.expanded = !c.expanded;
        }
        self.sub = SubField::Author;
        // Leaving the expanded view drops the offset field from the cycle.
        if !self.expanded() && self.component == Component::Offset {
            self.component = Component::Minute;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datetime::parse_in_offset;
    use crate::model::Commit;

    fn app(walls: &[&str], mode: EditMode) -> App {
        let commits = walls
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let s = parse_in_offset(w, 0).unwrap();
                EditableCommit::new(Commit {
                    id: format!("{i:040x}"),
                    short_id: format!("{i:07}"),
                    summary: format!("c{i}"),
                    author: s,
                    committer: s,
                })
            })
            .collect();
        App::new(commits, mode, false, false)
    }

    fn wall(app: &App, i: usize) -> String {
        datetime::format(app.commits[i].author)
    }

    #[test]
    fn navigation_clamps_at_the_ends() {
        let mut a = app(&["2024-01-01 01:00", "2024-01-01 02:00"], EditMode::Single);
        a.handle(Action::PrevRow);
        assert_eq!(a.selected, 0);
        a.handle(Action::NextRow);
        a.handle(Action::NextRow);
        assert_eq!(a.selected, 1);
    }

    #[test]
    fn increment_minute_single_touches_one_row() {
        let mut a = app(&["2024-01-01 01:00", "2024-01-01 02:00"], EditMode::Single);
        a.component = Component::Hour;
        a.handle(Action::Increment);
        assert_eq!(wall(&a, 0), "2024-01-01 02:00");
        assert_eq!(wall(&a, 1), "2024-01-01 02:00"); // unchanged
    }

    #[test]
    fn increment_shift_cascades_to_newer_rows() {
        let mut a = app(
            &["2024-01-01 01:00", "2024-01-01 01:30", "2024-01-01 03:00"],
            EditMode::Shift,
        );
        a.component = Component::Hour;
        a.handle(Action::Increment); // +1h on row 0, cascades
        assert_eq!(wall(&a, 0), "2024-01-01 02:00");
        assert_eq!(wall(&a, 1), "2024-01-01 02:30");
        assert_eq!(wall(&a, 2), "2024-01-01 04:00");
    }

    #[test]
    fn toggle_mode_switches_cascade() {
        let mut a = app(&["2024-01-01 01:00"], EditMode::Single);
        assert!(!a.cascade());
        a.handle(Action::ToggleMode);
        assert!(a.cascade());
        assert_eq!(a.message.as_deref(), Some("mode: shift"));
    }

    #[test]
    fn expand_adds_offset_to_the_component_cycle() {
        let mut a = app(&["2024-01-01 01:00"], EditMode::Single);
        assert_eq!(a.components().len(), 5);
        a.handle(Action::ToggleExpand);
        assert!(a.expanded());
        assert_eq!(a.components().len(), 6);
    }

    #[test]
    fn expanded_targets_the_focused_subfield() {
        let mut a = app(&["2024-01-01 01:00"], EditMode::Single);
        a.handle(Action::ToggleExpand);
        assert_eq!(a.target(), Target::Author);
        a.component = Component::Hour;
        a.handle(Action::Increment);
        assert_eq!(datetime::format(a.commits[0].author), "2024-01-01 02:00");
        assert_eq!(datetime::format(a.commits[0].committer), "2024-01-01 01:00");
        // Switch to committer and edit it independently.
        a.handle(Action::ToggleSubField);
        assert_eq!(a.target(), Target::Committer);
        a.handle(Action::Increment);
        assert_eq!(datetime::format(a.commits[0].committer), "2024-01-01 02:00");
    }

    #[test]
    fn text_edit_sets_an_absolute_time() {
        let mut a = app(&["2024-01-01 01:00"], EditMode::Single);
        a.handle(Action::BeginEdit);
        assert!(a.is_editing());
        // Clear and type a new value.
        for _ in 0..16 {
            a.handle(Action::Backspace);
        }
        for c in "2024-06-15 09:30".chars() {
            a.handle(Action::Char(c));
        }
        a.handle(Action::CommitEdit);
        assert!(!a.is_editing());
        assert_eq!(wall(&a, 0), "2024-06-15 09:30");
    }

    #[test]
    fn text_edit_rejects_bad_input_and_stays_open() {
        let mut a = app(&["2024-01-01 01:00"], EditMode::Single);
        a.handle(Action::BeginEdit);
        for _ in 0..16 {
            a.handle(Action::Backspace);
        }
        for c in "nonsense".chars() {
            a.handle(Action::Char(c));
        }
        a.handle(Action::CommitEdit);
        assert!(a.is_editing()); // still editing
        assert!(a.message.is_some());
        assert_eq!(wall(&a, 0), "2024-01-01 01:00"); // unchanged
    }

    #[test]
    fn write_and_quit_without_edits_go_straight_through() {
        // Nothing changed -> no confirmation prompt.
        let mut a = app(&["2024-01-01 01:00"], EditMode::Single);
        a.handle(Action::Write);
        assert!(a.write_requested);
        assert!(a.quit);

        let mut b = app(&["2024-01-01 01:00"], EditMode::Single);
        b.handle(Action::Quit);
        assert!(!b.write_requested);
        assert!(b.quit);
    }

    fn edited(mode: EditMode) -> App {
        let mut a = app(&["2024-01-01 01:00", "2024-01-01 02:00"], mode);
        a.component = Component::Hour;
        a.handle(Action::Increment); // make row 0 dirty
        assert!(crate::model::any_changed(&a.commits));
        a
    }

    #[test]
    fn write_with_edits_prompts_then_confirms() {
        let mut a = edited(EditMode::Single);
        a.handle(Action::Write);
        assert!(matches!(
            a.mode,
            Mode::Confirm {
                kind: ConfirmKind::Write
            }
        ));
        assert_eq!(a.context(), crate::input::Context::Confirm);
        assert!(!a.write_requested && !a.quit);
        a.handle(Action::ConfirmYes);
        assert!(a.write_requested && a.quit);
    }

    #[test]
    fn write_prompt_can_be_cancelled() {
        let mut a = edited(EditMode::Single);
        a.handle(Action::Write);
        a.handle(Action::ConfirmNo);
        assert_eq!(a.mode, Mode::Navigate);
        assert!(!a.write_requested && !a.quit);
    }

    #[test]
    fn force_write_skips_the_prompt() {
        let mut a = edited(EditMode::Single);
        a.handle(Action::WriteForce);
        assert!(a.write_requested && a.quit);
    }

    #[test]
    fn quit_with_edits_prompts_then_discards() {
        let mut a = edited(EditMode::Single);
        a.handle(Action::Quit);
        assert!(matches!(
            a.mode,
            Mode::Confirm {
                kind: ConfirmKind::Quit
            }
        ));
        a.handle(Action::ConfirmYes);
        assert!(a.quit && !a.write_requested);
    }

    #[test]
    fn force_quit_aborts_from_any_mode() {
        // Even mid text-edit, a hard abort (Ctrl-C -> QuitForce) quits.
        let mut a = edited(EditMode::Single);
        a.handle(Action::BeginEdit);
        assert!(a.is_editing());
        a.handle(Action::QuitForce);
        assert!(a.quit && !a.write_requested);
    }

    #[test]
    fn distribute_and_copy_and_reset() {
        let mut a = app(
            &["2024-01-01 00:00", "2024-01-01 05:00", "2024-01-01 02:00"],
            EditMode::Single,
        );
        a.handle(Action::Distribute);
        assert_eq!(wall(&a, 1), "2024-01-01 01:00"); // midpoint of 00:00..02:00

        a.selected = 1;
        a.handle(Action::CopyPrevious);
        assert_eq!(wall(&a, 1), wall(&a, 0));

        a.handle(Action::ResetRow);
        assert_eq!(wall(&a, 1), "2024-01-01 05:00"); // back to snapshot
    }
}
