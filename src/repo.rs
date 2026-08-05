//! Reading side: open a repository, resolve the requested range, and
//! snapshot the commits to edit.
//!
//! The linear walk ([`walk_linear`]) is a pure function over a
//! parent-lookup closure, so its boundary / limit / merge-abort logic
//! is unit-tested with an in-memory graph and no gix. The gix-specific
//! parts (rev-parse, reading signatures) are thin wrappers around it.

use crate::cli::RangeRequest;
use crate::datetime::Stamp;
use crate::error::RedateError;
use crate::model::Commit;
use crate::sign::{SignFormat, Signer};
use gix::ObjectId;
use std::collections::HashMap;

/// Where the rewritten tip should be written back.
pub enum RefTarget {
    /// The branch HEAD points at.
    Branch(gix::refs::FullName),
    /// Detached HEAD.
    Detached,
}

/// The commits to edit plus the state needed to write them back.
pub struct Loaded {
    /// Commits oldest-first (index 0 = oldest).
    pub commits: Vec<Commit>,
    /// Current tip (HEAD commit) before the rewrite.
    pub old_tip: ObjectId,
    /// The ref to move after rewriting.
    pub ref_target: RefTarget,
    /// Whether the working tree has uncommitted changes (a notice only;
    /// trees are unchanged so the changes are preserved regardless).
    pub dirty: bool,
}

/// A tag under `refs/tags/*` whose peeled commit is inside the loaded
/// range; the rewrite moves it onto the rewritten commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedTag {
    /// Full ref name ("refs/tags/v1.0").
    pub full_name: gix::refs::FullName,
    /// Short display name ("v1.0").
    pub short: String,
    /// What the ref points at directly: the tag object (annotated) or
    /// the commit itself (lightweight).
    pub ref_oid: ObjectId,
    /// The peeled commit inside the range.
    pub commit: ObjectId,
    /// Index of that commit in the oldest-first range.
    pub commit_index: usize,
    /// Annotated tag (a tag object) rather than a lightweight ref.
    pub annotated: bool,
    /// Annotated and carrying a PGP/SSH signature.
    pub signed: bool,
}

/// Outcome of scanning `refs/tags/*` against the loaded range.
#[derive(Debug, Default)]
pub struct TagScan {
    /// Tags that move with the commits they point at.
    pub tags: Vec<PlannedTag>,
    /// Warnings for tags left untouched (tag-of-tag chains, symbolic
    /// refs, unreadable objects).
    pub skipped: Vec<String>,
}

/// Scan `refs/tags/*` for tags pointing at commits of the loaded range
/// (`commits` oldest-first). Tag-of-tag chains and symbolic refs that
/// would be affected are reported in `skipped` and left alone; tags
/// pointing outside the range are ignored silently.
pub fn tags_in_range(repo: &gix::Repository, commits: &[Commit]) -> Result<TagScan, RedateError> {
    let mut index: HashMap<ObjectId, usize> = HashMap::new();
    for (i, c) in commits.iter().enumerate() {
        let oid =
            ObjectId::from_hex(c.id.as_bytes()).map_err(|e| RedateError::Write(e.to_string()))?;
        index.insert(oid, i);
    }

    let mut scan = TagScan::default();
    // Collect the refs first: classifying reads objects, which must not
    // fight the iterator's borrow of the packed-refs buffer.
    let platform = repo
        .references()
        .map_err(|e| RedateError::Write(e.to_string()))?;
    let iter = platform
        .tags()
        .map_err(|e| RedateError::Write(e.to_string()))?;
    let mut refs = Vec::new();
    for r in iter {
        match r {
            Ok(r) => refs.push(r.detach()),
            Err(e) => scan
                .skipped
                .push(format!("skipping an unreadable tag ref ({e})")),
        }
    }

    for r in refs {
        let short = r.name.shorten().to_string();
        let ref_oid = match r.target {
            gix::refs::Target::Object(oid) => oid,
            gix::refs::Target::Symbolic(_) => {
                scan.skipped
                    .push(format!("tag '{short}' is a symbolic ref; leaving it alone"));
                continue;
            }
        };
        let obj = match repo.find_object(ref_oid) {
            Ok(obj) => obj,
            Err(e) => {
                scan.skipped.push(format!(
                    "tag '{short}' points at an unreadable object; leaving it alone ({e})"
                ));
                continue;
            }
        };
        match obj.kind {
            gix::object::Kind::Commit => {
                if let Some(&i) = index.get(&ref_oid) {
                    scan.tags.push(PlannedTag {
                        full_name: r.name,
                        short,
                        ref_oid,
                        commit: ref_oid,
                        commit_index: i,
                        annotated: false,
                        signed: false,
                    });
                }
            }
            gix::object::Kind::Tag => {
                let decoded = obj.try_into_tag().map_err(|e| e.to_string()).and_then(|t| {
                    t.decode()
                        .map(|d| {
                            // gix splits only PGP signature blocks out of
                            // the message; SSH blocks stay embedded in it.
                            let signed = d.pgp_signature.is_some()
                                || crate::sign::embedded_signature(d.message).is_some();
                            (d.target(), d.target_kind, signed)
                        })
                        .map_err(|e| e.to_string())
                });
                let (target, target_kind, signed) = match decoded {
                    Ok(v) => v,
                    Err(e) => {
                        scan.skipped.push(format!(
                            "tag '{short}' cannot be decoded; leaving it alone ({e})"
                        ));
                        continue;
                    }
                };
                match target_kind {
                    gix::object::Kind::Commit => {
                        if let Some(&i) = index.get(&target) {
                            scan.tags.push(PlannedTag {
                                full_name: r.name,
                                short,
                                ref_oid,
                                commit: target,
                                commit_index: i,
                                annotated: true,
                                signed,
                            });
                        }
                    }
                    // A tag-of-tag chain: only worth a warning when it
                    // actually peels into the range. A chain that does not
                    // falls through to the catch-all arm below, which is
                    // what an untaken `if` inside this arm did before.
                    gix::object::Kind::Tag
                        if peel_tag_chain(repo, target)
                            .is_some_and(|commit| index.contains_key(&commit)) =>
                    {
                        scan.skipped.push(format!(
                            "tag '{short}' points at another tag; \
                             tag chains are not rewritten, leaving it alone"
                        ));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(scan)
}

/// Follow a tag-of-tag chain (bounded) down to the commit it ultimately
/// points at, if any.
fn peel_tag_chain(repo: &gix::Repository, mut oid: ObjectId) -> Option<ObjectId> {
    for _ in 0..10 {
        let obj = repo.find_object(oid).ok()?;
        match obj.kind {
            gix::object::Kind::Commit => return Some(oid),
            gix::object::Kind::Tag => {
                oid = obj.try_into_tag().ok()?.target_id().ok()?.detach();
            }
            _ => return None,
        }
    }
    None
}

/// Discover the repository containing the current directory.
pub fn open() -> Result<gix::Repository, RedateError> {
    gix::discover(".").map_err(|e| RedateError::NotARepo(e.to_string()))
}

/// The raw `redate.mode` git config value, if set.
pub fn config_edit_mode(repo: &gix::Repository) -> Option<String> {
    repo.config_snapshot()
        .string("redate.mode")
        .map(|v| v.to_string())
}

/// The signer implied by the repository's git config (`gpg.format`,
/// `user.signingkey`, and the per-format program), used to re-sign
/// rewritten commits that were originally signed.
pub fn signing_config(repo: &gix::Repository) -> Signer {
    let cfg = repo.config_snapshot();
    let get = |key: &str| cfg.string(key).map(|v| v.to_string());

    let format = SignFormat::parse(&get("gpg.format").unwrap_or_default());
    let key = get("user.signingkey").unwrap_or_default();
    let program = match format {
        SignFormat::Ssh => get("gpg.ssh.program").unwrap_or_else(|| "ssh-keygen".to_string()),
        SignFormat::X509 => get("gpg.x509.program").unwrap_or_else(|| "gpgsm".to_string()),
        SignFormat::OpenPgp => get("gpg.program").unwrap_or_else(|| "gpg".to_string()),
    };
    Signer {
        format,
        key,
        program: program.into(),
    }
}

/// Resolve the requested range against `repo` and snapshot its commits.
pub fn load(repo: &gix::Repository, range: &RangeRequest) -> Result<Loaded, RedateError> {
    let head = repo
        .head()
        .map_err(|e| RedateError::NotARepo(e.to_string()))?;
    let old_tip = head.id().ok_or(RedateError::UnbornHead)?.detach();
    let ref_target = match head.referent_name() {
        Some(name) => RefTarget::Branch(name.to_owned()),
        None => RefTarget::Detached,
    };

    // v1 always moves the current branch/HEAD, so the range must end at
    // the checked-out tip.
    let tip = resolve(repo, &range.tip)?;
    if tip != old_tip {
        return Err(RedateError::TipNotHead);
    }
    let boundary = match &range.boundary {
        Some(spec) => Some(resolve(repo, spec)?),
        None => None,
    };

    let oids = walk_linear(tip, boundary, range.limit, |oid| parents_of(repo, oid)).map_err(
        |e| match e {
            WalkError::Empty => RedateError::EmptyRange,
            WalkError::Merge(oid) => RedateError::MergeInRange(short_hex(oid)),
            WalkError::BoundaryNotAncestor(_) => {
                RedateError::NotAnAncestor(range.boundary.clone().unwrap_or_default())
            }
        },
    )?;

    // walk_linear yields newest-first; the model wants oldest-first.
    let mut commits = Vec::with_capacity(oids.len());
    for oid in oids.iter().rev() {
        commits.push(read_commit(repo, *oid)?);
    }

    let dirty = repo.is_dirty().unwrap_or(false);

    Ok(Loaded {
        commits,
        old_tip,
        ref_target,
        dirty,
    })
}

fn resolve(repo: &gix::Repository, spec: &str) -> Result<ObjectId, RedateError> {
    repo.rev_parse_single(spec)
        .map(|id| id.detach())
        .map_err(|_| RedateError::BadRevspec(spec.to_string()))
}

fn parents_of(repo: &gix::Repository, oid: ObjectId) -> Vec<ObjectId> {
    repo.find_commit(oid)
        .map(|c| c.parent_ids().map(|id| id.detach()).collect())
        .unwrap_or_default()
}

fn read_commit(repo: &gix::Repository, oid: ObjectId) -> Result<Commit, RedateError> {
    let commit = repo
        .find_commit(oid)
        .map_err(|e| RedateError::Write(e.to_string()))?;
    let author = signature_stamp(commit.author())?;
    let committer = signature_stamp(commit.committer())?;
    let summary = commit
        .message()
        .map(|m| m.summary().to_string())
        .unwrap_or_default();
    let full = oid.to_string();
    let short = full.chars().take(7).collect();
    Ok(Commit {
        id: full,
        short_id: short,
        summary,
        author,
        committer,
    })
}

fn signature_stamp<E: std::fmt::Display>(
    sig: Result<gix::actor::SignatureRef<'_>, E>,
) -> Result<Stamp, RedateError> {
    let sig = sig.map_err(|e| RedateError::Write(e.to_string()))?;
    let time = sig.time().map_err(|e| RedateError::Write(e.to_string()))?;
    Ok(Stamp::new(time.seconds, time.offset))
}

fn short_hex(oid: ObjectId) -> String {
    oid.to_string().chars().take(10).collect()
}

/// Error from the linear walk.
#[derive(Debug, PartialEq, Eq)]
pub enum WalkError<Id> {
    /// A commit with more than one parent was reached.
    Merge(Id),
    /// The walk selected no commits.
    Empty,
    /// A boundary was given but never reached (not an ancestor).
    BoundaryNotAncestor(Id),
}

/// Walk from `tip` toward its first parent, returning the selected oids
/// newest-first. Stops before `boundary` (exclusive), at `limit`
/// commits, or at the parentless root. Aborts on a merge (>1 parent).
///
/// `parents` returns a commit's parent ids (empty for the root).
pub fn walk_linear<Id, F>(
    tip: Id,
    boundary: Option<Id>,
    limit: Option<usize>,
    mut parents: F,
) -> Result<Vec<Id>, WalkError<Id>>
where
    Id: Copy + Eq,
    F: FnMut(Id) -> Vec<Id>,
{
    let mut out = Vec::new();
    let mut cur = tip;
    let mut hit_boundary = false;
    loop {
        if boundary == Some(cur) {
            hit_boundary = true;
            break;
        }
        let ps = parents(cur);
        if ps.len() > 1 {
            return Err(WalkError::Merge(cur));
        }
        out.push(cur);
        if let Some(lim) = limit {
            if out.len() >= lim {
                break;
            }
        }
        match ps.first() {
            Some(&p) => cur = p,
            None => break, // reached the root (already pushed)
        }
    }
    if let Some(b) = boundary {
        if !hit_boundary {
            return Err(WalkError::BoundaryNotAncestor(b));
        }
    }
    if out.is_empty() {
        return Err(WalkError::Empty);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Build a parent lookup for a linear chain n -> n-1 -> ... -> 0,
    /// where 0 is the root. Higher number = newer.
    fn linear(parents: &[(i32, Vec<i32>)]) -> impl Fn(i32) -> Vec<i32> + '_ {
        let map: HashMap<i32, Vec<i32>> = parents.iter().cloned().collect();
        move |id| map.get(&id).cloned().unwrap_or_default()
    }

    // Chain: 3 -> 2 -> 1 -> 0 (root).
    fn chain() -> Vec<(i32, Vec<i32>)> {
        vec![(3, vec![2]), (2, vec![1]), (1, vec![0]), (0, vec![])]
    }

    #[test]
    fn boundary_is_exclusive() {
        let got = walk_linear(3, Some(1), None, linear(&chain())).unwrap();
        assert_eq!(got, vec![3, 2]); // 1 excluded
    }

    #[test]
    fn limit_caps_the_walk() {
        let got = walk_linear(3, None, Some(2), linear(&chain())).unwrap();
        assert_eq!(got, vec![3, 2]);
    }

    #[test]
    fn no_boundary_reaches_and_includes_root() {
        let got = walk_linear(3, None, None, linear(&chain())).unwrap();
        assert_eq!(got, vec![3, 2, 1, 0]);
    }

    #[test]
    fn merge_aborts() {
        let mut c = chain();
        c[1] = (2, vec![1, 9]); // 2 becomes a merge of 1 and 9
        let err = walk_linear(3, None, None, linear(&c)).unwrap_err();
        assert_eq!(err, WalkError::Merge(2));
    }

    #[test]
    fn tip_equal_boundary_is_empty() {
        let err = walk_linear(3, Some(3), None, linear(&chain())).unwrap_err();
        assert_eq!(err, WalkError::Empty);
    }

    #[test]
    fn boundary_not_in_ancestry_is_reported() {
        let err = walk_linear(3, Some(42), None, linear(&chain())).unwrap_err();
        assert_eq!(err, WalkError::BoundaryNotAncestor(42));
    }

    #[test]
    fn single_commit_repo_takes_the_root() {
        let got = walk_linear(0, None, Some(10), linear(&[(0, vec![])])).unwrap();
        assert_eq!(got, vec![0]);
    }

    #[test]
    fn signing_config_reads_ssh_format_and_key() {
        let dir = std::env::temp_dir().join(format!("git-redate-cfg-b-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        gix::init(&dir).unwrap();
        // Append signing config, then reopen to pick it up.
        let cfg = dir.join(".git/config");
        let mut text = std::fs::read_to_string(&cfg).unwrap_or_default();
        text.push_str("[gpg]\n\tformat = ssh\n[user]\n\tsigningkey = /keys/id.pub\n");
        std::fs::write(&cfg, text).unwrap();
        let repo = gix::open(&dir).unwrap();
        let s = signing_config(&repo);
        assert_eq!(s.format, SignFormat::Ssh);
        assert_eq!(s.key, "/keys/id.pub");
        assert_eq!(s.program.to_string_lossy(), "ssh-keygen");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
