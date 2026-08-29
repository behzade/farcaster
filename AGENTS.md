# Agent Instructions

Keep Farcaster backend-neutral above its protocol adapters. Pi-specific session,
transport, trust, and extension behavior belongs behind the Pi backend boundary.
Do not add TypeScript or dependency directories to this repository.

Use the existing Cargo target directory from the active environment. Start with
the narrowest relevant Cargo check and always run `git diff --check`.
