# Usage

```
git redate [<revspec>] [-n <N>] [--root] [--dry-run] [--separate] [--mode <single|shift>]
```

## Choosing the commits

| Invocation | Edits |
|------------|-------|
| `git redate <commit>` | `<commit>..HEAD`, i.e. every commit *after* `<commit>` (exclusive), following git's `A..B` convention |
| `git redate` | the last `N` commits (default 10, set with `-n`) |
| `git redate A..B` | the range `A..B` (`A` is required and excluded; an empty `B` means HEAD) |
| `git redate --root` | the entire history, including the first (parentless) commit |

To also edit `<commit>` itself, pass its parent: `git redate <commit>~1`.
For the very first commit, use `--root`.

The range must end at HEAD, and v1 refuses a range that contains a merge
commit (linear history only).

## The editor

A commit list opens, oldest at the top. Move around and adjust dates:

| Key | Action |
|-----|--------|
| `up` / `down`, `k` / `j` | select a commit |
| `left` / `right`, `h` / `l` | move between date fields (year, month, day, hour, minute) |
| `+` / `-`, `shift+up` / `shift+down` | adjust the focused field (with calendar carry) |
| `e` / `Enter` | type an absolute date (`YYYY-MM-DD HH:MM`) |
| `Tab` | expand the row to edit author and committer (and their offsets) separately |
| `t` | switch between author and committer in an expanded row |
| `s` | toggle single / shift edit mode |
| `c` | copy the previous (older) commit's time onto this one |
| `=` | spread the commits evenly in time between the first and last |
| `d` | reset the selected commit to its original dates |
| `?` | toggle the help overlay |
| `w` | write the changes |
| `q` / `Esc` / `Ctrl-C` | cancel without writing |

By default one date is applied to both the author and committer
timestamps. Each commit keeps its original UTC offset; the wall-clock
time you edit is interpreted in that offset (expand a row with `Tab` to
change the offset).

## Edit modes

- **single** - editing a commit changes only that commit.
- **shift** - editing a commit moves it *and every newer commit* by the
  same delta, preserving the gaps between them.

Example, starting from `01:01, 01:02, 02:00`: in shift mode, changing
the first commit to `02:01` (a +1h delta) yields `02:01, 02:02, 03:00`.

Toggle live with `s`. The startup default is resolved as:

```
--mode flag  >  git config redate.mode  >  built-in default (single)
```

Set your preferred default once:

```
git config redate.mode shift
```

## Dry run

`--dry-run` never writes. In a terminal it opens the editor and then
prints the planned changes; in a script (no TTY) it just previews the
selected range - handy for checking a range or catching a merge before
committing to it.

```
git redate --dry-run HEAD~5
```

## Safety and undo

Rewriting history changes commit ids. git-redate keeps this recoverable:

- Before moving the branch it prints the old tip and the exact undo
  command:

  ```
  git-redate: rewrote 4 commit(s)
    old tip: 1a2b3c4...
    new tip: 9f8e7d6...
    undo with: git reset --hard 1a2b3c4...
  ```

- The branch (or detached HEAD) move writes a reflog entry, so
  `git reflog` and `branch@{1}` also recover the previous state.
- Only commit dates change; file trees are untouched, so a dirty working
  tree is preserved (git-redate just prints a notice).

## Limitations (v1)

- Linear history only - a merge commit in the range is refused.
- The range must end at HEAD.
- GPG-signed commits have their signature dropped when rewritten (the
  date change invalidates it); a notice reports how many.
