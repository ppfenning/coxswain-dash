# coxswain-dash

`coxtop`, the Coxswain fleet dashboard: a terminal view of the chair, spend, machines, lanes, runs, queue and inbox,
built with [ratatui](https://ratatui.rs). It reads `cox dash --feed` and runs every action as a `cox` command after a
confirm that shows the command. Part of [Coxswain](https://github.com/ppfenning/coxswain).

```
cargo run --bin coxtop
```

Checks (the same three run in CI and in every build lane, from `.agent-checks`):

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
