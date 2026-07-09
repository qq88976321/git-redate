//! The library error type.
//!
//! Kept independent of gix and the terminal: gix and I/O errors are
//! converted to strings at the `repo`/`rewrite` boundary so this enum
//! stays a stable, typed surface. The binary wraps these with
//! `anyhow` context at the top level.

/// Errors that abort a redate run.
#[derive(Debug, thiserror::Error)]
pub enum RedateError {
    /// The current directory is not inside a git working tree.
    #[error("not a git repository (or any parent): {0}")]
    NotARepo(String),

    /// A revision (commit or range endpoint) could not be resolved.
    #[error("could not resolve revision '{0}'")]
    BadRevspec(String),

    /// The selected range contains a merge commit (more than one
    /// parent), which v1 does not rewrite.
    #[error(
        "merge commit {0} is in the selected range; \
         git-redate v1 supports linear history only"
    )]
    MergeInRange(String),

    /// The resolved range selected no commits.
    #[error("no commits selected to redate")]
    EmptyRange,

    /// HEAD does not point at a commit yet (fresh repository).
    #[error("HEAD is unborn (no commits yet)")]
    UnbornHead,

    /// An `A..B` range was missing one of its endpoints.
    #[error("range '{0}' needs both endpoints, as in A..B")]
    BadRange(String),

    /// The range's lower boundary is not an ancestor of the tip.
    #[error("'{0}' is not an ancestor of HEAD, so it cannot bound the range")]
    NotAnAncestor(String),

    /// The range does not end at HEAD, which v1 cannot rewrite safely.
    #[error("git-redate v1 can only rewrite a range that ends at HEAD")]
    TipNotHead,

    /// The interactive editor was requested without a terminal.
    #[error("git-redate needs an interactive terminal (use --dry-run in scripts)")]
    NotATty,

    /// Writing objects or moving the ref failed.
    #[error("failed to rewrite history: {0}")]
    Write(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_message_includes_the_oid() {
        let e = RedateError::MergeInRange("abc1234".into());
        assert!(e.to_string().contains("abc1234"));
        assert!(e.to_string().contains("linear history"));
    }

    #[test]
    fn bad_revspec_quotes_the_spec() {
        let e = RedateError::BadRevspec("nope~3".into());
        assert_eq!(e.to_string(), "could not resolve revision 'nope~3'");
    }
}
