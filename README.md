# Pi

This repo owns Behzad's reviewed Pi coding-agent setup. It contains local
extensions, the vendored Guardian reviewer, the OS sandbox wrapper, background
job support, tests, and pinned Nix builds for third-party extensions.

Machine policy stays in `nix-config`. That repo chooses the Guardian model,
command rules, sandbox paths, and notification settings. This repo owns code
and package pins.

## Layout

- `extensions/guardian`: vendored and changed permission reviewer.
- `extensions/sandbox`: OS sandbox wrapper and Guardian one-shot grant bridge.
- `extensions/*.ts`: local notification, input, session, and title hooks.
- `skills/background-jobs`: non-blocking shell job control.
- `nix`: pinned package builds for Guardian, sandbox, subagents, and server
  compaction.
- `tests`: shared policy and notification checks.

## Checks

```sh
npm ci --prefix extensions/guardian
npm test --prefix extensions/guardian
node --test tests/governance.test.ts
nix flake check
```

Build one extension with:

```sh
nix build .#guardian
nix build .#sandbox
nix build .#subagents
nix build .#openai-server-compaction
```

`nix-config` consumes this repo as a flake input and deploys the source and
built packages through Home Manager.
