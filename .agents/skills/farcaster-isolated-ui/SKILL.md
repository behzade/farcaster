---
name: farcaster-isolated-ui
description: Inspect Farcaster UI in the isolated macOS app. Council review requires an explicit user request.
---

# Isolated UI work

- Keep scope and checks proportional. Simple edits do not need workers, screenshots, or a release build by default.
- Before launching or refreshing Isolated, read [isolated-instance.md](references/isolated-instance.md). Use `scripts/run-isolated-macos.sh`, verify the loaded executable, and preserve the main app and user focus.
- Use Computer Use for UI actions and captures. Report tool limits; source checks and screenshots do not prove interactions work.
- For visual comparisons, capture matched whole screens with the same viewport, theme, session, and state. Disclose mismatches and keep evidence outside the repo.
- Only run [council review](references/council.md) when the user explicitly asks for “council” on the current task. Past requests do not carry over.
- Preserve others’ edits, run `git diff --check`, and report the change and any remaining uncertainty. Commit only when authorized.
