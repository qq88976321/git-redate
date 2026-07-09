//! The edit model and its pure operations.
//!
//! Commits are held oldest-first: index 0 is the oldest commit in the
//! range ("the first commit"), the last index is the tip. The cascade
//! "shift" edit therefore moves commit `i` and every *newer* commit
//! (higher index) by the same delta, preserving the gaps between them.
//!
//! Everything here is pure and gix-free: `Stamp`s are edited in place
//! and later converted to `gix_date::Time` at the rewrite boundary.

use crate::datetime::{self, Component, Stamp};

/// A read-only snapshot of a commit selected for editing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Full hex object id (the rewrite key).
    pub id: String,
    /// Abbreviated id for display.
    pub short_id: String,
    /// First line of the commit message.
    pub summary: String,
    pub author: Stamp,
    pub committer: Stamp,
}

/// A commit plus its in-progress edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditableCommit {
    /// The pristine snapshot, for diffing and reset.
    pub original: Commit,
    /// Current (possibly edited) author time.
    pub author: Stamp,
    /// Current (possibly edited) committer time.
    pub committer: Stamp,
    /// Author/committer rows are shown separately.
    pub expanded: bool,
    /// Edits apply to both author and committer together.
    pub linked: bool,
}

/// Which timestamp(s) an edit targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Author and committer together (the default, linked view).
    Both,
    Author,
    Committer,
}

impl EditableCommit {
    /// Start editing a commit: times mirror the snapshot, linked, and
    /// collapsed.
    pub fn new(original: Commit) -> Self {
        let author = original.author;
        let committer = original.committer;
        Self {
            original,
            author,
            committer,
            expanded: false,
            linked: true,
        }
    }

    /// Whether either timestamp differs from the snapshot.
    pub fn changed(&self) -> bool {
        self.author != self.original.author || self.committer != self.original.committer
    }

    /// The field a delta is measured against for `target`.
    fn primary(&self, target: Target) -> Stamp {
        match target {
            Target::Committer => self.committer,
            Target::Both | Target::Author => self.author,
        }
    }

    fn apply_bump(&mut self, target: Target, component: Component, steps: i64) {
        match target {
            Target::Author => self.author = datetime::bump(self.author, component, steps),
            Target::Committer => self.committer = datetime::bump(self.committer, component, steps),
            Target::Both => {
                self.author = datetime::bump(self.author, component, steps);
                self.committer = datetime::bump(self.committer, component, steps);
            }
        }
    }

    fn apply_set(&mut self, target: Target, stamp: Stamp) {
        match target {
            Target::Author => self.author = stamp,
            Target::Committer => self.committer = stamp,
            Target::Both => {
                self.author = stamp;
                self.committer = stamp;
            }
        }
    }

    fn apply_delta(&mut self, target: Target, delta: i64) {
        match target {
            Target::Author => self.author = datetime::add_delta(self.author, delta),
            Target::Committer => self.committer = datetime::add_delta(self.committer, delta),
            Target::Both => {
                self.author = datetime::add_delta(self.author, delta);
                self.committer = datetime::add_delta(self.committer, delta);
            }
        }
    }
}

/// Whether any commit has unsaved edits.
pub fn any_changed(commits: &[EditableCommit]) -> bool {
    commits.iter().any(EditableCommit::changed)
}

/// Increment/decrement a field of commit `i` by `steps`. In `cascade`
/// mode the resulting seconds delta also shifts every newer commit's
/// same field, preserving the gaps.
pub fn bump(
    commits: &mut [EditableCommit],
    i: usize,
    target: Target,
    component: Component,
    steps: i64,
    cascade: bool,
) {
    if i >= commits.len() {
        return;
    }
    let before = commits[i].primary(target).seconds;
    commits[i].apply_bump(target, component, steps);
    let delta = commits[i].primary(target).seconds - before;
    if cascade && delta != 0 {
        cascade_delta(commits, i, target, delta);
    }
}

/// Set commit `i`'s field(s) to an absolute stamp (from text entry). In
/// `cascade` mode the delta from the old value shifts newer commits.
pub fn set(commits: &mut [EditableCommit], i: usize, target: Target, stamp: Stamp, cascade: bool) {
    if i >= commits.len() {
        return;
    }
    let before = commits[i].primary(target).seconds;
    commits[i].apply_set(target, stamp);
    let delta = commits[i].primary(target).seconds - before;
    if cascade && delta != 0 {
        cascade_delta(commits, i, target, delta);
    }
}

/// Add `delta` seconds to `target` field(s) of every commit newer than
/// `i` (higher index), preserving their offsets.
fn cascade_delta(commits: &mut [EditableCommit], i: usize, target: Target, delta: i64) {
    for ec in commits.iter_mut().skip(i + 1) {
        ec.apply_delta(target, delta);
    }
}

/// Copy the older neighbour's (index `i-1`) author and committer times
/// onto commit `i`. No-op at the oldest commit.
pub fn copy_from_previous(commits: &mut [EditableCommit], i: usize) {
    if i == 0 || i >= commits.len() {
        return;
    }
    let author = commits[i - 1].author;
    let committer = commits[i - 1].committer;
    commits[i].author = author;
    commits[i].committer = committer;
}

/// Space the interior commits evenly in time between the fixed first
/// and last commits (author and committer independently), keeping each
/// commit's own offset. No-op with fewer than three commits.
pub fn distribute(commits: &mut [EditableCommit]) {
    let n = commits.len();
    if n < 3 {
        return;
    }
    let a0 = commits[0].author.seconds;
    let a1 = commits[n - 1].author.seconds;
    let c0 = commits[0].committer.seconds;
    let c1 = commits[n - 1].committer.seconds;
    for (j, ec) in commits[1..n - 1].iter_mut().enumerate() {
        let k = j + 1;
        let a = interpolate(a0, a1, k, n - 1);
        let c = interpolate(c0, c1, k, n - 1);
        ec.author = Stamp::new(a, ec.author.offset);
        ec.committer = Stamp::new(c, ec.committer.offset);
    }
}

/// `start + (end - start) * k / steps`, computed in i128 to avoid
/// overflow and rounded to the nearest second.
fn interpolate(start: i64, end: i64, k: usize, steps: usize) -> i64 {
    let span = end as i128 - start as i128;
    let num = span * k as i128;
    let steps = steps as i128;
    // Round to nearest rather than truncating toward zero.
    let rounded = (num + span.signum() * (steps / 2)) / steps;
    start + rounded as i64
}

/// Reset commit `i` to its snapshot.
pub fn reset(commits: &mut [EditableCommit], i: usize) {
    if i >= commits.len() {
        return;
    }
    commits[i].author = commits[i].original.author;
    commits[i].committer = commits[i].original.committer;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datetime::parse_in_offset;

    fn commit(id: &str, wall: &str) -> Commit {
        let s = parse_in_offset(wall, 0).unwrap();
        Commit {
            id: id.to_string(),
            short_id: id[..id.len().min(7)].to_string(),
            summary: format!("commit {id}"),
            author: s,
            committer: s,
        }
    }

    fn editable(walls: &[&str]) -> Vec<EditableCommit> {
        walls
            .iter()
            .enumerate()
            .map(|(k, w)| EditableCommit::new(commit(&format!("id{k}0000000"), w)))
            .collect()
    }

    fn walls(commits: &[EditableCommit]) -> Vec<String> {
        commits.iter().map(|c| datetime::format(c.author)).collect()
    }

    #[test]
    fn new_mirrors_snapshot_and_is_unchanged() {
        let ec = EditableCommit::new(commit("abc", "2024-01-01 01:01"));
        assert!(!ec.changed());
        assert!(ec.linked);
        assert!(!ec.expanded);
        assert_eq!(ec.author, ec.original.author);
    }

    #[test]
    fn bump_single_touches_only_that_commit() {
        let mut cs = editable(&["2024-01-01 01:01", "2024-01-01 01:02", "2024-01-01 02:00"]);
        bump(&mut cs, 0, Target::Both, Component::Hour, 1, false);
        assert_eq!(
            walls(&cs),
            ["2024-01-01 02:01", "2024-01-01 01:02", "2024-01-01 02:00"]
        );
        assert!(cs[0].changed());
        assert!(!cs[1].changed());
    }

    #[test]
    fn bump_shift_matches_the_worked_example() {
        // 01:01, 01:02, 02:00 -> edit the first by +1h in shift mode ->
        // 02:01, 02:02, 03:00 (relative gaps preserved).
        let mut cs = editable(&["2024-01-01 01:01", "2024-01-01 01:02", "2024-01-01 02:00"]);
        bump(&mut cs, 0, Target::Both, Component::Hour, 1, true);
        assert_eq!(
            walls(&cs),
            ["2024-01-01 02:01", "2024-01-01 02:02", "2024-01-01 03:00"]
        );
        // committer moved in lockstep with author (linked / Both).
        assert!(cs.iter().all(|c| c.author == c.committer));
    }

    #[test]
    fn bump_shift_from_the_middle_leaves_older_untouched() {
        let mut cs = editable(&["2024-01-01 01:00", "2024-01-01 02:00", "2024-01-01 03:00"]);
        bump(&mut cs, 1, Target::Both, Component::Hour, 1, true);
        assert_eq!(
            walls(&cs),
            ["2024-01-01 01:00", "2024-01-01 03:00", "2024-01-01 04:00"]
        );
        assert!(!cs[0].changed());
    }

    #[test]
    fn set_absolute_single_and_cascade() {
        let mut cs = editable(&["2024-01-01 01:00", "2024-01-01 01:30", "2024-01-01 02:00"]);
        let target_time = parse_in_offset("2024-01-01 05:00", 0).unwrap();
        // single: only commit 0 changes.
        set(&mut cs, 0, Target::Both, target_time, false);
        assert_eq!(walls(&cs)[0], "2024-01-01 05:00");
        assert_eq!(walls(&cs)[1], "2024-01-01 01:30");

        // cascade from a fresh set: delta +4h propagates.
        let mut cs = editable(&["2024-01-01 01:00", "2024-01-01 01:30", "2024-01-01 02:00"]);
        set(&mut cs, 0, Target::Both, target_time, true);
        assert_eq!(
            walls(&cs),
            ["2024-01-01 05:00", "2024-01-01 05:30", "2024-01-01 06:00"]
        );
    }

    #[test]
    fn target_author_only_leaves_committer() {
        let mut cs = editable(&["2024-01-01 01:00"]);
        bump(&mut cs, 0, Target::Author, Component::Hour, 2, false);
        assert_eq!(datetime::format(cs[0].author), "2024-01-01 03:00");
        assert_eq!(datetime::format(cs[0].committer), "2024-01-01 01:00");
    }

    #[test]
    fn copy_from_previous_takes_the_older_neighbour() {
        let mut cs = editable(&["2024-01-01 01:00", "2024-01-01 09:00"]);
        copy_from_previous(&mut cs, 1);
        assert_eq!(datetime::format(cs[1].author), "2024-01-01 01:00");
        // No-op at the oldest.
        let mut cs2 = editable(&["2024-01-01 01:00"]);
        copy_from_previous(&mut cs2, 0);
        assert!(!cs2[0].changed());
    }

    #[test]
    fn distribute_spaces_interior_evenly() {
        // 00:00 .. 04:00 across 5 commits -> hourly steps.
        let mut cs = editable(&[
            "2024-01-01 00:00",
            "2024-01-01 03:00",
            "2024-01-01 03:10",
            "2024-01-01 03:20",
            "2024-01-01 04:00",
        ]);
        distribute(&mut cs);
        assert_eq!(
            walls(&cs),
            [
                "2024-01-01 00:00",
                "2024-01-01 01:00",
                "2024-01-01 02:00",
                "2024-01-01 03:00",
                "2024-01-01 04:00",
            ]
        );
        // Endpoints untouched.
        assert!(!cs[0].changed());
        assert!(!cs[4].changed());
    }

    #[test]
    fn distribute_is_noop_below_three() {
        let mut cs = editable(&["2024-01-01 00:00", "2024-01-01 09:00"]);
        distribute(&mut cs);
        assert!(!cs[0].changed());
        assert!(!cs[1].changed());
    }

    #[test]
    fn reset_restores_the_snapshot() {
        let mut cs = editable(&["2024-01-01 01:00"]);
        bump(&mut cs, 0, Target::Both, Component::Day, 5, false);
        assert!(cs[0].changed());
        reset(&mut cs, 0);
        assert!(!cs[0].changed());
    }

    #[test]
    fn any_changed_reflects_edits() {
        let mut cs = editable(&["2024-01-01 01:00", "2024-01-01 02:00"]);
        assert!(!any_changed(&cs));
        bump(&mut cs, 1, Target::Both, Component::Minute, 1, false);
        assert!(any_changed(&cs));
    }

    #[test]
    fn bump_offset_preserved_on_cascade() {
        // Different offsets per commit; a Both shift keeps each offset.
        let mut cs = vec![
            EditableCommit::new(Commit {
                id: "a".into(),
                short_id: "a".into(),
                summary: "a".into(),
                author: Stamp::new(0, 8 * 3600),
                committer: Stamp::new(0, 8 * 3600),
            }),
            EditableCommit::new(Commit {
                id: "b".into(),
                short_id: "b".into(),
                summary: "b".into(),
                author: Stamp::new(3600, -5 * 3600),
                committer: Stamp::new(3600, -5 * 3600),
            }),
        ];
        bump(&mut cs, 0, Target::Both, Component::Hour, 1, true);
        assert_eq!(cs[0].author.offset, 8 * 3600);
        assert_eq!(cs[1].author.offset, -5 * 3600);
        // Both advanced by exactly one hour (3600s).
        assert_eq!(cs[0].author.seconds, 3600);
        assert_eq!(cs[1].author.seconds, 3600 + 3600);
    }
}
