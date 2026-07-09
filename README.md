# git-redate

A modern rewrite of [PotatoLabs/git-redate](https://github.com/PotatoLabs/git-redate):
interactively edit the author and committer dates of a range of commits
from a terminal UI, then rewrite history in place. Written in Rust,
powered by [gitoxide](https://github.com/GitoxideLabs/gitoxide) - no
shelling out to `git`.

> Status: under active development toward v0.1.0.

## Why

The original `git-redate` opens a plain text file in `$EDITOR` and
rewrites dates with `git filter-branch`. This rewrite keeps the idea but
makes it:

- **Interactive** - a proper terminal UI: arrow-key navigation, in-place
  increment/decrement of any date component, and a cascade "shift" mode.
- **Fast** - pure Rust, rewriting commit objects directly with gitoxide.
- **Safe** - a reflog entry and a printed undo command on every write,
  plus a `--dry-run` preview.

## Install

```
cargo install --path .
# ensure the resulting `git-redate` binary is on PATH so that
# `git redate` dispatches to it.
```

## Usage

```
git redate [<revspec>] [-n <N>] [--root] [--dry-run] [--separate] [--mode <single|shift>]
```

- `git redate <commit>` - edit the commits in `<commit>..HEAD`
  (exclusive of `<commit>`).
- `git redate` - edit the last `N` commits (default 10, `-n` to change).
- `git redate A..B` - edit the range `A..B`.
- `--root` - include the very first (parentless) commit.
- `--dry-run` - print the planned changes and write nothing.

Full key bindings, edit modes, configuration, and recovery are
documented in the [docs site](website/) and finalized before release.

## License

MIT
