# Rendering image for the demo tape: the upstream vhs image plus git.
#
# ghcr.io/charmbracelet/vhs bundles ttyd + ffmpeg + chromium, which is
# everything vhs itself needs, but it has no git - and the demo needs it
# twice: demo/make-demo-repo.sh builds the throwaway repository the tape
# edits, and the demo runs `git redate ...`, i.e. as a git subcommand,
# which is the whole point of the tool. Installing it here keeps the
# render a single `just demo` and keeps the version pinned to a Debian
# release rather than to whatever the host happens to have.
#
# git-redate itself is NOT baked in: `just demo` mounts the musl binary
# from target/, so re-recording after a code change needs no rebuild of
# this image.
FROM ghcr.io/charmbracelet/vhs

RUN apt-get update \
    && apt-get install -y --no-install-recommends git \
    && rm -rf /var/lib/apt/lists/*
