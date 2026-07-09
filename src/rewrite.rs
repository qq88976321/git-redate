//! Writing side: rebuild commit objects with edited dates and remapped
//! parents, then (in a later step) move the ref.
//!
//! Rewriting is done oldest-first over the range. Only commits from the
//! first *changed* one onward are rebuilt: earlier commits keep their
//! object ids, and every commit at or after the first change is rebuilt
//! so its parent link points at the rewritten ancestor. Trees are left
//! untouched, so file content is identical; only the author/committer
//! times (and parent links) change.

use crate::datetime::Stamp;
use crate::error::RedateError;
use crate::model::EditableCommit;
use crate::repo::RefTarget;
use gix::bstr::ByteSlice;
use gix::ObjectId;
use std::collections::HashMap;

/// Outcome of a completed rewrite, for the summary printed to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteReport {
    /// Tip before the rewrite (the undo target).
    pub old_tip: ObjectId,
    /// Tip after the rewrite (equals old_tip when nothing changed).
    pub new_tip: ObjectId,
    /// Number of commits rewritten.
    pub count: usize,
    /// Commits whose GPG signature was dropped.
    pub dropped_signatures: usize,
}

/// Rewrite the edited commits and move the current branch/HEAD to the
/// new tip, writing a reflog entry. A no-op (nothing changed) leaves
/// the ref untouched. Not called for `--dry-run` (which writes nothing;
/// main prints the planned changes from the model instead).
pub fn apply(
    repo: &gix::Repository,
    commits: &[EditableCommit],
    old_tip: ObjectId,
    ref_target: &RefTarget,
) -> Result<RewriteReport, RedateError> {
    let rewritten = write_rewritten(repo, commits)?;
    if rewritten.count > 0 {
        move_ref(
            repo,
            ref_target,
            old_tip,
            rewritten.new_tip,
            rewritten.count,
        )?;
    }
    Ok(RewriteReport {
        old_tip,
        new_tip: rewritten.new_tip,
        count: rewritten.count,
        dropped_signatures: rewritten.dropped_signatures,
    })
}

/// Point the branch (or detached HEAD) at `new_tip`, writing a reflog
/// entry and asserting it still points at `old_tip` first.
fn move_ref(
    repo: &gix::Repository,
    ref_target: &RefTarget,
    old_tip: ObjectId,
    new_tip: ObjectId,
    count: usize,
) -> Result<(), RedateError> {
    use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};
    use gix::refs::Target;

    let name: gix::refs::FullName = match ref_target {
        RefTarget::Branch(n) => n.clone(),
        RefTarget::Detached => "HEAD".try_into().map_err(write_err)?,
    };
    let message = format!("redate: rewrote {count} commit(s)");
    repo.edit_reference(RefEdit {
        change: Change::Update {
            log: LogChange {
                mode: RefLog::AndReference,
                force_create_reflog: true,
                message: message.into(),
            },
            expected: PreviousValue::MustExistAndMatch(Target::Object(old_tip)),
            new: Target::Object(new_tip),
        },
        name,
        deref: false,
    })
    .map_err(write_err)?;
    Ok(())
}

/// Result of rebuilding the range's commit objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewritten {
    /// New tip oid (equals the old tip if nothing changed).
    pub new_tip: ObjectId,
    /// Number of commits actually rebuilt.
    pub count: usize,
    /// How many rebuilt commits had a GPG signature dropped.
    pub dropped_signatures: usize,
    /// old oid -> new oid, for the rebuilt commits.
    pub map: HashMap<ObjectId, ObjectId>,
}

/// Rebuild and write the edited commits (oldest-first), returning the
/// new tip and the old->new id map. Writes objects to the odb but does
/// not move any ref (see [`crate::rewrite::apply`]).
pub fn write_rewritten(
    repo: &gix::Repository,
    commits: &[EditableCommit],
) -> Result<Rewritten, RedateError> {
    let tip_oid = parse_oid(&commits.last().ok_or(RedateError::EmptyRange)?.original.id)?;

    let Some(first) = commits.iter().position(EditableCommit::changed) else {
        // No edits: the tip is unchanged.
        return Ok(Rewritten {
            new_tip: tip_oid,
            count: 0,
            dropped_signatures: 0,
            map: HashMap::new(),
        });
    };

    let mut map: HashMap<ObjectId, ObjectId> = HashMap::new();
    let mut dropped = 0;
    let mut new_tip = tip_oid;

    for ec in &commits[first..] {
        let old_oid = parse_oid(&ec.original.id)?;
        let orig = repo.find_commit(old_oid).map_err(write_err)?;
        let decoded = orig.decode().map_err(write_err)?;

        let old_parents: Vec<ObjectId> = orig.parent_ids().map(|id| id.detach()).collect();
        let parents = remap_parents(&old_parents, &map);

        let author = signature(orig.author().map_err(write_err)?, ec.author);
        let committer = signature(orig.committer().map_err(write_err)?, ec.committer);

        // Preserve extra headers except the now-invalid GPG signature.
        let mut extra = Vec::new();
        let mut had_sig = false;
        for &(k, ref v) in &decoded.extra_headers {
            if k == b"gpgsig".as_bstr() {
                had_sig = true;
                continue;
            }
            extra.push((k.to_owned(), v.clone().into_owned()));
        }
        if had_sig {
            dropped += 1;
        }

        let commit = gix::objs::Commit {
            tree: orig.tree_id().map_err(write_err)?.detach(),
            parents: parents.into_iter().collect(),
            author,
            committer,
            encoding: decoded.encoding.map(|e| e.to_owned()),
            message: decoded.message.to_owned(),
            extra_headers: extra,
        };

        let new_oid = repo.write_object(&commit).map_err(write_err)?.detach();
        map.insert(old_oid, new_oid);
        new_tip = new_oid;
    }

    Ok(Rewritten {
        new_tip,
        count: commits.len() - first,
        dropped_signatures: dropped,
        map,
    })
}

/// Replace each parent that was rewritten (present in `map`) with its
/// new oid; leave the rest unchanged.
pub fn remap_parents(parents: &[ObjectId], map: &HashMap<ObjectId, ObjectId>) -> Vec<ObjectId> {
    parents
        .iter()
        .map(|p| map.get(p).copied().unwrap_or(*p))
        .collect()
}

/// Build an owned signature from the original name/email with an edited
/// time.
fn signature(orig: gix::actor::SignatureRef<'_>, stamp: Stamp) -> gix::actor::Signature {
    gix::actor::Signature {
        name: orig.name.to_owned(),
        email: orig.email.to_owned(),
        time: gix::date::Time::new(stamp.seconds, stamp.offset),
    }
}

fn parse_oid(hex: &str) -> Result<ObjectId, RedateError> {
    ObjectId::from_hex(hex.as_bytes()).map_err(|e| RedateError::Write(e.to_string()))
}

fn write_err<E: std::fmt::Display>(e: E) -> RedateError {
    RedateError::Write(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datetime;
    use crate::model::{Commit, EditableCommit};
    use std::sync::atomic::{AtomicU32, Ordering};

    fn oid(n: u32) -> ObjectId {
        ObjectId::from_hex(format!("{n:040x}").as_bytes()).unwrap()
    }

    #[test]
    fn remap_replaces_only_rewritten_parents() {
        let mut map = HashMap::new();
        map.insert(oid(1), oid(101));
        // parent 1 was rewritten -> 101; parent 2 was not -> kept.
        let got = remap_parents(&[oid(1), oid(2)], &map);
        assert_eq!(got, vec![oid(101), oid(2)]);
    }

    #[test]
    fn remap_empty_parents_is_empty() {
        let map = HashMap::new();
        assert!(remap_parents(&[], &map).is_empty());
    }

    // ---- gix scratch-repo integration ----

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    struct Scratch {
        dir: std::path::PathBuf,
        repo: gix::Repository,
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn scratch() -> Scratch {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("git-redate-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = gix::init(&dir).unwrap();
        Scratch { dir, repo }
    }

    fn empty_tree(repo: &gix::Repository) -> ObjectId {
        repo.write_object(&gix::objs::Tree {
            entries: Vec::new(),
        })
        .unwrap()
        .detach()
    }

    fn write_test_commit(
        repo: &gix::Repository,
        tree: ObjectId,
        parents: Vec<ObjectId>,
        secs: i64,
        msg: &str,
    ) -> ObjectId {
        let sig = gix::actor::Signature {
            name: "Tester".into(),
            email: "test@example.com".into(),
            time: gix::date::Time::new(secs, 0),
        };
        let commit = gix::objs::Commit {
            tree,
            parents: parents.into_iter().collect(),
            author: sig.clone(),
            committer: sig,
            encoding: None,
            message: msg.into(),
            extra_headers: Vec::new(),
        };
        repo.write_object(&commit).unwrap().detach()
    }

    /// A linear repo of three commits at 01:00, 02:00, 03:00 (+00:00).
    fn three_commits(repo: &gix::Repository) -> Vec<EditableCommit> {
        let tree = empty_tree(repo);
        let base = datetime::parse_in_offset("2024-01-01 01:00", 0)
            .unwrap()
            .seconds;
        let mut oids = Vec::new();
        let mut parent: Vec<ObjectId> = Vec::new();
        for i in 0..3 {
            let oid = write_test_commit(
                repo,
                tree,
                parent.clone(),
                base + i * 3600,
                &format!("c{i}"),
            );
            oids.push(oid);
            parent = vec![oid];
        }
        oids.iter()
            .enumerate()
            .map(|(i, o)| {
                let s = Stamp::new(base + i as i64 * 3600, 0);
                EditableCommit::new(Commit {
                    id: o.to_string(),
                    short_id: o.to_string()[..7].to_string(),
                    summary: format!("c{i}"),
                    author: s,
                    committer: s,
                })
            })
            .collect()
    }

    #[test]
    fn nothing_changed_keeps_the_tip() {
        let s = scratch();
        let commits = three_commits(&s.repo);
        let tip = parse_oid(&commits.last().unwrap().original.id).unwrap();
        let out = write_rewritten(&s.repo, &commits).unwrap();
        assert_eq!(out.count, 0);
        assert_eq!(out.new_tip, tip);
    }

    #[test]
    fn editing_middle_rewrites_it_and_newer_only() {
        let s = scratch();
        let mut commits = three_commits(&s.repo);
        // Edit the middle commit (index 1) to 05:00.
        let new_time = datetime::parse_in_offset("2024-01-01 05:00", 0).unwrap();
        crate::model::set(&mut commits, 1, crate::model::Target::Both, new_time, false);

        let out = write_rewritten(&s.repo, &commits).unwrap();
        // Commits 1 and 2 are rebuilt (2's parent changed); 0 is not.
        assert_eq!(out.count, 2);
        assert!(!out
            .map
            .contains_key(&parse_oid(&commits[0].original.id).unwrap()));

        // The rewritten middle commit carries the new author time and an
        // identical (empty) tree.
        let old1 = parse_oid(&commits[1].original.id).unwrap();
        let new1 = out.map[&old1];
        let rebuilt = s.repo.find_commit(new1).unwrap();
        assert_eq!(
            rebuilt.author().unwrap().time().unwrap().seconds,
            new_time.seconds
        );
        assert_eq!(
            rebuilt.tree_id().unwrap().detach(),
            s.repo
                .find_commit(old1)
                .unwrap()
                .tree_id()
                .unwrap()
                .detach()
        );

        // The newest commit was re-parented onto the rewritten middle.
        let new_tip = s.repo.find_commit(out.new_tip).unwrap();
        let tip_parent = new_tip.parent_ids().next().unwrap().detach();
        assert_eq!(tip_parent, new1);
    }

    #[test]
    fn shift_edit_moves_all_newer_times() {
        let s = scratch();
        let mut commits = three_commits(&s.repo);
        // Shift the first commit +1h; newer ones follow.
        crate::model::bump(
            &mut commits,
            0,
            crate::model::Target::Both,
            datetime::Component::Hour,
            1,
            true,
        );
        let out = write_rewritten(&s.repo, &commits).unwrap();
        assert_eq!(out.count, 3);

        // Each rewritten commit is one hour later than the original.
        let base = datetime::parse_in_offset("2024-01-01 01:00", 0)
            .unwrap()
            .seconds;
        for (i, ec) in commits.iter().enumerate() {
            let old = parse_oid(&ec.original.id).unwrap();
            let new = out.map[&old];
            let c = s.repo.find_commit(new).unwrap();
            let want = base + i as i64 * 3600 + 3600;
            assert_eq!(c.author().unwrap().time().unwrap().seconds, want);
            assert_eq!(c.committer().unwrap().time().unwrap().seconds, want);
        }
    }

    /// Point a ref at `oid` (creating it), used to set up HEAD/branch.
    fn set_ref(repo: &gix::Repository, name: &str, oid: ObjectId) {
        use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};
        use gix::refs::Target;
        repo.edit_reference(RefEdit {
            change: Change::Update {
                log: LogChange {
                    mode: RefLog::AndReference,
                    force_create_reflog: true,
                    message: "setup".into(),
                },
                expected: PreviousValue::Any,
                new: Target::Object(oid),
            },
            name: name.try_into().unwrap(),
            deref: false,
        })
        .unwrap();
    }

    #[test]
    fn apply_moves_a_branch_to_the_new_tip() {
        let s = scratch();
        let mut commits = three_commits(&s.repo);
        let old_tip = parse_oid(&commits.last().unwrap().original.id).unwrap();
        set_ref(&s.repo, "refs/heads/main", old_tip);

        crate::model::bump(
            &mut commits,
            0,
            crate::model::Target::Both,
            datetime::Component::Hour,
            2,
            true,
        );
        let name: gix::refs::FullName = "refs/heads/main".try_into().unwrap();
        let report = apply(&s.repo, &commits, old_tip, &RefTarget::Branch(name)).unwrap();

        assert_eq!(report.count, 3);
        assert_eq!(report.old_tip, old_tip);
        assert_ne!(report.new_tip, old_tip);
        // The branch now points at the new tip.
        let branch_tip = s
            .repo
            .find_reference("refs/heads/main")
            .unwrap()
            .id()
            .detach();
        assert_eq!(branch_tip, report.new_tip);
    }

    #[test]
    fn apply_moves_detached_head() {
        let s = scratch();
        let mut commits = three_commits(&s.repo);
        let old_tip = parse_oid(&commits.last().unwrap().original.id).unwrap();
        set_ref(&s.repo, "HEAD", old_tip);

        let new_time = datetime::parse_in_offset("2024-01-01 09:00", 0).unwrap();
        crate::model::set(&mut commits, 2, crate::model::Target::Both, new_time, false);
        let report = apply(&s.repo, &commits, old_tip, &RefTarget::Detached).unwrap();

        assert_eq!(report.count, 1);
        let head_id = s.repo.head_id().unwrap().detach();
        assert_eq!(head_id, report.new_tip);
    }

    #[test]
    fn apply_noop_leaves_the_ref() {
        let s = scratch();
        let commits = three_commits(&s.repo);
        let old_tip = parse_oid(&commits.last().unwrap().original.id).unwrap();
        set_ref(&s.repo, "refs/heads/main", old_tip);
        let name: gix::refs::FullName = "refs/heads/main".try_into().unwrap();
        let report = apply(&s.repo, &commits, old_tip, &RefTarget::Branch(name)).unwrap();
        assert_eq!(report.count, 0);
        assert_eq!(report.new_tip, old_tip);
        let branch_tip = s
            .repo
            .find_reference("refs/heads/main")
            .unwrap()
            .id()
            .detach();
        assert_eq!(branch_tip, old_tip);
    }
}
