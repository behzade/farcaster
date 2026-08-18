# Agent Instructions

Complete the requested change, run checks that cover it, and inspect the final
diff. Keep unrelated work intact.

## Development environment

- The project shell is already active. Use the tools on `PATH`.
- Do not run `nix`, `nix-build`, `nix-store`, `nix develop`, or any other Nix
  command unless the user asks for that exact check in the current task.
- Cargo is already available. Run it directly and keep the `CARGO_TARGET_DIR`
  supplied by the environment; do not override it or create another target
  directory.
- Do not create new Nix caches, Rust target directories, or dependency folders.
  If the environment lacks a tool or build variable, report that problem.

## UI assets

- Use Phosphor icons from
  `/Users/behzad/Projects/personal/issues/assets/phosphor-icons` for application
  icons instead of Unicode glyphs or improvised icons.

## Checks

- Start with the smallest check that covers the changed behavior. Do not run
  every command listed in the README.
- For `sandbox-broker`, use Cargo with
  `--manifest-path sandbox-broker/Cargo.toml`.
- For `apps/pi-gpui`, use Cargo with
  `--manifest-path apps/pi-gpui/Cargo.toml`. Use `make check-gpui` only when the
  change needs the full GPUI check set.
- For TypeScript, use the affected package's `npm` check or the exact relevant
  `node --test` files.
- Always run `git diff --check`. Report any check you skipped and why.
