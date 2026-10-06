# coxswain-dash

`towpath`, the Coxswain fleet dashboard: a terminal view of the chair, spend, machines, lanes, runs, queue and inbox,
built with [ratatui](https://ratatui.rs). It reads `cox dash --feed` and runs every action as a `cox` command after a
confirm that shows the command. Part of [Coxswain](https://github.com/ppfenning/coxswain).

## Install

From 0.31.0 bare `cox` opens `towpath` as its entry point, so the `towpath` binary must be on your PATH.

### Homebrew

Installing `cox` from the tap installs `towpath` alongside it. No Rust toolchain is needed.

```
brew install <tap>/cox
```

### Release asset

Each release attaches a prebuilt archive for two platforms, each with a `.sha256` sibling:

- macOS arm64: `towpath-<tag>-aarch64-apple-darwin.tar.gz`
- Linux x86_64: `towpath-<tag>-x86_64-unknown-linux-gnu.tar.gz`

No other platform has a prebuilt binary. Each archive holds both binaries, `towpath` and its alias, described below. Download the asset and its `.sha256` file for your platform from the GitHub release page of this repository. Verify the checksum, extract, and move `towpath` to a directory on your PATH. This example is for macOS arm64.

```
shasum -a 256 -c towpath-<tag>-aarch64-apple-darwin.tar.gz.sha256
tar -xzf towpath-<tag>-aarch64-apple-darwin.tar.gz
mv towpath <a-directory-on-your-PATH>/
```

On Linux x86_64, use `sha256sum -c` in place of `shasum -a 256 -c`, and the `x86_64-unknown-linux-gnu` asset name.

### The coxtop alias

`coxtop` was the previous name of this program. It remains as an alias that prints a one-line notice and then runs `towpath`. It goes away in 0.37. The crate is still named `coxswain-dash`.

### From source

A development checkout keeps building from source with `cargo build --release` and needs Rust. This is the path for development only.

```
cargo run --bin towpath
```

Checks (the same three run in CI and in every build lane, from `.agent-checks`):

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
