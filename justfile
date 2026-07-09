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

# Run unit tests.
test:
    cargo test

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
