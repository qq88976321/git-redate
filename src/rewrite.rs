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
use crate::repo::{PlannedTag, RefTarget};
use crate::sign::Signer;
use gix::bstr::ByteSlice;
use gix::objs::WriteTo;
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
    /// Commits that were re-signed.
    pub resigned: usize,
    /// Commits whose signature was dropped (`--no-sign`).
    pub dropped_signatures: usize,
    /// Tags moved onto the rewritten commits.
    pub moved_tags: Vec<MovedTag>,
}

/// What happened to a moved tag's signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagSig {
    /// The tag had no signature (or is lightweight).
    Unsigned,
    /// Originally signed; re-signed for the new target.
    Resigned,
    /// Originally signed; signature dropped (`--no-sign`).
    Dropped,
}

/// A tag moved onto the rewritten history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedTag {
    /// Short tag name ("v1.0").
    pub name: String,
    /// Old ref target (tag object or commit), for manual restore.
    pub old: ObjectId,
    /// New ref target.
    pub new: ObjectId,
    /// Signature outcome.
    pub sig: TagSig,
}

/// Rewrite the edited commits and move the current branch/HEAD to the
/// new tip, writing a reflog entry. Tags in `tags` whose commit was
/// rewritten move with it, in the same atomic ref transaction as the
/// branch. Originally-signed commits and tags are re-signed with
/// `signer` (or dropped when `signer` is `None`, i.e. `--no-sign`).
/// A no-op (nothing changed) leaves every ref untouched. Not called
/// for `--dry-run` (which writes nothing).
pub fn apply(
    repo: &gix::Repository,
    commits: &[EditableCommit],
    old_tip: ObjectId,
    ref_target: &RefTarget,
    tags: &[PlannedTag],
    signer: Option<&Signer>,
) -> Result<RewriteReport, RedateError> {
    let rewritten = write_rewritten(repo, commits, signer)?;
    let mut moved_tags = Vec::new();
    if rewritten.count > 0 {
        // Rebuild (and re-sign) tag objects first: a signing failure
        // aborts before any ref has moved.
        let (tag_edits, moved) = retag(repo, tags, &rewritten.map, signer)?;
        moved_tags = moved;
        let branch = branch_edit(ref_target, old_tip, rewritten.new_tip, rewritten.count)?;
        repo.edit_references(std::iter::once(branch).chain(tag_edits))
            .map_err(write_err)?;
    }
    Ok(RewriteReport {
        old_tip,
        new_tip: rewritten.new_tip,
        count: rewritten.count,
        resigned: rewritten.resigned,
        dropped_signatures: rewritten.dropped_signatures,
        moved_tags,
    })
}

/// The edit pointing the branch (or detached HEAD) at `new_tip`,
/// writing a reflog entry and asserting it still points at `old_tip`.
fn branch_edit(
    ref_target: &RefTarget,
    old_tip: ObjectId,
    new_tip: ObjectId,
    count: usize,
) -> Result<gix::refs::transaction::RefEdit, RedateError> {
    use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};
    use gix::refs::Target;

    let name: gix::refs::FullName = match ref_target {
        RefTarget::Branch(n) => n.clone(),
        RefTarget::Detached => "HEAD".try_into().map_err(write_err)?,
    };
    let message = format!("redate: rewrote {count} commit(s)");
    Ok(RefEdit {
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
}

/// Build the ref edits moving each planned tag whose commit was
/// rewritten, rebuilding annotated tag objects on the way. Tags whose
/// commit kept its id (before the first change) are left alone.
fn retag(
    repo: &gix::Repository,
    tags: &[PlannedTag],
    map: &HashMap<ObjectId, ObjectId>,
    signer: Option<&Signer>,
) -> Result<(Vec<gix::refs::transaction::RefEdit>, Vec<MovedTag>), RedateError> {
    use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};
    use gix::refs::Target;

    let mut edits = Vec::new();
    let mut moved = Vec::new();
    for t in tags {
        let Some(&new_commit) = map.get(&t.commit) else {
            continue;
        };
        let (new_target, sig) = if t.annotated {
            rebuild_tag(repo, t, new_commit, signer)?
        } else {
            (new_commit, TagSig::Unsigned)
        };
        edits.push(RefEdit {
            change: Change::Update {
                log: LogChange {
                    mode: RefLog::AndReference,
                    // git does not reflog tags by default; do not force one.
                    force_create_reflog: false,
                    message: "redate: moved to rewritten commit".into(),
                },
                expected: PreviousValue::MustExistAndMatch(Target::Object(t.ref_oid)),
                new: Target::Object(new_target),
            },
            name: t.full_name.clone(),
            deref: false,
        });
        moved.push(MovedTag {
            name: t.short.clone(),
            old: t.ref_oid,
            new: new_target,
            sig,
        });
    }
    Ok((edits, moved))
}

/// Rebuild an annotated tag object onto `new_commit`, keeping name,
/// tagger, and message verbatim; the signature is re-created (or
/// dropped under `--no-sign`), mirroring commit re-signing.
fn rebuild_tag(
    repo: &gix::Repository,
    t: &PlannedTag,
    new_commit: ObjectId,
    signer: Option<&Signer>,
) -> Result<(ObjectId, TagSig), RedateError> {
    let orig = repo.find_tag(t.ref_oid).map_err(write_err)?;
    let decoded = orig.decode().map_err(write_err)?;
    let mut tag = gix::objs::Tag::try_from(decoded).map_err(write_err)?;
    tag.target = new_commit;
    let mut had_sig = tag.pgp_signature.take().is_some();
    // gix splits only PGP signature blocks out of the message; an SSH
    // (or other) block stays embedded and must be stripped so the stale
    // signature is not carried into the rebuilt tag. The newline that
    // ended the message itself is kept, as git writes it.
    if let Some(pos) = crate::sign::embedded_signature(tag.message.as_ref()) {
        tag.message.truncate(pos);
        had_sig = true;
    }

    let sig = if had_sig {
        match signer {
            Some(s) => {
                // git signs the tag bytes that precede the signature -
                // the message-terminating newline included. gix writes
                // that newline itself when a signature is present, so
                // take it off the message and add it to the payload,
                // leaving the signed bytes exactly what git verifies.
                if tag.message.ends_with(b"\n") {
                    let without_nl = tag.message.len() - 1;
                    tag.message.truncate(without_nl);
                }
                let mut payload = Vec::with_capacity(tag.size() as usize + 1);
                tag.write_to(&mut payload)
                    .map_err(|e| RedateError::Write(e.to_string()))?;
                payload.push(b'\n');
                let armored = s
                    .sign(&payload)
                    .map_err(|e| RedateError::Signing(e.to_string()))?;
                tag.pgp_signature = Some(armored.into());
                TagSig::Resigned
            }
            None => TagSig::Dropped,
        }
    } else {
        TagSig::Unsigned
    };
    let new_oid = repo.write_object(&tag).map_err(write_err)?.detach();
    Ok((new_oid, sig))
}

/// Result of rebuilding the range's commit objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewritten {
    /// New tip oid (equals the old tip if nothing changed).
    pub new_tip: ObjectId,
    /// Number of commits actually rebuilt.
    pub count: usize,
    /// How many rebuilt commits were re-signed.
    pub resigned: usize,
    /// How many rebuilt commits had their signature dropped.
    pub dropped_signatures: usize,
    /// old oid -> new oid, for the rebuilt commits.
    pub map: HashMap<ObjectId, ObjectId>,
}

/// Rebuild and write the edited commits (oldest-first), returning the
/// new tip and the old->new id map. Originally-signed commits are
/// re-signed with `signer`, or have their signature dropped when
/// `signer` is `None`. Writes objects to the odb but does not move any
/// ref (see [`crate::rewrite::apply`]).
pub fn write_rewritten(
    repo: &gix::Repository,
    commits: &[EditableCommit],
    signer: Option<&Signer>,
) -> Result<Rewritten, RedateError> {
    let tip_oid = parse_oid(&commits.last().ok_or(RedateError::EmptyRange)?.original.id)?;

    let Some(first) = commits.iter().position(EditableCommit::changed) else {
        // No edits: the tip is unchanged.
        return Ok(Rewritten {
            new_tip: tip_oid,
            count: 0,
            resigned: 0,
            dropped_signatures: 0,
            map: HashMap::new(),
        });
    };

    let mut map: HashMap<ObjectId, ObjectId> = HashMap::new();
    let mut resigned = 0;
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

        // Carry over extra headers except the now-invalid signature; it
        // is re-created below (or dropped under --no-sign).
        let mut extra = Vec::new();
        let mut had_sig = false;
        for &(k, ref v) in &decoded.extra_headers {
            if k == b"gpgsig".as_bstr() {
                had_sig = true;
                continue;
            }
            extra.push((k.to_owned(), v.clone().into_owned()));
        }

        let mut commit = gix::objs::Commit {
            tree: orig.tree_id().map_err(write_err)?.detach(),
            parents: parents.into_iter().collect(),
            author,
            committer,
            encoding: decoded.encoding.map(|e| e.to_owned()),
            message: decoded.message.to_owned(),
            extra_headers: extra,
        };

        if had_sig {
            match signer {
                Some(s) => {
                    // Sign the payload as serialized WITHOUT gpgsig
                    // (exactly what git signs), then store the armored
                    // result as the gpgsig header (appended last).
                    let mut payload = Vec::with_capacity(commit.size() as usize);
                    commit
                        .write_to(&mut payload)
                        .map_err(|e| RedateError::Write(e.to_string()))?;
                    let armored = s
                        .sign(&payload)
                        .map_err(|e| RedateError::Signing(e.to_string()))?;
                    commit
                        .extra_headers
                        .push((b"gpgsig".as_bstr().to_owned(), armored.into()));
                    resigned += 1;
                }
                None => dropped += 1,
            }
        }

        let new_oid = repo.write_object(&commit).map_err(write_err)?.detach();
        map.insert(old_oid, new_oid);
        new_tip = new_oid;
    }

    Ok(Rewritten {
        new_tip,
        count: commits.len() - first,
        resigned,
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
        let out = write_rewritten(&s.repo, &commits, None).unwrap();
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

        let out = write_rewritten(&s.repo, &commits, None).unwrap();
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
        let out = write_rewritten(&s.repo, &commits, None).unwrap();
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
        let report = apply(
            &s.repo,
            &commits,
            old_tip,
            &RefTarget::Branch(name),
            &[],
            None,
        )
        .unwrap();

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
        let report = apply(&s.repo, &commits, old_tip, &RefTarget::Detached, &[], None).unwrap();

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
        let report = apply(
            &s.repo,
            &commits,
            old_tip,
            &RefTarget::Branch(name),
            &[],
            None,
        )
        .unwrap();
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

    // ---- tag scan ----

    /// The pristine `Commit`s of an editable range, as `repo::load`
    /// would have returned them.
    fn originals(commits: &[EditableCommit]) -> Vec<Commit> {
        commits.iter().map(|e| e.original.clone()).collect()
    }

    /// Write an annotated tag object pointing at `target`.
    fn write_tag_object(
        repo: &gix::Repository,
        name: &str,
        target: ObjectId,
        target_kind: gix::objs::Kind,
        signature: Option<&[u8]>,
    ) -> ObjectId {
        let tag = gix::objs::Tag {
            target,
            target_kind,
            name: name.into(),
            tagger: Some(gix::actor::Signature {
                name: "Tagger".into(),
                email: "tag@example.com".into(),
                time: gix::date::Time::new(1_700_000_000, 0),
            }),
            message: "a tag".into(),
            pgp_signature: signature.map(|s| s.into()),
        };
        repo.write_object(&tag).unwrap().detach()
    }

    #[test]
    fn scan_finds_lightweight_and_annotated_tags() {
        let s = scratch();
        let commits = three_commits(&s.repo);
        let c0 = parse_oid(&commits[0].original.id).unwrap();
        let c2 = parse_oid(&commits[2].original.id).unwrap();
        set_ref(&s.repo, "refs/tags/light", c0);
        let tag_oid = write_tag_object(&s.repo, "annot", c2, gix::objs::Kind::Commit, None);
        set_ref(&s.repo, "refs/tags/annot", tag_oid);

        let scan = crate::repo::tags_in_range(&s.repo, &originals(&commits)).unwrap();
        assert!(scan.skipped.is_empty());
        assert_eq!(scan.tags.len(), 2);
        let annot = scan.tags.iter().find(|t| t.short == "annot").unwrap();
        assert!(annot.annotated);
        assert!(!annot.signed);
        assert_eq!(annot.ref_oid, tag_oid);
        assert_eq!(annot.commit, c2);
        assert_eq!(annot.commit_index, 2);
        let light = scan.tags.iter().find(|t| t.short == "light").unwrap();
        assert!(!light.annotated);
        assert_eq!(light.ref_oid, c0);
        assert_eq!(light.commit, c0);
        assert_eq!(light.commit_index, 0);
    }

    #[test]
    fn scan_ignores_tags_outside_range() {
        let s = scratch();
        let commits = three_commits(&s.repo);
        let tip = parse_oid(&commits[2].original.id).unwrap();
        let tree = empty_tree(&s.repo);
        let outside = write_test_commit(&s.repo, tree, vec![tip], 1_800_000_000, "outside");
        set_ref(&s.repo, "refs/tags/outside", outside);

        let scan = crate::repo::tags_in_range(&s.repo, &originals(&commits)).unwrap();
        assert!(scan.tags.is_empty());
        assert!(scan.skipped.is_empty());
    }

    #[test]
    fn scan_skips_tag_of_tag_with_warning() {
        let s = scratch();
        let commits = three_commits(&s.repo);
        let c1 = parse_oid(&commits[1].original.id).unwrap();
        let inner = write_tag_object(&s.repo, "inner", c1, gix::objs::Kind::Commit, None);
        set_ref(&s.repo, "refs/tags/inner", inner);
        let outer = write_tag_object(&s.repo, "outer", inner, gix::objs::Kind::Tag, None);
        set_ref(&s.repo, "refs/tags/outer", outer);

        let scan = crate::repo::tags_in_range(&s.repo, &originals(&commits)).unwrap();
        assert_eq!(scan.tags.len(), 1);
        assert_eq!(scan.tags[0].short, "inner");
        assert_eq!(scan.skipped.len(), 1);
        assert!(scan.skipped[0].contains("outer"));
    }

    // ---- tag rewriting ----

    /// Write a signed annotated tag byte-for-byte the way git does: the
    /// signed payload is everything up to and including the newline that
    /// ends the message, and the armored signature follows it.
    fn write_signed_tag(
        repo: &gix::Repository,
        signer: &Signer,
        name: &str,
        target: ObjectId,
    ) -> ObjectId {
        let mut tag = gix::objs::Tag {
            target,
            target_kind: gix::objs::Kind::Commit,
            name: name.into(),
            tagger: Some(gix::actor::Signature {
                name: "Tagger".into(),
                email: "tag@example.com".into(),
                time: gix::date::Time::new(1_700_000_000, 0),
            }),
            message: "a tag".into(),
            pgp_signature: None,
        };
        let mut payload = Vec::new();
        tag.write_to(&mut payload).unwrap();
        payload.push(b'\n');
        let armored = signer.sign(&payload).unwrap();
        tag.pgp_signature = Some(armored.into());
        repo.write_object(&tag).unwrap().detach()
    }

    /// Verify an object's embedded SSH signature the way git does: split
    /// the raw object at the signature block, then check the signature
    /// over everything before it (namespace `git`). Returns false when
    /// the signature does not cover exactly those bytes.
    fn ssh_signature_verifies(repo: &gix::Repository, oid: ObjectId, signer: &Signer) -> bool {
        use std::process::{Command, Stdio};
        let data = repo.find_object(oid).unwrap().data.clone();
        let pos = crate::sign::embedded_signature(&data).expect("object carries a signature");
        let (payload, signature) = data.split_at(pos);

        let dir = std::path::Path::new(&signer.key).parent().unwrap();
        let sig_file = dir.join("object.sig");
        std::fs::write(&sig_file, signature).unwrap();
        // allowed_signers takes `principal keytype keydata`; the public
        // key file carries a trailing comment, so keep the first two
        // fields only.
        let pub_key = std::fs::read_to_string(format!("{}.pub", signer.key)).unwrap();
        let mut fields = pub_key.split_whitespace();
        let (ktype, kdata) = (fields.next().unwrap(), fields.next().unwrap());
        let allowed = dir.join("allowed_signers");
        std::fs::write(&allowed, format!("redate@test {ktype} {kdata}\n")).unwrap();

        let mut child = Command::new("ssh-keygen")
            .arg("-Y")
            .arg("verify")
            .arg("-f")
            .arg(&allowed)
            .args(["-I", "redate@test", "-n", "git", "-s"])
            .arg(&sig_file)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::io::Write::write_all(child.stdin.as_mut().unwrap(), payload).unwrap();
        child.wait().unwrap().success()
    }

    #[test]
    fn apply_retargets_a_lightweight_tag() {
        let s = scratch();
        let mut commits = three_commits(&s.repo);
        let old_tip = parse_oid(&commits.last().unwrap().original.id).unwrap();
        set_ref(&s.repo, "refs/heads/main", old_tip);
        let c1 = parse_oid(&commits[1].original.id).unwrap();
        set_ref(&s.repo, "refs/tags/v1", c1);

        let new_time = datetime::parse_in_offset("2024-01-01 05:00", 0).unwrap();
        crate::model::set(&mut commits, 1, crate::model::Target::Both, new_time, false);
        let scan = crate::repo::tags_in_range(&s.repo, &originals(&commits)).unwrap();
        let name: gix::refs::FullName = "refs/heads/main".try_into().unwrap();
        let report = apply(
            &s.repo,
            &commits,
            old_tip,
            &RefTarget::Branch(name),
            &scan.tags,
            None,
        )
        .unwrap();

        assert_eq!(report.moved_tags.len(), 1);
        let m = &report.moved_tags[0];
        assert_eq!(m.name, "v1");
        assert_eq!(m.old, c1);
        assert_ne!(m.new, c1);
        assert_eq!(m.sig, TagSig::Unsigned);
        // The ref moved to the rewritten commit (the new tip's parent).
        let tag_ref = s.repo.find_reference("refs/tags/v1").unwrap().id().detach();
        assert_eq!(tag_ref, m.new);
        let new_tip = s.repo.find_commit(report.new_tip).unwrap();
        assert_eq!(new_tip.parent_ids().next().unwrap().detach(), m.new);
    }

    #[test]
    fn apply_rebuilds_an_annotated_tag_keeping_tagger_and_message() {
        let s = scratch();
        let mut commits = three_commits(&s.repo);
        let old_tip = parse_oid(&commits.last().unwrap().original.id).unwrap();
        set_ref(&s.repo, "refs/heads/main", old_tip);
        let c2 = parse_oid(&commits[2].original.id).unwrap();
        let tag_oid = write_tag_object(&s.repo, "v2", c2, gix::objs::Kind::Commit, None);
        set_ref(&s.repo, "refs/tags/v2", tag_oid);

        let new_time = datetime::parse_in_offset("2024-01-01 09:00", 0).unwrap();
        crate::model::set(&mut commits, 2, crate::model::Target::Both, new_time, false);
        let scan = crate::repo::tags_in_range(&s.repo, &originals(&commits)).unwrap();
        let name: gix::refs::FullName = "refs/heads/main".try_into().unwrap();
        let report = apply(
            &s.repo,
            &commits,
            old_tip,
            &RefTarget::Branch(name),
            &scan.tags,
            None,
        )
        .unwrap();

        let m = &report.moved_tags[0];
        assert_ne!(m.new, tag_oid);
        let new_tag = s.repo.find_tag(m.new).unwrap();
        let d = new_tag.decode().unwrap();
        assert_eq!(d.name, "v2");
        assert_eq!(d.target(), report.new_tip);
        assert_eq!(d.message, "a tag");
        // The tagger line is carried over verbatim (name, email, time).
        assert_eq!(
            d.tagger.unwrap(),
            "Tagger <tag@example.com> 1700000000 +0000"
        );
        assert!(d.pgp_signature.is_none());
        let tag_ref = s.repo.find_reference("refs/tags/v2").unwrap().id().detach();
        assert_eq!(tag_ref, m.new);
    }

    #[test]
    fn apply_leaves_tags_before_the_first_change() {
        let s = scratch();
        let mut commits = three_commits(&s.repo);
        let old_tip = parse_oid(&commits.last().unwrap().original.id).unwrap();
        set_ref(&s.repo, "refs/heads/main", old_tip);
        let c0 = parse_oid(&commits[0].original.id).unwrap();
        set_ref(&s.repo, "refs/tags/v0", c0);

        // Editing commit 1 rewrites 1 and 2; commit 0 keeps its id.
        let new_time = datetime::parse_in_offset("2024-01-01 05:00", 0).unwrap();
        crate::model::set(&mut commits, 1, crate::model::Target::Both, new_time, false);
        let scan = crate::repo::tags_in_range(&s.repo, &originals(&commits)).unwrap();
        assert_eq!(scan.tags.len(), 1);
        let name: gix::refs::FullName = "refs/heads/main".try_into().unwrap();
        let report = apply(
            &s.repo,
            &commits,
            old_tip,
            &RefTarget::Branch(name),
            &scan.tags,
            None,
        )
        .unwrap();

        assert!(report.moved_tags.is_empty());
        let tag_ref = s.repo.find_reference("refs/tags/v0").unwrap().id().detach();
        assert_eq!(tag_ref, c0);
    }

    #[test]
    fn apply_noop_moves_no_tags() {
        let s = scratch();
        let commits = three_commits(&s.repo);
        let old_tip = parse_oid(&commits.last().unwrap().original.id).unwrap();
        set_ref(&s.repo, "refs/heads/main", old_tip);
        set_ref(&s.repo, "refs/tags/v1", old_tip);

        let scan = crate::repo::tags_in_range(&s.repo, &originals(&commits)).unwrap();
        let name: gix::refs::FullName = "refs/heads/main".try_into().unwrap();
        let report = apply(
            &s.repo,
            &commits,
            old_tip,
            &RefTarget::Branch(name),
            &scan.tags,
            None,
        )
        .unwrap();

        assert_eq!(report.count, 0);
        assert!(report.moved_tags.is_empty());
        let tag_ref = s.repo.find_reference("refs/tags/v1").unwrap().id().detach();
        assert_eq!(tag_ref, old_tip);
    }

    #[test]
    fn resigned_tag_carries_a_fresh_ssh_signature() {
        let s = scratch();
        let Some(signer) = ephemeral_ssh_signer(&s.dir.join("keys")) else {
            eprintln!("skipping: ssh-keygen not available");
            return;
        };
        let mut commits = three_commits(&s.repo);
        let old_tip = parse_oid(&commits.last().unwrap().original.id).unwrap();
        set_ref(&s.repo, "refs/heads/main", old_tip);
        let c2 = parse_oid(&commits[2].original.id).unwrap();
        let tag_oid = write_signed_tag(&s.repo, &signer, "vsig", c2);
        set_ref(&s.repo, "refs/tags/vsig", tag_oid);

        let new_time = datetime::parse_in_offset("2024-01-01 09:00", 0).unwrap();
        crate::model::set(&mut commits, 2, crate::model::Target::Both, new_time, false);
        let scan = crate::repo::tags_in_range(&s.repo, &originals(&commits)).unwrap();
        assert!(scan.tags[0].signed);
        let name: gix::refs::FullName = "refs/heads/main".try_into().unwrap();
        let report = apply(
            &s.repo,
            &commits,
            old_tip,
            &RefTarget::Branch(name),
            &scan.tags,
            Some(&signer),
        )
        .unwrap();

        let m = &report.moved_tags[0];
        assert_eq!(m.sig, TagSig::Resigned);
        // The commits were unsigned; the counters stay commit-only.
        assert_eq!(report.resigned, 0);
        let new_tag = s.repo.find_tag(m.new).unwrap();
        let d = new_tag.decode().unwrap();
        // gix parses SSH signature blocks back into the message; find
        // the fresh signature there and the original message before it.
        let pos =
            crate::sign::embedded_signature(d.message).expect("re-signed tag keeps a signature");
        assert!(d.message[pos..].starts_with(b"-----BEGIN SSH SIGNATURE-----"));
        assert_eq!(&d.message[..pos - 1], "a tag");
        assert_eq!(d.target(), report.new_tip);
        // The signature must cover exactly the bytes before it, or git
        // rejects the tag ("incorrect signature").
        assert!(
            ssh_signature_verifies(&s.repo, tag_oid, &signer),
            "the fixture must be signed the way git signs"
        );
        assert!(
            ssh_signature_verifies(&s.repo, m.new, &signer),
            "the re-signed tag must verify over its payload"
        );
    }

    #[test]
    fn no_sign_drops_the_tag_signature() {
        let s = scratch();
        let Some(signer) = ephemeral_ssh_signer(&s.dir.join("keys")) else {
            eprintln!("skipping: ssh-keygen not available");
            return;
        };
        let mut commits = three_commits(&s.repo);
        let old_tip = parse_oid(&commits.last().unwrap().original.id).unwrap();
        set_ref(&s.repo, "refs/heads/main", old_tip);
        let c2 = parse_oid(&commits[2].original.id).unwrap();
        let tag_oid = write_signed_tag(&s.repo, &signer, "vsig", c2);
        set_ref(&s.repo, "refs/tags/vsig", tag_oid);

        let new_time = datetime::parse_in_offset("2024-01-01 09:00", 0).unwrap();
        crate::model::set(&mut commits, 2, crate::model::Target::Both, new_time, false);
        let scan = crate::repo::tags_in_range(&s.repo, &originals(&commits)).unwrap();
        let name: gix::refs::FullName = "refs/heads/main".try_into().unwrap();
        let report = apply(
            &s.repo,
            &commits,
            old_tip,
            &RefTarget::Branch(name),
            &scan.tags,
            None,
        )
        .unwrap();

        let m = &report.moved_tags[0];
        assert_eq!(m.sig, TagSig::Dropped);
        assert_eq!(report.dropped_signatures, 0);
        let new_tag = s.repo.find_tag(m.new).unwrap();
        let d = new_tag.decode().unwrap();
        assert!(d.pgp_signature.is_none());
        // The stale signature was stripped, not carried in the message,
        // and the message keeps the newline git terminates it with.
        assert_eq!(d.message, "a tag\n");
    }

    #[test]
    fn tag_signing_failure_aborts_before_any_ref_moves() {
        let s = scratch();
        let Some(signer) = ephemeral_ssh_signer(&s.dir.join("keys")) else {
            eprintln!("skipping: ssh-keygen not available");
            return;
        };
        let mut commits = three_commits(&s.repo);
        let old_tip = parse_oid(&commits.last().unwrap().original.id).unwrap();
        set_ref(&s.repo, "refs/heads/main", old_tip);
        let c2 = parse_oid(&commits[2].original.id).unwrap();
        let tag_oid = write_signed_tag(&s.repo, &signer, "vsig", c2);
        set_ref(&s.repo, "refs/tags/vsig", tag_oid);

        let new_time = datetime::parse_in_offset("2024-01-01 09:00", 0).unwrap();
        crate::model::set(&mut commits, 2, crate::model::Target::Both, new_time, false);
        let scan = crate::repo::tags_in_range(&s.repo, &originals(&commits)).unwrap();
        // The commits are unsigned, so only the tag re-sign invokes the
        // (broken) signer.
        let bad = Signer {
            format: crate::sign::SignFormat::Ssh,
            key: signer.key.clone(),
            program: "no-such-signer-binary".into(),
        };
        let name: gix::refs::FullName = "refs/heads/main".try_into().unwrap();
        let err = apply(
            &s.repo,
            &commits,
            old_tip,
            &RefTarget::Branch(name),
            &scan.tags,
            Some(&bad),
        )
        .unwrap_err();

        assert!(matches!(err, RedateError::Signing(_)));
        // Neither the branch nor the tag moved.
        let branch = s
            .repo
            .find_reference("refs/heads/main")
            .unwrap()
            .id()
            .detach();
        assert_eq!(branch, old_tip);
        let tag_ref = s
            .repo
            .find_reference("refs/tags/vsig")
            .unwrap()
            .id()
            .detach();
        assert_eq!(tag_ref, tag_oid);
    }

    // ---- re-signing ----

    fn ephemeral_ssh_signer(keydir: &std::path::Path) -> Option<Signer> {
        use std::process::{Command, Stdio};
        std::fs::create_dir_all(keydir).ok()?;
        let key = keydir.join("id");
        let status = Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-N", "", "-C", "redate@test", "-f"])
            .arg(&key)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()?;
        if !status.success() {
            return None;
        }
        Some(Signer {
            format: crate::sign::SignFormat::Ssh,
            key: key.to_string_lossy().into_owned(),
            program: "ssh-keygen".into(),
        })
    }

    fn write_signed_commit(
        repo: &gix::Repository,
        signer: &Signer,
        tree: ObjectId,
        secs: i64,
    ) -> ObjectId {
        let sig = gix::actor::Signature {
            name: "Tester".into(),
            email: "test@example.com".into(),
            time: gix::date::Time::new(secs, 0),
        };
        let mut commit = gix::objs::Commit {
            tree,
            parents: Vec::new().into_iter().collect(),
            author: sig.clone(),
            committer: sig,
            encoding: None,
            message: "signed".into(),
            extra_headers: Vec::new(),
        };
        let mut payload = Vec::new();
        commit.write_to(&mut payload).unwrap();
        let armored = signer.sign(&payload).unwrap();
        commit
            .extra_headers
            .push((b"gpgsig".as_bstr().to_owned(), armored.into()));
        repo.write_object(&commit).unwrap().detach()
    }

    fn has_gpgsig(repo: &gix::Repository, oid: ObjectId) -> Option<Vec<u8>> {
        let c = repo.find_commit(oid).unwrap();
        let decoded = c.decode().unwrap();
        decoded
            .extra_headers
            .iter()
            .find(|(k, _)| *k == b"gpgsig".as_bstr())
            .map(|(_, v)| v.to_vec())
    }

    fn editable_from(oid: ObjectId, secs: i64) -> EditableCommit {
        let stamp = Stamp::new(secs, 0);
        EditableCommit::new(crate::model::Commit {
            id: oid.to_string(),
            short_id: oid.to_string()[..7].to_string(),
            summary: "signed".into(),
            author: stamp,
            committer: stamp,
        })
    }

    #[test]
    fn resign_keeps_a_valid_signature_and_no_sign_drops_it() {
        let s = scratch();
        let Some(signer) = ephemeral_ssh_signer(&s.dir.join("keys")) else {
            eprintln!("skipping: ssh-keygen not available");
            return;
        };
        let tree = empty_tree(&s.repo);
        let base = datetime::parse_in_offset("2024-01-01 01:00", 0)
            .unwrap()
            .seconds;
        let oid = write_signed_commit(&s.repo, &signer, tree, base);
        assert!(
            has_gpgsig(&s.repo, oid).is_some(),
            "fixture should be signed"
        );

        // Re-sign: edit the date, rewrite with the signer.
        let mut commits = vec![editable_from(oid, base)];
        crate::model::bump(
            &mut commits,
            0,
            crate::model::Target::Both,
            datetime::Component::Hour,
            1,
            false,
        );
        let out = write_rewritten(&s.repo, &commits, Some(&signer)).unwrap();
        assert_eq!(out.resigned, 1);
        assert_eq!(out.dropped_signatures, 0);
        let sig = has_gpgsig(&s.repo, out.new_tip).expect("re-signed commit keeps a gpgsig");
        assert!(sig.starts_with(b"-----BEGIN SSH SIGNATURE-----"));

        // --no-sign: same edit, signature dropped.
        let mut commits2 = vec![editable_from(oid, base)];
        crate::model::bump(
            &mut commits2,
            0,
            crate::model::Target::Both,
            datetime::Component::Hour,
            1,
            false,
        );
        let out2 = write_rewritten(&s.repo, &commits2, None).unwrap();
        assert_eq!(out2.dropped_signatures, 1);
        assert_eq!(out2.resigned, 0);
        assert!(has_gpgsig(&s.repo, out2.new_tip).is_none());
    }
}
