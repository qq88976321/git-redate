# git-redate

A modern rewrite of [PotatoLabs/git-redate](https://github.com/PotatoLabs/git-redate):
interactively edit the author and committer dates of a range of commits
from a terminal UI, then rewrite history in place. Written in Rust,
powered by [gitoxide](https://github.com/GitoxideLabs/gitoxide) - no
shelling out to `git`.

## Why

The original `git-redate` opens a plain text file in `$EDITOR` and
rewrites dates with `git filter-branch`. This rewrite keeps the idea but
makes it:

- **Interactive** - a real terminal UI: navigate commits, adjust any
  date field in place with calendar-correct carry, type absolute dates,
  and spread commits evenly.
- **Cascading** - a "shift" mode where editing one commit moves it and
  every newer commit by the same delta, preserving the gaps.
- **Fast and self-contained** - a single Rust binary that rewrites
  commit objects directly with gitoxide (no `git`/`libgit2` at runtime).
- **Safe** - a reflog entry and a printed `git reset --hard` undo
  command on every write, plus a `--dry-run` preview.

## Install

```
cargo install --path .
```

This builds a `git-redate` binary; with it on your `PATH`, git runs
`git redate ...` as a subcommand. Requires Rust 1.85+ (set by gitoxide).

## Usage

```
git redate [<revspec>] [-n <N>] [--root] [--dry-run] [--separate] [--mode <single|shift>] [--no-sign]
```

### Choosing commits

| Invocation | Edits |
|------------|-------|
| `git redate <commit>` | `<commit>..HEAD` - commits *after* `<commit>` (exclusive), following git's `A..B` convention |
| `git redate` | the last `N` commits (default 10, `-n` to change) |
| `git redate A..B` | the range `A..B` (`A` required and excluded; empty `B` = HEAD) |
| `git redate --root` | the entire history, including the first commit |

To edit `<commit>` itself, pass its parent (`git redate <commit>~1`); for
the first commit, use `--root`. The range must end at HEAD, and a range
containing a merge commit is refused (linear history only, in v1).

### Key bindings

| Key | Action |
|-----|--------|
| `up`/`down`, `k`/`j` | select a commit; in an expanded row, step between the author and committer lines |
| `left`/`right`, `h`/`l` | move between date fields |
| `/`, `n` / `N` | search commits by summary or hash; jump to next / previous match |
| `+`/`-`, `shift+up`/`shift+down`, `ctrl-a`/`ctrl-x` | adjust the focused field (calendar carry) |
| `e` / `Enter` | type an absolute date (`YYYY-MM-DD HH:MM`) |
| `Space` | expand a row to edit author/committer (and offsets) separately; `up`/`down` then step between the two lines |
| `Tab` / `shift-Tab`, `s` | toggle single / shift mode |
| `c` | copy the previous (older) commit's time |
| `=` | distribute the middle commits evenly (first and last fixed) |
| `u` / `U` | reset the selected commit / reset all commits (confirm) |
| `ctrl-z` / `ctrl-r` | undo / redo the last edit |
| `?` / `F1` | help overlay (`Esc` inside it returns to the editor) |
| `w` / `W` | write changes (confirm / force) |
| `q` / `Q` / `Esc` | quit (confirm / force);  `Ctrl-C` aborts |

Bindings follow common TUI conventions (lazygit, gitui, tig, vim). `w`
and `q`/`Esc` ask for confirmation when there are unsaved edits; the
uppercase `W`/`Q` skip the prompt. The `/` search prompt supports
readline-style line editing (`ctrl-a`/`ctrl-e`, `ctrl-w`, `ctrl-u`,
`ctrl-k`, and word motion with `ctrl-left`/`ctrl-right`).

One date is applied to both the author and committer by default: every
edit in the collapsed view (typing, `+`/`-`, and the shift cascade)
keeps the two equal. A commit whose author and committer originally
differ (as a rebase or amend leaves them) is unified to a single date
the moment you edit it here, so tools that show the author date (`git
log`, GitHub) and tools that show the committer date (GitLab's web UI,
tig) all display the value you set. To keep or introduce a difference,
expand the row with `Space` and edit the author and committer lines
separately. Each commit keeps its original UTC offset; the wall-clock
time you edit is interpreted in that offset.

### Edit modes

- **single** - edits only the selected commit.
- **shift** - edits the selected commit and every newer commit by the
  same delta, preserving the gaps. Starting from `01:01, 01:02, 02:00`,
  changing the first to `02:01` yields `02:01, 02:02, 03:00`.

Toggle live with `s`. The startup default is resolved as
`--mode` flag > `git config redate.mode` > built-in `single`:

```
git config redate.mode shift    # make shift the default
```

### Dry run

`--dry-run` writes nothing. In a terminal it opens the editor and prints
the planned changes; in a script (no TTY) it previews the selected range
- useful for validating a range or catching a merge.

## Safety and undo

Rewriting changes commit ids, but git-redate keeps it recoverable:

```
git-redate: rewrote 4 commit(s)
  old tip: 1a2b3c4...
  new tip: 9f8e7d6...
  undo with: git reset --hard 1a2b3c4...
```

The branch/HEAD move also writes a reflog entry (`git reflog`,
`branch@{1}`). Only dates change - file trees are untouched, so
uncommitted changes are preserved.

## Signatures

A commit's signature covers its dates, so changing a date invalidates
it. git-redate **re-signs** any commit that was originally signed, using
your repository's signing config (`gpg.format`, `user.signingkey`) - the
same SSH or OpenPGP key `git commit -S` would use. `git log
--show-signature` on the rewritten commits stays Good.

- Signing runs after the editor exits, so a gpg/ssh passphrase prompt
  works normally. If signing fails (no key, locked agent), the whole
  rewrite aborts and nothing is written.
- `--no-sign` drops signatures instead (the rewritten commits become
  unsigned).
- SSH and OpenPGP are supported; x509/gpgsm is not (use `--no-sign`).

## Limitations (v1)

- Linear history only (a merge in the range is refused).
- The range must end at HEAD.
- x509/gpgsm signatures cannot be re-created (use `--no-sign`).

## Development

```
just gate     # fmt --check, clippy -D warnings, test, release build
just test     # unit tests
just build    # debug build
just run -- --dry-run HEAD~5
```

Docs live in `website/` and build with `just site-build` (Zensical).
Releases are cut with `just release` (cargo-release + git-cliff); the
first release is `just release 0.1.0`.

## License

MIT
