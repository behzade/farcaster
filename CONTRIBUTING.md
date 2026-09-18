# Contributing

Bug reports and focused pull requests are welcome. For large changes, open an issue first so we can agree on the scope.

Use the Nix development shell, then run:

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets -- -D warnings
```

Keep changes small, add useful tests, and explain what changed and how you checked it.
