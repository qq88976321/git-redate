# Recording the demo

Maintainer runbook. The demo is not shot by a human: `demo/redate.tape`
scripts every keystroke, so any agent can regenerate it after a UI
change. Output is `demo/redate.mp4` (the source of truth, embedded in
the docs site) plus `demo/redate.gif` derived from it (embedded in the
README, because github.com will not play a repo-relative `<video>`).

## How it works

`ghcr.io/charmbracelet/vhs` bundles ttyd + ffmpeg + chromium - which is
everything vhs needs, and this host has none of them natively - but it
has no git. `demo/Dockerfile` adds it, because the demo needs git twice:
`demo/make-demo-repo.sh` builds the throwaway repository the tape edits,
and the demo runs `git redate ...`, i.e. as a git subcommand, which is
the whole point of the tool.

git-redate itself is not baked into that image. `just demo` builds the
**musl** binary and mounts it, so the container's glibc is irrelevant
and a code change needs no image rebuild.

The demo repository is rebuilt from scratch inside the tape's hidden
preamble on every render: the demo rewrites its history, so it cannot be
a committed fixture. Everything in it is pinned - identity, dates, file
contents, signing off - so the commit ids are the same on every run.

## Prerequisites

- docker. **Sandbox off**: the image pull hits ghcr.io and docker.sock
  is a unix socket.
- `rustup target add x86_64-unknown-linux-musl` (the same target the
  Linux release binaries use).

## Steps

1. [ ] `just demo`. That builds the image (cached after the first run),
       builds the musl binary, renders `demo/redate.mp4`, and then runs
       `just gifs` to derive `demo/redate.gif`.
2. [ ] Acceptance criteria - inspect the mp4 with the image's ffmpeg,
       e.g.

       ```
       docker run --rm -v "$PWD/demo":/demo -v /tmp/f:/f \
         --entrypoint sh git-redate-vhs:local -c \
         'ffprobe -v error -show_entries format=duration -of csv=p=0 /demo/redate.mp4;
          ffmpeg -y -v error -ss 20 -i /demo/redate.mp4 -frames:v 1 /f/t20.png'
       ```

       - [ ] duration matches the tape's Sleeps (~33s; `PlaybackSpeed`
             is 1.0, so they add up directly). Much shorter means the
             12fps capture dropped frames - lower `Set Framerate`.
       - [ ] the prompt is a clean `$` and the first visible line is
             `$ git redate HEAD~5` - none of the hidden setup shows.
       - [ ] shift mode: all five rows change together and stay in
             chronological order, each marked `*`.
       - [ ] the expanded row shows `author 08:40` over `commit 06:40`,
             i.e. the two dates actually differ.
       - [ ] the report's `moved tag v0.1.0: <old> -> <new>` line fits
             on ONE line (that is what sets the 106-column grid).
       - [ ] the final screen holds the whole story without scrolling:
             command, tag note, report, and the new `git log`.
3. [ ] Check the sizes. The GIF is what every README visitor downloads:
       keep it under ~1 MB (currently ~0.55 MB at 1200px/8fps/64
       colors). If it grows, shorten the tape rather than degrading the
       palette.
4. [ ] Commit the mp4, the GIF, and any tape/script/README edits
       together: `docs(demo): re-record the demo`. Both files are binary
       but small; commit them normally, no git-lfs.

## Regeneration rule

Any commit that changes the TUI's rendering, its keybindings, the
report, or the demo fixture MUST regenerate the demo in the same branch
(steps 1-4). If drift is found later, treat it as a bug and regenerate
immediately - a demo that shows a UI the tool no longer has is worse
than no demo.

## Where the files are referenced

- `README.md` - `![git-redate demo](demo/redate.gif)`
- `website/docs/index.md` - `<video src="demo/redate.mp4">`; the mp4 is
  copied into `website/docs/demo/` by `just site-build`, `just
  site-serve`, and the `pages.yml` workflow (that directory is
  gitignored - `demo/` stays the single source).
