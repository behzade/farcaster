# Upstream source

This extension vendors the `preflight` package from:

- Repository: https://github.com/yevhen/bo-pi
- Commit: `e959ecf868cbede7aa1108a5cab5c352c6f57728`
- Package version: `0.0.10`
- Retrieved: 2026-07-26
- License: MIT, copied in `LICENSE`

Local changes:

- Route review rules through a Codex-style risk and user-authorization review.
- Keep the existing declarative command and protected-path gate.
- Deny critical actions and fail closed when review fails without a UI.
- Track repeated model-approved actions.
- Offer an exact project rule after the same action is approved twice.
- Include a script content hash in exact action fingerprints.
- Pin the reviewer to `openai-codex/gpt-5.6-terra` at low reasoning with no
  main-model fallback.
- Review explicit mixed sibling verdicts in one model request.
- Ignore project rules and settings until Pi trusts the project.
- Hand exact one-shot outside-write grants to the OS sandbox.
- Expand Pi-style home paths and guard both lexical and resolved paths.
- Pass Pi cancellation to reviewer calls and retry waits.

The Nix package builds this checked-in source. It does not fetch extension code
from GitHub or npm at runtime.
