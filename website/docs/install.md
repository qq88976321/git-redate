# Install

## From source

git-redate is a standard Cargo project. Build and install the binary
with:

```
cargo install --path .
```

This produces a `git-redate` binary. As long as it is on your `PATH`,
git will dispatch `git redate ...` to it as a subcommand.

### Requirements

- Rust 1.85 or newer (the [gitoxide](https://github.com/GitoxideLabs/gitoxide)
  dependency sets this minimum).
- No system `git` or `libgit2` is required at runtime - history is read
  and rewritten with pure-Rust gitoxide.

## From a release build

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
