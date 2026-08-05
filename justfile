# Entry points for git-redate development and release.

# Docs toolchain: run Zensical via uvx (isolated, no venv to manage).
# Pinned because Zensical is pre-1.0; bump this single line to upgrade.
zensical := "zensical@0.0.47"

# List available recipes.
default:
    @just --list

# Debug build.
build:
    cargo build

# Install the `git-redate` binary to ~/.cargo/bin (already on PATH);
# with it on PATH, git runs `git redate ...` as a subcommand. Remove
# with `cargo uninstall git-redate`.
install:
    cargo install --path .

# Fast local install (dev profile, no LTO) for a working `git-redate`
# on PATH. Much quicker than `install` after code changes, since it
# skips the release profile's whole-program LTO; use `install` only
# when you actually want the optimized binary.
install-dev:
    cargo install --path . --debug

# Run unit tests.
test:
    cargo test

# Lint the shell scripts we ship (install.sh is executed by users
# straight off a release asset, so it has to be POSIX sh clean).
# Enforced in CI; kept out of `gate` so the gate does not silently
# skip a step on a machine without shellcheck.
lint-sh:
    shellcheck -s sh install.sh scripts/test-install.sh

# End-to-end round trip for install.sh against a fake release tree
# served over file://. Builds the musl asset first, so it needs
# `rustup target add x86_64-unknown-linux-musl`. This is the only way
# to exercise the install path without cutting a real release.
test-install:
    sh scripts/test-install.sh

# Full quality gate; run before every commit.
gate:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test
    cargo build --release

# cargo-release pre-release hook: run the gate, then regenerate
# CHANGELOG.md for the release being prepared. cargo-release sets
# NEW_VERSION and DRY_RUN; on a dry run the changelog is left untouched
# so the working tree stays clean. Needs git-cliff on PATH.
release-hook:
    just gate
    if [ "$DRY_RUN" != "true" ]; then git-cliff --tag "v${NEW_VERSION}" -o CHANGELOG.md; fi

# Run the binary against the current repository; pass extra args, e.g.
# `just run -- --dry-run HEAD~5`.
run *args:
    cargo run -- {{args}}

# First release: `just release 0.1.0` (explicit version -> tag v0.1.0,
# no patch bump); afterwards `just release patch|minor|major`.
# Bump version and tag vX.Y.Z locally (no push).
release level="patch":
    cargo release {{level}} --execute

# Build the docs site (website/, Zensical). Runs Zensical via uvx;
# needs uv on PATH (no venv to activate).
site-build:
    cd website && uvx {{zensical}} build --clean

# Serve the docs site locally at 0.0.0.0:8099 (Ctrl-C to stop).
site-serve:
    cd website && uvx {{zensical}} serve -a 0.0.0.0:8099
