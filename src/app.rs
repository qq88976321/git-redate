//! Interactive editor state machine.
//!
//! `App` holds the edit buffer and cursor; [`App::handle`] applies an
//! [`Action`] as a pure state transition (no terminal, no gix), so the
//! whole interaction is unit-testable. `main` owns the render/read loop
//! and, on `w`, hands `App::commits` to the rewrite step.

use crate::cli::EditMode;
use crate::datetime::{self, Component, Stamp};
use crate::input::Action;
use crate::lineedit::LineEditor;
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
    ResetAll,
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
    /// Typing an incremental search query that jumps the selection.
    Search {
        editor: LineEditor,
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

/// A point-in-time copy of the editable timestamps and cursor, used to
/// implement undo/redo. Only the mutable timestamps and the cursor are
/// captured; the view (expanded rows, edit mode) is left as it is.
#[derive(Clone)]
struct Snapshot {
    /// (author, committer) per commit, indexed like `App::commits`.
    times: Vec<(Stamp, Stamp)>,
    selected: usize,
    component: Component,
    sub: SubField,
}

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
    /// Undo/redo history of timestamp edits (most recent on top).
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// Last committed search query, replayed by `n`/`N`.
    search_query: Option<String>,
    /// Selection when the current search started, restored on cancel.
    search_origin: usize,
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
            undo: Vec::new(),
            redo: Vec::new(),
            search_query: None,
            search_origin: 0,
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
            Mode::Search { .. } => crate::input::Context::Search,
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

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            times: self
                .commits
                .iter()
                .map(|c| (c.author, c.committer))
                .collect(),
            selected: self.selected,
            component: self.component,
            sub: self.sub,
        }
    }

    /// After a timestamp-mutating edit, push `before` onto the undo stack
    /// (and clear redo - a fresh edit forks history), but only if a
    /// timestamp actually changed, so no-op edits leave no undo step.
    fn push_edit(&mut self, before: Snapshot) {
        let changed = self
            .commits
            .iter()
            .zip(before.times.iter())
            .any(|(c, t)| c.author != t.0 || c.committer != t.1);
        if changed {
            self.undo.push(before);
            self.redo.clear();
        }
    }

    /// Restore timestamps and cursor from a snapshot, leaving the view
    /// (expanded rows, edit mode) untouched.
    fn restore(&mut self, s: &Snapshot) {
        for (c, t) in self.commits.iter_mut().zip(s.times.iter()) {
            c.author = t.0;
            c.committer = t.1;
        }
        self.selected = s.selected.min(self.commits.len().saturating_sub(1));
        self.component = s.component;
        self.sub = s.sub;
        // The offset field only exists in the expanded view.
        if self.component == Component::Offset && !self.expanded() {
            self.component = Component::Minute;
        }
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            let now = self.snapshot();
            self.restore(&prev);
            self.redo.push(now);
        } else {
            self.message = Some("nothing to undo".to_string());
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            let now = self.snapshot();
            self.restore(&next);
            self.undo.push(now);
        } else {
            self.message = Some("nothing to redo".to_string());
        }
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
            Mode::Search { .. } => self.handle_search(action),
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
                let before = self.snapshot();
                model::copy_from_previous(&mut self.commits, self.selected);
                self.push_edit(before);
            }
            Action::Distribute => {
                let before = self.snapshot();
                model::distribute(&mut self.commits);
                self.push_edit(before);
                self.message = Some("distributed evenly".to_string());
            }
            Action::ResetRow => {
                let before = self.snapshot();
                model::reset(&mut self.commits, self.selected);
                self.push_edit(before);
            }
            Action::ResetAll => self.request_reset_all(),
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::BeginSearch => {
                self.search_origin = self.selected;
                self.mode = Mode::Search {
                    editor: LineEditor::default(),
                };
            }
            Action::NextMatch => self.jump_match(1),
            Action::PrevMatch => self.jump_match(-1),
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

    /// `U` resets every commit; prompt first, since it discards all edits.
    /// A no-op (nothing changed) just reports so.
    fn request_reset_all(&mut self) {
        if model::any_changed(&self.commits) {
            self.mode = Mode::Confirm {
                kind: ConfirmKind::ResetAll,
            };
        } else {
            self.message = Some("nothing to reset".to_string());
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
                ConfirmKind::ResetAll => {
                    let before = self.snapshot();
                    model::reset_all(&mut self.commits);
                    self.push_edit(before);
                    self.mode = Mode::Navigate;
                    self.message = Some("reset all".to_string());
                }
            },
            Action::ConfirmNo => self.mode = Mode::Navigate,
            _ => {}
        }
    }

    fn handle_search(&mut self, action: Action) {
        match action {
            Action::Line(op) => {
                // Apply the edit, then re-run the search from the origin
                // only when the text actually changed (not on cursor moves).
                let query = if let Mode::Search { editor } = &mut self.mode {
                    editor.apply(op);
                    op.mutates().then(|| editor.text().to_string())
                } else {
                    None
                };
                if let Some(q) = query {
                    self.selected = self
                        .match_index(self.search_origin, 1, &q)
                        .unwrap_or(self.search_origin);
                }
            }
            Action::CommitSearch => {
                if let Mode::Search { editor } = &self.mode {
                    let q = editor.text().to_string();
                    self.search_query = (!q.is_empty()).then_some(q);
                }
                self.mode = Mode::Navigate;
            }
            Action::CancelSearch => {
                self.selected = self.search_origin.min(self.commits.len().saturating_sub(1));
                self.mode = Mode::Navigate;
            }
            _ => {}
        }
    }

    /// Index of the next commit matching `q` (case-insensitive substring
    /// of the summary or short id), scanning from `start` in direction
    /// `dir` (+1/-1) and wrapping. `None` if nothing matches.
    fn match_index(&self, start: usize, dir: isize, q: &str) -> Option<usize> {
        let n = self.commits.len();
        if n == 0 || q.is_empty() {
            return None;
        }
        let needle = q.to_lowercase();
        (0..n).find_map(|step| {
            let idx = (start as isize + dir * step as isize).rem_euclid(n as isize) as usize;
            let c = &self.commits[idx].original;
            let hit = c.summary.to_lowercase().contains(&needle)
                || c.short_id.to_lowercase().contains(&needle);
            hit.then_some(idx)
        })
    }

    /// `n`/`N`: jump to the next/previous match of the committed query,
    /// starting one commit past the current selection so it advances.
    fn jump_match(&mut self, dir: isize) {
        let Some(q) = self.search_query.clone() else {
            self.message = Some("no active search (press / to search)".to_string());
            return;
        };
        let n = self.commits.len();
        let start = (self.selected as isize + dir).rem_euclid(n as isize) as usize;
        if let Some(i) = self.match_index(start, dir, &q) {
            self.selected = i;
        } else {
            self.message = Some(format!("no match for \"{q}\""));
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
                let before = self.snapshot();
                model::set(&mut self.commits, self.selected, target, stamp, cascade);
                self.push_edit(before);
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
        let before = self.snapshot();
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
        self.push_edit(before);
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
    use crate::lineedit::LineOp;
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
    fn reset_all_prompts_then_restores_every_commit() {
        let mut a = edited(EditMode::Single);
        // A second dirty row, to prove the whole list is reset.
        a.selected = 1;
        a.handle(Action::Increment);
        assert!(crate::model::any_changed(&a.commits));

        a.handle(Action::ResetAll);
        assert!(matches!(
            a.mode,
            Mode::Confirm {
                kind: ConfirmKind::ResetAll
            }
        ));
        a.handle(Action::ConfirmYes);
        assert_eq!(a.mode, Mode::Navigate);
        assert!(a.commits.iter().all(|c| !c.changed()));
    }

    #[test]
    fn reset_all_can_be_cancelled_and_is_noop_when_clean() {
        let mut a = edited(EditMode::Single);
        a.handle(Action::ResetAll);
        a.handle(Action::ConfirmNo);
        assert_eq!(a.mode, Mode::Navigate);
        assert!(crate::model::any_changed(&a.commits)); // edits kept

        // Nothing changed -> no prompt, just a message.
        let mut b = app(&["2024-01-01 01:00"], EditMode::Single);
        b.handle(Action::ResetAll);
        assert_eq!(b.mode, Mode::Navigate);
        assert!(b.message.is_some());
    }

    #[test]
    fn undo_and_redo_round_trip_an_edit() {
        let mut a = edited(EditMode::Single); // row 0 +1h at the hour field
        assert_eq!(wall(&a, 0), "2024-01-01 02:00");
        a.handle(Action::Undo);
        assert_eq!(wall(&a, 0), "2024-01-01 01:00");
        assert!(!crate::model::any_changed(&a.commits));
        a.handle(Action::Redo);
        assert_eq!(wall(&a, 0), "2024-01-01 02:00");
    }

    #[test]
    fn a_fresh_edit_forks_history_and_clears_redo() {
        let mut a = edited(EditMode::Single);
        a.handle(Action::Undo); // back to the clean state
        assert_eq!(wall(&a, 0), "2024-01-01 01:00");
        // A new edit while there is redo history discards that redo.
        a.component = Component::Hour;
        a.handle(Action::Increment); // 02:00 again, but a distinct step
        a.handle(Action::Redo);
        assert_eq!(a.message.as_deref(), Some("nothing to redo"));
        assert_eq!(wall(&a, 0), "2024-01-01 02:00");
    }

    #[test]
    fn undo_with_empty_history_reports() {
        let mut a = app(&["2024-01-01 01:00"], EditMode::Single);
        a.handle(Action::Undo);
        assert_eq!(a.message.as_deref(), Some("nothing to undo"));
    }

    #[test]
    fn no_op_edit_records_no_undo_step() {
        // copy-from-previous at the oldest commit changes nothing, so
        // there is nothing to undo afterwards.
        let mut a = app(&["2024-01-01 01:00", "2024-01-01 02:00"], EditMode::Single);
        a.handle(Action::CopyPrevious); // selected == 0 -> no-op
        a.handle(Action::Undo);
        assert_eq!(a.message.as_deref(), Some("nothing to undo"));
    }

    fn typed_search(a: &mut App, query: &str) {
        a.handle(Action::BeginSearch);
        for c in query.chars() {
            a.handle(Action::Line(LineOp::Insert(c)));
        }
    }

    #[test]
    fn search_jumps_to_the_match_and_cancel_restores_origin() {
        let mut a = app(
            &["2024-01-01 01:00", "2024-01-01 02:00", "2024-01-01 03:00"],
            EditMode::Single,
        );
        // Summaries are c0, c1, c2; from row 0, "c2" jumps to row 2.
        typed_search(&mut a, "c2");
        assert_eq!(a.selected, 2);
        a.handle(Action::CancelSearch);
        assert_eq!(a.selected, 0); // origin restored
        assert_eq!(a.mode, Mode::Navigate);
        assert!(a.search_query.is_none());
        // The commit list itself is never touched by searching.
        assert!(!crate::model::any_changed(&a.commits));
    }

    #[test]
    fn committed_search_cycles_with_n_and_shift_n() {
        let mut a = app(
            &[
                "2024-01-01 01:00",
                "2024-01-01 02:00",
                "2024-01-01 03:00",
                "2024-01-01 04:00",
            ],
            EditMode::Single,
        );
        typed_search(&mut a, "c"); // matches every commit
        a.handle(Action::CommitSearch);
        assert_eq!(a.search_query.as_deref(), Some("c"));
        let start = a.selected;
        a.handle(Action::NextMatch);
        assert_eq!(a.selected, (start + 1) % 4);
        a.handle(Action::PrevMatch);
        assert_eq!(a.selected, start);
    }

    #[test]
    fn search_matches_short_id_too() {
        let mut a = app(&["2024-01-01 01:00", "2024-01-01 02:00"], EditMode::Single);
        // short_id is formatted as 7 digits: "0000000", "0000001".
        typed_search(&mut a, "0000001");
        assert_eq!(a.selected, 1);
    }

    #[test]
    fn next_match_without_a_search_reports() {
        let mut a = app(&["2024-01-01 01:00"], EditMode::Single);
        a.handle(Action::NextMatch);
        assert!(a.message.is_some());
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
