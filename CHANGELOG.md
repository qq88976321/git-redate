# Changelog

All notable changes to this project are documented in this file.
The format is based on Keep a Changelog, and this project adheres to
Conventional Commits.

## [0.1.1] - 2026-08-05

### Features

- **repo**: Refuse to start without a committer identity

### Bug Fixes

- **rewrite**: Give the test scratch repo its own git identity

### Documentation

- Disclose that this is a personal, AI-assisted tool
- Record the identity guard and test hermeticity rule
- **demo**: Script the demo recording with vhs
- **demo**: Record the demo and embed it in README and docs

## [0.1.0] - 2026-08-05

### Features

- **error**: Add RedateError enum
- **datetime**: Jiff-based timestamp math with offset preservation
- **cli**: Clap derive Cli and RangeRequest normalization
- **config**: Resolve edit mode from CLI, git config, default
- **model**: Editable commits and pure edit operations
- **repo**: Gix read side with pure linear walk and merge abort
- **rewrite**: Rebuild commit objects with edited dates and parent remap
- **rewrite**: Move the ref with a reflog entry and report
- **tui**: Editor state machine, keymap, and ratatui rendering
- Wire the binary end-to-end
- **app**: Confirm-on-write/quit and contextual Esc
- **ui**: Render the confirm prompt and refresh the key hints
- **sign**: Produce commit signatures via the configured signer
- **repo**: Read signing config; feat(cli): add --no-sign
- **rewrite**: Re-sign originally-signed commits, abort on failure
- **tui**: Add a restrained Catppuccin Mocha color scheme
- **tui**: Tint unedited timestamps sky, distinct from summaries
- **model**: Add reset_all to restore every commit
- **tui**: Reset all commits via U with a confirm prompt
- **tui**: Undo/redo edits with ctrl-z / ctrl-r
- **lineedit**: Add a readline-style single-line editor
- **tui**: Search commits with / and readline input, cycle with n/N
- **tui**: Priority-ordered footer hints across up to two rows
- **model**: Unify author/committer on linked edits
- **tui**: Up/down navigate author/committer, Tab toggles edit mode
- **cli**: Add --no-retag to leave tags untouched
- **repo**: Scan refs/tags for tags pointing into the range
- **sign**: Detect signature blocks embedded in a tag message
- **rewrite**: Move tags with the branch in one atomic ref transaction
- **model**: Count tags moved by the pending edits
- **tui**: Show pending tag moves in the write confirmation
- Wire tag rewriting through the binary
- **dist**: Add install.sh

### Bug Fixes

- **tui**: Esc closes help and / opens search from help
- **rewrite**: Sign the tag payload git actually verifies
- **repo**: Collapse the tag-chain match arm for clippy 1.97

### Refactor

- **input**: Intuitive keymap grounded in TUI conventions

### Documentation

- Add CLAUDE.md constitution and README skeleton
- Add Zensical documentation site
- Finalize README
- Document the new keymap and signature re-signing
- Document search, undo/redo, reset-all and the two-row footer
- Clarify one-date collapse and the reworked keymap
- Document tag moving and --no-retag
- **site**: Mirror the tag section on the docs site
- Document the one-line install
- Add the release runbook
- Record the distribution decisions

### Testing

- **dist**: Add an offline round trip for install.sh

### Continuous Integration

- Add GitHub Actions workflows (ci, release, pages)
- Shellcheck the shipped shell scripts
- **release**: Ship musl binaries and install.sh as release assets
- **pages**: Let an in-flight deployment finish

### Miscellaneous

- Init project
- Scaffold cargo lib+bin (git-redate) with lints and release profile
- Add dev tooling (justfile, release.toml, cliff.toml, CHANGELOG)
- Add just install recipe
- Add just install-dev recipe for a fast dev-profile install
