# Isolated instance and Computer Use

## Source of truth

Check these files when behavior differs from this guide:

- `scripts/run-isolated-macos.sh`: bundle creation, executable refresh, background launch.
- `src/app/infrastructure/isolation.rs`: temporary data copy and isolated MCP listener.
- `src/app/infrastructure/launch.rs`: app identity, window focus, and saved bounds.
- `src/app/ui/accessibility.rs`: conditional panel caching for AX clients.
- `src/app/ui/primitives/search.rs`: accessible search labels on the actual inputs.

The launcher uses `${CARGO_TARGET_DIR:-<repo>/target}` and expects `release/farcaster` and the release `Farcaster.app` bundle. Use `scripts/bundle.sh` if the bundle is missing, after checking its current arguments and build behavior. Do not rebuild Nix inputs or create new cache/target trees to solve a small UI change.

```sh
cargo build --offline --release -p farcaster --bin farcaster
scripts/run-isolated-macos.sh
```

Use the supported Rust/Zig versions from the current environment; do not assume the shell's first `cargo` is correct. Keep the existing Cargo target. On macOS, run tests through `cargo test` or `scripts/run-macos.sh <built-test> [args]`, never directly from `deps`.

## Refresh the exact instance

`/private/tmp/farcaster-ui-instance-path` contains the reusable bundle path. Its identity is `io.github.behzade.farcaster.isolated`, distinct from the main app. The launcher reuses a running process and **does not copy a new executable into it**.

To load a build:

1. Read the pointer and find the process whose executable is exactly `<bundle>/Contents/MacOS/farcaster`. Identify the PID before stopping anything. Never use a broad `pkill farcaster`.
2. Record the old executable hash and save a baseline copy when needed. Stop only that exact Isolated PID, wait for it to exit, then call the launcher. Do not overwrite a running executable.
3. Verify the bundle executable matches the intended build, the new PID uses that bundle, and the launch log identifies isolated state. Keep only one Isolated process.

The launcher's ad hoc signing changes the executable's full-file hash. Compare its Mach-O UUID and code sections with the built executable, or compare hashes before signing; a post-signing hash mismatch alone does not mean the wrong build loaded.

`open -g` preserves the user's active app. Relaunching does not authorize activating a window or changing the user's main app.

## What isolation covers

`--isolated` creates a private temporary app-data directory, snapshots SQLite state, copies supported project/trust/image data, relocates session locators, and starts a separate localhost MCP listener. It does **not** isolate project files or backend histories. Do not submit prompts, run agents, accept new trust grants, or make source-control changes merely to test the UI.

`FARCASTER_DATA_DIR` names a **seed source**, not a permanent isolated destination. Each process creates a new copy. A relaunch may reset the selected session, expansions, and trust state. Record the actual temp path from the launch log; do not assume an earlier directory still exists. A copied seed can help match app state across builds, but shared backend histories can still change.

## Computer Use failures

Use the installed Computer Use skill and its supported app-targeted tools. Get fresh state after actions, derive element indices from that state, and check what actually changed. Prefer the bundle ID over a failing display-name target. Use only screenshot-grounded coordinate or keyboard fallbacks.

| Symptom | Check and limit |
| --- | --- |
| Visible control missing from a fresh AX tree | Compare another full tree after a redraw and inspect the actual view path. GPUI cached prepaint currently omits AX-node replay; `cached_panel` bypasses that cache while AX is active. All panel paths, including sheets/native overlays, must use the same rule. |
| Stale element or click has no effect | Refresh state once; confirm the target still exists. Inspect the screenshot and use a supported fallback if needed. Do not infer success from a tool call alone. |
| Search text reaches the composer | Stop typing. Verify focus and field value before another edit; never use Return or multiline text until the target is certain. Never submit a draft as a probe. |
| Search has only a generic label | Put its accessible name on `Input`, not a role-less wrapper. Keep stable IDs and proper roles on custom controls. |
| `noWindowsAvailable` or background actions fail | Confirm the exact running bundle and current window. AX-tree fixes do not establish a fix for OS/tool window targeting. Respect the user's manual-foreground preference; report the limit instead of repeating the same failed operation or handoff request. |
| Tiny, white, clipped, or moving capture | Fetch a settled state before saving it. Recheck after resize/maximize; never present a transition frame as a layout result. |

For AX fixes, check more than the first frame: keep fetching full trees after unrelated redraws, open/close a modal through AX controls, and verify both sidebars and named search fields remain available. The current headless GPUI test platform does not activate native accessibility, so prepaint tests alone do not prove native AX behavior.
