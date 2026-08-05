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

## Install

```
curl -fsSL https://github.com/qq88976321/git-redate/releases/latest/download/install.sh | sh
```

A checksum-verified prebuilt binary for Linux and macOS, installed into
`~/.local/bin` without `sudo`. See [Install](install.md) for the flags,
the platform list, and building from source.

## Demo

<figure>
  <figcaption>Five commits made between 02:11 and 03:40: shift mode moves
  the whole run three hours, single mode nudges the last one on its own,
  and expanding it pulls the committer date back from the author date.
  The write reports the old tip, the undo command, and the tag that
  followed its commit.</figcaption>
  <video src="demo/redate.mp4" controls preload="metadata" width="100%"></video>
</figure>

## At a glance

```
git redate <commit>        # edit <commit>..HEAD (exclusive)
git redate                 # edit the last 10 commits
git redate --root          # edit the entire history
git redate --dry-run HEAD~5
```

See [Usage](usage.md) for the key bindings, edit modes, configuration,
and recovery.

Signed commits are re-signed with your git signing config (SSH or
OpenPGP), so `git log --show-signature` stays Good; `--no-sign` drops
signatures instead.

Tags pointing at rewritten commits move with them (annotated tags are
rebuilt and re-signed), so they do not stay behind on the old objects;
`--no-retag` leaves them alone.

## Limitations (v1)

- Linear history only: a range containing a merge commit is refused.
- The range must end at HEAD (the checked-out tip).
- x509/gpgsm signatures cannot be re-created (use `--no-sign`).
- A tag pointing at another tag is not rewritten (it is reported and
  left alone), and tags are never pushed for you.
- A committer identity must be configured (`user.name` and
  `user.email`) - the reflog entry needs one. git derives one from the
  system instead; git-redate refuses before the editor opens rather
  than invent an identity.

## Status

!!! note "Personal tool, built with heavy AI assistance (Claude Code)"

    I review what ships and use it on my own repositories, but it comes
    with no warranty and no support commitment: issues and PRs are welcome
    and may still go unanswered.

    It rewrites history: preview with `--dry-run` first, keep the old tip
    the report prints, and do not point it at a branch other people have
    already pulled.

Source on [GitHub](https://github.com/qq88976321/git-redate),
[MIT licensed](https://github.com/qq88976321/git-redate/blob/master/LICENSE).
