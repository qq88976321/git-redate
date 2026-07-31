//! Command-line surface and its normalization into a [`RangeRequest`].
//!
//! `normalize` is pure (string manipulation only): it turns the parsed
//! flags into an abstract range description that `repo` later resolves
//! against a real repository. That split keeps the argument semantics
//! unit-testable without gix.

use crate::error::RedateError;
use clap::{Parser, ValueEnum};
use std::fmt;

/// Startup editing behaviour. `single` edits only the selected commit;
/// `shift` moves the selected commit and every newer one by the same
/// delta (relative gaps preserved). Toggled live with `s`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum EditMode {
    #[default]
    Single,
    Shift,
}

impl fmt::Display for EditMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            EditMode::Single => "single",
            EditMode::Shift => "shift",
        })
    }
}

impl std::str::FromStr for EditMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "single" => Ok(EditMode::Single),
            "shift" => Ok(EditMode::Shift),
            other => Err(format!(
                "unknown edit mode '{other}' (expected single or shift)"
            )),
        }
    }
}

/// Interactively edit git commit dates.
#[derive(Parser, Debug)]
#[command(name = "git-redate", version, about, long_about = None)]
pub struct Cli {
    /// A commit (edits <commit>..HEAD, exclusive) or an A..B range.
    /// Omit to edit the last -n commits.
    pub revspec: Option<String>,

    /// Number of commits back from HEAD when no revspec is given.
    #[arg(short = 'n', long = "number", default_value_t = 10, value_name = "N")]
    pub number: usize,

    /// Include the root commit (edit the entire history to HEAD).
    #[arg(long, conflicts_with = "revspec")]
    pub root: bool,

    /// Print the planned changes and write nothing.
    #[arg(long)]
    pub dry_run: bool,

    /// Start with author/committer rows expanded for separate editing.
    #[arg(long)]
    pub separate: bool,

    /// Startup edit mode; overrides `git config redate.mode`.
    #[arg(long, value_enum, value_name = "single|shift")]
    pub mode: Option<EditMode>,

    /// Drop GPG/SSH signatures instead of re-signing rewritten commits.
    #[arg(long)]
    pub no_sign: bool,

    /// Leave tags pointing at the old commits instead of moving them.
    #[arg(long)]
    pub no_retag: bool,
}

/// An abstract range to resolve against a repository. Exactly one of
/// `boundary` / `limit` bounds the walk (or neither, with
/// `include_root`, to take the whole history).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeRequest {
    /// Revision to resolve as the newest end of the range.
    pub tip: String,
    /// Exclusive lower bound; `None` means bound by `limit`/root.
    pub boundary: Option<String>,
    /// Max commits to take when there is no `boundary`.
    pub limit: Option<usize>,
    /// Walk down to and include the parentless root commit.
    pub include_root: bool,
}

/// Turn the parsed CLI into an abstract [`RangeRequest`].
pub fn normalize(cli: &Cli) -> Result<RangeRequest, RedateError> {
    // `--root` (guaranteed by clap not to co-occur with a revspec):
    // edit the entire history, root included.
    if cli.root {
        return Ok(RangeRequest {
            tip: "HEAD".to_string(),
            boundary: None,
            limit: None,
            include_root: true,
        });
    }

    match cli.revspec.as_deref() {
        // No revspec: the last N commits from HEAD.
        None => Ok(RangeRequest {
            tip: "HEAD".to_string(),
            boundary: None,
            limit: Some(cli.number),
            include_root: false,
        }),
        Some(spec) if spec.contains("..") => parse_range(spec),
        // A single commit is the exclusive boundary of <commit>..HEAD.
        Some(spec) => Ok(RangeRequest {
            tip: "HEAD".to_string(),
            boundary: Some(spec.to_string()),
            limit: None,
            include_root: false,
        }),
    }
}

/// Parse an `A..B` range. `A` is the exclusive boundary and is
/// required; an empty `B` means HEAD. The three-dot form `A...B`
/// (symmetric difference) is not supported in v1.
fn parse_range(spec: &str) -> Result<RangeRequest, RedateError> {
    if spec.contains("...") {
        return Err(RedateError::BadRange(spec.to_string()));
    }
    let (a, b) = spec
        .split_once("..")
        .ok_or_else(|| RedateError::BadRange(spec.to_string()))?;
    if a.is_empty() {
        return Err(RedateError::BadRange(spec.to_string()));
    }
    let tip = if b.is_empty() { "HEAD" } else { b };
    Ok(RangeRequest {
        tip: tip.to_string(),
        boundary: Some(a.to_string()),
        limit: None,
        include_root: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Cli {
        // Prepend the binary name that clap expects at argv[0].
        let mut full = vec!["git-redate"];
        full.extend_from_slice(args);
        Cli::try_parse_from(full).expect("args should parse")
    }

    #[test]
    fn no_args_takes_last_ten() {
        let r = normalize(&cli(&[])).unwrap();
        assert_eq!(
            r,
            RangeRequest {
                tip: "HEAD".into(),
                boundary: None,
                limit: Some(10),
                include_root: false,
            }
        );
    }

    #[test]
    fn number_flag_sets_the_limit() {
        let r = normalize(&cli(&["-n", "3"])).unwrap();
        assert_eq!(r.limit, Some(3));
        assert_eq!(r.boundary, None);
    }

    #[test]
    fn single_commit_is_exclusive_boundary_to_head() {
        let r = normalize(&cli(&["abc123"])).unwrap();
        assert_eq!(r.tip, "HEAD");
        assert_eq!(r.boundary.as_deref(), Some("abc123"));
        assert_eq!(r.limit, None);
    }

    #[test]
    fn range_splits_on_two_dots() {
        let r = normalize(&cli(&["v1.0..HEAD~1"])).unwrap();
        assert_eq!(r.boundary.as_deref(), Some("v1.0"));
        assert_eq!(r.tip, "HEAD~1");
        assert_eq!(r.limit, None);
    }

    #[test]
    fn range_empty_tip_defaults_to_head() {
        let r = normalize(&cli(&["abc.."])).unwrap();
        assert_eq!(r.boundary.as_deref(), Some("abc"));
        assert_eq!(r.tip, "HEAD");
    }

    #[test]
    fn range_empty_boundary_is_rejected() {
        assert!(matches!(
            normalize(&cli(&["..HEAD"])),
            Err(RedateError::BadRange(_))
        ));
    }

    #[test]
    fn three_dot_range_is_rejected() {
        assert!(matches!(
            normalize(&cli(&["a...b"])),
            Err(RedateError::BadRange(_))
        ));
    }

    #[test]
    fn root_takes_whole_history() {
        let r = normalize(&cli(&["--root"])).unwrap();
        assert_eq!(
            r,
            RangeRequest {
                tip: "HEAD".into(),
                boundary: None,
                limit: None,
                include_root: true,
            }
        );
    }

    #[test]
    fn root_conflicts_with_revspec() {
        let parsed = Cli::try_parse_from(["git-redate", "--root", "abc123"]);
        assert!(parsed.is_err());
    }

    #[test]
    fn mode_flag_parses_and_is_optional() {
        assert_eq!(cli(&[]).mode, None);
        assert_eq!(cli(&["--mode", "shift"]).mode, Some(EditMode::Shift));
        assert_eq!(cli(&["--mode", "single"]).mode, Some(EditMode::Single));
    }

    #[test]
    fn edit_mode_from_str_and_display_round_trip() {
        assert_eq!("shift".parse::<EditMode>().unwrap(), EditMode::Shift);
        assert_eq!("SINGLE".parse::<EditMode>().unwrap(), EditMode::Single);
        assert!("bogus".parse::<EditMode>().is_err());
        assert_eq!(EditMode::Shift.to_string(), "shift");
        assert_eq!(EditMode::default(), EditMode::Single);
    }

    #[test]
    fn dry_run_and_separate_flags() {
        let c = cli(&["--dry-run", "--separate"]);
        assert!(c.dry_run);
        assert!(c.separate);
    }

    #[test]
    fn no_retag_flag() {
        assert!(!cli(&[]).no_retag);
        assert!(cli(&["--no-retag"]).no_retag);
    }
}
