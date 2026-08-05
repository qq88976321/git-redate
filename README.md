# git-redate

A modern rewrite of [PotatoLabs/git-redate](https://github.com/PotatoLabs/git-redate):
interactively edit the author and committer dates of a range of commits
from a terminal UI, then rewrite history in place. Written in Rust,
powered by [gitoxide](https://github.com/GitoxideLabs/gitoxide) - no
shelling out to `git`.

Docs site: <https://qq88976321.github.io/git-redate/>.

<!-- github.com will NOT play a repo-relative <video>, and user-attachment
URLs need a manual re-upload on every re-render, so the README embeds a
committed GIF derived from the mp4 (regenerate with `just demo`; see
docs/demo-recording.md). The crisp mp4 plays inline on the docs site. -->
![git-redate demo](demo/redate.gif)

Five commits made between 02:11 and 03:40: shift mode moves the whole
run three hours, single mode nudges the last one on its own, and
expanding it pulls the committer date back from the author date. The
write reports the old tip, the undo command, and the tag that followed
its commit.

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

## Status

Personal tool, built with heavy AI assistance (Claude Code). I review
what ships and use it on my own repositories, but it comes with no
warranty and no support commitment: issues and PRs are welcome and may
still go unanswered.

It rewrites history, so treat it accordingly: preview with `--dry-run`
first, keep the old tip the report prints, and do not point it at a
branch other people have already pulled.

## Install

```
curl -fsSL https://github.com/qq88976321/git-redate/releases/latest/download/install.sh | sh
```

This downloads the prebuilt binary for your platform, checks it against
the release's sha256, and installs it into `~/.local/bin` - no `sudo`
anywhere. With it on your `PATH`, git runs `git redate ...` as a
subcommand.

| Platform | Release asset |
|----------|---------------|
| Linux x86_64 | `git-redate-x86_64-unknown-linux-musl.tar.gz` |
| Linux aarch64 | `git-redate-aarch64-unknown-linux-musl.tar.gz` |
| macOS Intel | `git-redate-x86_64-apple-darwin.tar.gz` |
| macOS Apple silicon | `git-redate-aarch64-apple-darwin.tar.gz` |

The Linux builds are statically linked against musl, so they do not care
which glibc your distribution ships. Windows is not covered - build from
source there.

Rather read the script first, install somewhere else, or pin a version:

```
curl -fsSL https://github.com/qq88976321/git-redate/releases/latest/download/install.sh -o install.sh
less install.sh
sh install.sh --to ~/bin --version v0.1.0
```

Uninstall by deleting the binary (`rm ~/.local/bin/git-redate`).

### From source

```
cargo install --path .
```

Requires Rust 1.85+ (set by gitoxide). No system `git` or `libgit2` is
needed at runtime either way.

## Usage

```
git redate [<revspec>] [-n <N>] [--root] [--dry-run] [--separate] [--mode <single|shift>] [--no-sign] [--no-retag]
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
- useful for validating a range or catching a merge. Tags that would
follow the rewrite are listed with their current target:

```
  tag v1.0 would move (currently 7c34e6f...)
```

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

## Tags

A rewritten commit gets a new id, so a tag pointing at it would be left
behind on the old object - from the branch the tag looks like it
disappeared and `git describe` breaks. git-redate **moves those tags
onto the rewritten commits** by default (as `git filter-repo` does):

```
  moved tag v1.0: 7c34e6f... -> 5a87320...
  moved tag v1.1: d3ebba8... -> a8b277b...  (re-signed)
```

- A lightweight tag is retargeted; an annotated tag is rebuilt onto the
  new commit, keeping its name, tagger, and message.
- Only tags whose commit is actually rewritten move (everything from the
  first edited commit onward). Tags outside the range are untouched.
- The branch and all tags move in one atomic ref transaction, each
  guarded against a concurrent update.
- Before the editor opens, a note on stderr says how many tags point
  into the range; the write confirmation repeats the count.
- **Undo:** `git reset --hard <old tip>` restores the branch but *not*
  tags. Each report line carries the tag's old id, so restore one with
  `git update-ref refs/tags/<name> <old id>`.
- `--no-retag` leaves every tag pointing at the old commits.

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
- A signed annotated tag that moves is re-signed the same way, so
  `git verify-tag` stays Good; under `--no-sign` its signature is
  dropped and the report says so. Tag signing failure also aborts the
  rewrite, before any ref has moved.
- SSH and OpenPGP are supported; x509/gpgsm is not (use `--no-sign`).

## Limitations (v1)

- Linear history only (a merge in the range is refused).
- The range must end at HEAD.
- x509/gpgsm signatures cannot be re-created (use `--no-sign`).
- A tag pointing at another tag is not rewritten (warned about on
  stderr and left alone), and tags are never pushed for you.
- A committer identity must be configured (`user.name` and
  `user.email`): the reflog entry the rewrite writes needs one. git
  derives one from the system instead; git-redate refuses rather than
  invent an identity, and says so before the editor opens.

## Development

```
just gate     # fmt --check, clippy -D warnings, test, release build
just test     # unit tests
just build    # debug build
just run -- --dry-run HEAD~5
just demo     # re-record the demo (docker + vhs)
```

The demo above is scripted in [demo/redate.tape](demo/redate.tape) as a
vhs tape rendered to mp4, with `just gifs` deriving the README's inline
GIF, so both regenerate after any UI change - see
[docs/demo-recording.md](docs/demo-recording.md).

Docs live in `website/` and build with `just site-build` (Zensical).
Releases are cut with `just release` (cargo-release + git-cliff); the
first release is `just release 0.1.0`.

## License

MIT
