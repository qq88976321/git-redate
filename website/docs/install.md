# Install

## Quick install

```
curl -fsSL https://github.com/qq88976321/git-redate/releases/latest/download/install.sh | sh
```

The script downloads the prebuilt binary for your platform, verifies it
against the sha256 published with the release, and installs it into
`~/.local/bin`. Nothing needs `sudo`. As long as that directory is on
your `PATH`, git dispatches `git redate ...` to the binary as a
subcommand; the script tells you what to add to your profile if it is
not.

!!! tip "Read before you run"

    Piping a script into a shell is worth a look first:

    ```
    curl -fsSL https://github.com/qq88976321/git-redate/releases/latest/download/install.sh -o install.sh
    less install.sh
    sh install.sh
    ```

    The installer ships as an asset of each release, so the copy you
    fetch is the one written against that release's assets.

### Options

```
install.sh [--version <tag>] [--to <dir>]
```

| Option | Environment | Effect |
|--------|-------------|--------|
| `--version <tag>` | `GIT_REDATE_VERSION` | Install that release instead of the latest, e.g. `v0.1.0` |
| `--to <dir>` | `GIT_REDATE_INSTALL_DIR` | Install into `<dir>` instead of `~/.local/bin` |

Flags win over the environment. To pass a flag through the one-liner,
give the shell `-s --`:

```
curl -fsSL https://github.com/qq88976321/git-redate/releases/latest/download/install.sh | sh -s -- --to ~/bin
```

## Supported platforms

| Platform | Release asset |
|----------|---------------|
| Linux x86_64 | `git-redate-x86_64-unknown-linux-musl.tar.gz` |
| Linux aarch64 | `git-redate-aarch64-unknown-linux-musl.tar.gz` |
| macOS Intel | `git-redate-x86_64-apple-darwin.tar.gz` |
| macOS Apple silicon | `git-redate-aarch64-apple-darwin.tar.gz` |

The Linux binaries are statically linked against musl, so one build per
architecture runs on every distribution whatever glibc version it ships -
Alpine and CentOS 7 included. Windows has no prebuilt binary; build from
source there.

Every asset is published with a `git-redate-<target>.sha256` sidecar, so
a manual download can be checked the same way the installer does:

```
sha256sum -c git-redate-x86_64-unknown-linux-musl.sha256
```

## From source

git-redate is a standard Cargo project:

```
cargo install --path .
```

### Requirements

- Rust 1.85 or newer (the [gitoxide](https://github.com/GitoxideLabs/gitoxide)
  dependency sets this minimum).
- No system `git` or `libgit2` is required at runtime, whichever way you
  install - history is read and rewritten with pure-Rust gitoxide.

To build without installing:

```
cargo build --release
# the binary is at target/release/git-redate
```

Copy `target/release/git-redate` somewhere on your `PATH`.

## Verify

```
git redate --help
git redate --version
```

## Uninstall

Delete the binary:

```
rm ~/.local/bin/git-redate
```

If you installed with Cargo, use `cargo uninstall git-redate` instead.
