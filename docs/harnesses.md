# Harness Support

These tables reflect the capabilities declared by Farcaster's adapters. An installed harness version or selected model may narrow them.

All six adapters support new and resumed sessions, closing sessions, text and image prompts, stopping runs, queued follow-ups, model selection, reasoning effort, harness commands, MCP servers, approvals, streamed text, reasoning, and tool activity.

## Sessions

History means that Farcaster can discover and open saved sessions from the harness.

| Harness | History | Fork | Rename | Move project | Delete |
| --- | --- | --- | --- | --- | --- |
| Pi | Yes | Yes | Yes | Yes | Yes |
| Codex | Yes | Yes | Yes | Yes | Yes |
| Cursor | Yes | No | Yes | No | Yes |
| OpenCode | Yes | Yes | Yes | Yes | Yes |
| Claude | Yes | No | No | No | No |
| Antigravity | No | No | No | No | No |

## Runs and configuration

| Harness | Steer | Compact | Access modes | Reset effort | Usage |
| --- | --- | --- | --- | --- | --- |
| Pi | Yes | Yes | Auto; Sandboxed/Full with `pi-nono` | No | Yes |
| Codex | Yes | Yes | Sandboxed/Auto/Full | No | Yes |
| Cursor | No | No | Sandboxed/Full | No | No |
| OpenCode | Yes | Yes | Sandboxed/Full | Yes | Yes |
| Claude | Yes | No | Sandboxed/Full; Auto when the model allows it | No | Yes |
| Antigravity | No | No | Sandboxed/Full | No | No |

## Harness events

| Harness | Questions | Notifications | Child agents | File changes |
| --- | --- | --- | --- | --- |
| Pi | Yes | Yes | No | No |
| Codex | Yes | Yes | Yes | Yes |
| Cursor | Yes | Yes | Yes | Yes |
| OpenCode | Yes | Yes | Yes | Yes |
| Claude | No | No | Yes | Yes |
| Antigravity | No | Yes | No | Yes |

## Notes

- Pi uses its normal configuration in Auto mode. Sandboxed and Full controls require the `pi-nono` extension.
- Claude runs through `claude -p`.
- Antigravity can resume a known session, but Farcaster cannot discover its saved sessions.

Sources: [Pi](../src/modules/agents/adapter/pi/mod.rs) ([access modes](../src/modules/agents/adapter/pi/sandbox.rs)), [Codex](../src/modules/agents/adapter/codex/mod.rs), [Cursor](../src/modules/agents/adapter/cursor/mod.rs), [OpenCode](../src/modules/agents/adapter/opencode/mod.rs), [Claude](../src/modules/agents/adapter/claude/mod.rs), and [Antigravity](../src/modules/agents/adapter/antigravity/mod.rs) ([shared ACP capabilities](../src/modules/agents/adapter/acp/backend.rs)).
