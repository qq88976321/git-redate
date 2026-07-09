# git-redate

A modern rewrite of [PotatoLabs/git-redate](https://github.com/PotatoLabs/git-redate):
interactively edit the author and committer dates of a range of commits
from a terminal UI, then rewrite history in place. Written in Rust,
powered by [gitoxide](https://github.com/GitoxideLabs/gitoxide) - no
shelling out to `git`.

## Highlights

- **Interactive terminal UI** - navigate commits, adjust any date field
  in place with calendar-correct carry, and type absolute dates.
- **Shift mode** - edit one commit and every newer commit moves by the
  same delta, preserving the gaps between them.
- **Fast and self-contained** - a single Rust binary that rewrites
  commit objects directly with gitoxide.
- **Safe** - a reflog entry and a printed `git reset --hard` undo command
  on every write, plus a `--dry-run` preview.

## At a glance

```
git redate <commit>        # edit <commit>..HEAD (exclusive)
git redate                 # edit the last 10 commits
git redate --root          # edit the entire history
git redate --dry-run HEAD~5
```

See [Install](install.md) to build it and [Usage](usage.md) for the key
bindings, edit modes, configuration, and recovery.

Signed commits are re-signed with your git signing config (SSH or
OpenPGP), so `git log --show-signature` stays Good; `--no-sign` drops
signatures instead.

## Limitations (v1)

- Linear history only: a range containing a merge commit is refused.
- The range must end at HEAD (the checked-out tip).
- x509/gpgsm signatures cannot be re-created (use `--no-sign`).
