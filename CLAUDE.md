# git-redate - repo constitution for agents

A modern rewrite of PotatoLabs/git-redate: a Rust CLI, invoked as
`git redate <commit>`, that opens a terminal UI for editing the
author/committer dates of a range of commits and rewrites history with
gitoxide (no shelling out to git). User-facing usage lives in
README.md; this file is the development contract.

Workspace-wide procedure (routing, checklists, commit rules) comes from
`../../agent-os/INDEX.md`. This file only adds repo-specific facts.

## Quick facts

- Not yet published: no git remote wired. Development on branch
  `master` (cargo-release `allow-branch = ["master"]`). First release
  is cut with `just release 0.1.0` so the first tag is v0.1.0 (explicit
  version, not a patch bump). Releases are USER-ONLY.
- Toolchain: `cargo`/rustc 1.94 locally. Rust edition 2021, but
  MSRV = 1.85 (gitoxide gix 0.85 sets the floor). The CI msrv job stays
  commented until a 1.85 build is verified.
- Dependencies (do not add more without user confirmation):
  gix, ratatui, clap (derive), jiff, anyhow, thiserror. Dev tools
  installed out-of-band (NOT Cargo deps): cargo-release, git-cliff
  (CHANGELOG.md, config in cliff.toml), Zensical (docs, via uvx).

## Deliberate divergences from herdr-copy-search conventions

The sibling project herdr-copy-search is the template for the tooling,
but this crate differs on purpose:

- CLI parsing uses `clap` derive (herdr hand-rolls argv parsing with no
  clap dep) - the richer flag surface (revspec, -n, --root, --dry-run,
  --separate, --mode, --no-sign, --no-retag) warrants it.
- `jiff` is added for calendar-correct datetime + fixed-offset math
  (tzdb feature off; all editing is in each commit's own offset).
- `crossterm` is used only via `ratatui::crossterm` (version locked to
  ratatui's backend), not as a direct dependency.
- MSRV is 1.85, not 1.74, because of gix.

## Commands

```
just gate      # THE quality gate: fmt --check, clippy -D warnings,
               #   cargo test, cargo build --release. Run before every
               #   commit; all four must pass.
just build     # debug build
just test      # unit tests only
just run -- ARGS  # run the binary against the current repo, e.g.
                  #   `just run -- --dry-run HEAD~5`
just release LEVEL # cargo-release: gate, bump, regen CHANGELOG.md,
                   #   commit, tag. USER-ONLY. First release: 0.1.0.
just site-build / site-serve  # Zensical docs site (website/)
```

## Module map (src/) - lib + thin bin

- main.rs     entrypoint: parse CLI, open repo, resolve config, tty
              guard, load range, run TUI loop (or --dry-run), rewrite,
              print report; anyhow context + ExitCode
- cli.rs      clap `Cli` + normalize() -> RangeRequest (pure)
- config.rs   EffectiveConfig: built-in default <- git config
              (redate.*) <- CLI (pure resolver)
- error.rs    thiserror RedateError
- datetime.rs jiff parse/format/increment/delta; preserves offset
- model.rs    Commit / EditableCommit; pure edit ops (single/shift
              cascade, distribute, copy-from-previous, reset);
              moved_tag_count
- repo.rs     gix read: open, config_snapshot, resolve revspec/A..B/
              --root, linear first-parent walk, merge abort,
              tags_in_range -> TagScan (PlannedTag + skipped warnings)
- rewrite.rs  gix write: rebuild+write commit objects with parent
              remap, re-sign or drop signatures, rebuild/retarget tags,
              move branch + tags in ONE ref transaction + reflog,
              dry-run, RewriteReport (incl. moved_tags)
- sign.rs     produce a signature by shelling out to gpg / ssh-keygen
              (SignFormat, Signer::sign, embedded_signature); no crypto
              in-crate
- app.rs      TUI state machine (App, Mode incl. Confirm, edit_mode);
              pure handle, no I/O
- input.rs    KeyEvent -> Action (pure, context-aware keymap)
- ui.rs       ratatui rendering + panic-safe TerminalGuard

Pure logic (datetime, model, cli::normalize, config, walk_linear,
remap_parents, sign format/argv helpers) is kept independent of gix and
the terminal so it can be unit-tested directly.

## Design decisions (see docs/superpowers spec / plan for rationale)

- Range: `git redate <commit>` = `<commit>..HEAD`, exclusive of
  <commit> (git `A..B` semantics). Bare `git redate` = last N (default
  10, `-n`). `A..B` supported. `--root` includes the parentless commit.
- Timestamp: one date applied to both author + committer by default -
  EVERY edit in the collapsed view (set/bump/cascade) collapses committer
  onto author (value + offset), so an originally-differing pair is
  unified when edited. Space expands a row to edit author/committer (and
  offsets) separately; that is the only way to keep/introduce a gap.
- Timezone: each commit's original offset is preserved; the edited
  wall-clock time is interpreted in that offset.
- Edit modes: `single` (only the selected commit) and `shift` (edit a
  commit -> that commit and all NEWER commits move by the same delta,
  relative gaps preserved). Toggle with `s`; startup default from
  `git config redate.mode` (else single), overridable by `--mode`.
- Merge commits: v1 aborts if the range contains any merge (linear
  history only).
- Safety: reflog written on ref move; old tip SHA printed for undo;
  `--dry-run` writes nothing; a dirty worktree is only a notice (trees
  are unchanged, so uncommitted changes are preserved).
- Signatures: a date change invalidates a commit's signature, so
  originally-signed commits are RE-signed via the repo's signing config
  (SSH/OpenPGP, through gpg/ssh-keygen). Signing failure aborts the
  rewrite; `--no-sign` drops signatures; x509/gpgsm is unsupported.
  Payloads differ by object: a commit's is its serialization without the
  gpgsig header; a tag's ends with the newline that terminates the
  message (gix writes that newline itself, so rewrite.rs strips it from
  the message and appends it to the signed bytes) - get this wrong and
  git reports "incorrect signature".
- Tags: tags pointing into the range move onto the rewritten commits by
  default (`--no-retag` opts out). Lightweight = retarget, annotated =
  rebuild keeping name/tagger/message, signed = re-signed. A tag moves
  iff its commit index >= the first changed index, so the count is
  computed live for the write-confirm line. Branch + tags move in one
  atomic edit_references transaction (MustExistAndMatch each), after all
  tag objects are written, so a signing failure moves nothing; no
  reflog is forced for tag refs. Tag-of-tag chains and symbolic tag refs
  are warned about on stderr and left alone. `git reset --hard` does not
  restore tags, so the report prints each tag's old id plus the
  `git update-ref` to undo it.
- Keymap follows TUI conventions: Space expand, up/down (j/k) move
  between commits and, in an expanded row, step author<->committer,
  Tab/Shift-Tab (also s) toggle single/shift mode, u reset (U reset-all,
  confirmed), +/- and ctrl-a/ctrl-x adjust, ctrl-z/ctrl-r undo/redo,
  / search (readline input, n/N cycle), w/W write (confirm/force),
  q/Q/Esc quit (confirm/force), Ctrl-C abort. The footer packs tiered
  hints into up to two rows; the ? help panel is the full list and Esc
  closes it.

## Manual / driving tests

The interactive TUI needs a real TTY; headless `cargo run` cannot
exercise it. `--dry-run` works headless (no TUI) and is the CI-friendly
path. Unit tests (`just test`) cover the pure logic and thin gix I/O
against scratch repositories built in a tempdir (no dev-dependencies,
mirroring herdr). To drive the real TUI, build and run it in a throwaway
git repository in a real terminal (see README / plan verification).

## Conventions

- Conventional Commits, ASCII-only edits, MIT license.
- Commit scopes follow the module/area: feat(datetime), feat(repo),
  feat(rewrite), feat(tui), feat(cli), feat(config), chore, ci, docs.
- Commit after each coherent, self-contained change; never batch
  unrelated changes.
- NOTE: `git add -A` fails here (the sandbox injects character-device
  dotfiles in the repo root); always stage explicit paths.
