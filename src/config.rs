//! Effective settings, resolved by precedence.
//!
//! The only configurable setting in v1 is the startup edit mode:
//!
//! `--mode` flag  >  `git config redate.mode`  >  built-in default.
//!
//! This resolver is pure: the caller fetches the raw `redate.mode`
//! string from the repository (gix `config_snapshot`) and passes it in,
//! so the precedence logic is testable without a repository. An
//! unparseable git config value is ignored (with a warning) rather than
//! aborting the run.

use crate::cli::EditMode;

/// The resolved settings plus any non-fatal note to surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveConfig {
    pub mode: EditMode,
    /// A warning to show the user (e.g. an ignored bad config value).
    pub warning: Option<String>,
}

/// Resolve the startup edit mode. `cli_mode` is the `--mode` flag (if
/// given); `git_mode` is the raw `redate.mode` git config value (if
/// set). Precedence: CLI, then git config, then the built-in default.
pub fn resolve(cli_mode: Option<EditMode>, git_mode: Option<&str>) -> EffectiveConfig {
    if let Some(mode) = cli_mode {
        return EffectiveConfig {
            mode,
            warning: None,
        };
    }
    match git_mode {
        Some(raw) => match raw.parse::<EditMode>() {
            Ok(mode) => EffectiveConfig {
                mode,
                warning: None,
            },
            Err(_) => EffectiveConfig {
                mode: EditMode::default(),
                warning: Some(format!(
                    "ignoring invalid redate.mode '{}' (expected single or shift)",
                    raw.trim()
                )),
            },
        },
        None => EffectiveConfig {
            mode: EditMode::default(),
            warning: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_default_is_single() {
        let c = resolve(None, None);
        assert_eq!(c.mode, EditMode::Single);
        assert_eq!(c.warning, None);
    }

    #[test]
    fn git_config_sets_the_mode() {
        assert_eq!(resolve(None, Some("shift")).mode, EditMode::Shift);
        assert_eq!(resolve(None, Some("single")).mode, EditMode::Single);
    }

    #[test]
    fn cli_flag_overrides_git_config() {
        let c = resolve(Some(EditMode::Single), Some("shift"));
        assert_eq!(c.mode, EditMode::Single);
        assert_eq!(c.warning, None);
    }

    #[test]
    fn invalid_git_config_falls_back_with_warning() {
        let c = resolve(None, Some("nope"));
        assert_eq!(c.mode, EditMode::Single);
        assert!(c.warning.as_deref().unwrap().contains("nope"));
    }

    #[test]
    fn cli_flag_wins_even_over_invalid_git_config() {
        let c = resolve(Some(EditMode::Shift), Some("nope"));
        assert_eq!(c.mode, EditMode::Shift);
        assert_eq!(c.warning, None);
    }
}
