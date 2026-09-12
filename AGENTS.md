# Agent Instructions

Use Farcaster MCP to track substantial work with `workgraph_*` and coordinate overlapping changes with `worker_notices`.
When asked to show code or changes, use `submit_review` with relevant files, line ranges, and concise notes.

Never edit `README.md` or any user facing file in `docs` unless explicitly requesed by the user.

Keep Farcaster backend-neutral above its protocol adapters. Pi-specific session,
transport, trust, and extension behavior belongs behind the Pi backend boundary.
Do not add TypeScript or dependency directories to this repository.

Keep tests in sibling `*_tests.rs` files, not inline in production source files.

Use the existing Cargo target directory from the active environment. Start with
the narrowest relevant Cargo check and always run `git diff --check`.

On macOS, use `cargo test` so the configured runner stages tests outside the
crowded `deps` directory. For an already-built test, use
`scripts/run-macos.sh <binary> [args...]`; direct execution from `deps` can stall
system-wide program launches while macOS scans the directory.
