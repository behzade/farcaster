---
name: farcaster-isolated-ui
description: Build and inspect Farcaster UI changes in the isolated macOS app, capture matched before/after screens, and run the established blind council review before presenting changes for commit. Also use when diagnosing Computer Use or accessibility failures in Farcaster Isolated.
---

# Farcaster isolated UI work

Keep the requested scope. Inspect the running result yourself, then use matched screenshots for visual review. A council score does not authorize more changes or a commit.

## Establish the baseline

- Read the repo's `AGENTS.md`, check the diff, and use Farcaster workgraph/notices for substantial work and shared paths.
- Record the exact changes the user requested and the behavior to preserve. Typography or sidebar work does not authorize changes to composer height, spacing, or controls.
- Before editing visual code, capture the current isolated build and record its revision, executable, viewport, theme, selected session, scroll position, expansions, and focus state. Prefer a completed session with stable content. Keep a copy of the baseline executable if a second build will overwrite it.
- Use whole screens. Flat and grouped sidebars are two states of **each version**, never the before and after versions themselves. Capture both when the sidebar or shared layout changes. Add modal, focused, or expanded states that actually cover the requested work.

## Build and inspect

Read [references/isolated-instance.md](references/isolated-instance.md) before launching or replacing an instance. Use `scripts/run-isolated-macos.sh`; do not create another launcher or change the main app's state.

- Run the narrowest useful Cargo checks and `git diff --check`, using the active Cargo target and macOS test runner. Follow the repo's Nix limits.
- Build the release executable with the repo's supported toolchain. Verify the launcher copied that executable; a running instance keeps its old executable even after a successful build.
- Use the available Computer Use skill for all UI actions and captures. Inspect the exact Isolated bundle, not the main Farcaster window. Respect the user's focus constraints; the launcher deliberately uses `open -g`. Do not raise the app when the user has reserved focus changes for themselves.
- Verify the changed behavior live, including relevant focus, hover, narrow-window, modal, and expansion states. Tests and source review do not establish live UI behavior. Confirm the screenshot has settled before saving it.
- If Computer Use cannot reach a control, follow the failure checks in the reference. Do not keep requesting the same foreground handoff or reuse stale element indices.

## Blind visual review

For the established before/after council flow, read [references/council.md](references/council.md). Start the OpenCode `designer` with the other reviewers, not after the results arrive.

Freeze the implementation and captures before requesting scores. Reviewers score individual parts from whole screens, with neutral labels, image evidence, and capture limits. They do not propose improvements or choose a winner. The user judges the result.

**After scores arrive, report them without further source edits.** If a new change is needed, start another comparison batch. Present the change, focused checks, live evidence, scores, and limits before asking for any still-needed commit approval. Do not treat a prior batch's commit approval as approval for this batch.

For nonvisual accessibility fixes, verify AX behavior directly; do not invent a visual comparison for unchanged pixels.

## Finish

- Inspect the final diff for unrelated changes and preserve other workers' edits. Use `submit_review` when presenting code or changes.
- Leave one updated Isolated instance open, and report which build it runs. Keep baseline captures and evidence outside the repo.
- Commit only when the user authorizes it. Stage the reviewed files and inspect the staged paths. Never bundle unrelated work.
- Separate source checks, test results, live observation, council scores, and unresolved tool limits in the report. Do not claim that a successful screenshot proves clicks work, or that a test proves a window-targeting failure is fixed.
