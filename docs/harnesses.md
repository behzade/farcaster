# Harness Feature Table

This table shows the support declared by Farcaster's adapters. Actual support can depend on the installed harness version and selected model.

All six adapters support session resume, images, stopping runs, queued follow-ups, model selection, reasoning effort selection, MCP servers, approvals, and tool activity.

| Harness | History | Fork | Steer | Compact | Modes | Usage |
| --- | --- | --- | --- | --- | --- | --- |
| Pi | Yes | Yes | Yes | Yes | No | Yes |
| Codex | Yes | Yes | Yes | Yes | Yes | Yes |
| Cursor | Yes | No | No | No | Yes | No |
| OpenCode | Yes | Yes | Yes | Yes | Yes | Yes |
| Antigravity | No | No | No | No | Yes | No |
| Claude | Yes | No | No | No | Yes | Yes |

Other differences:

- Moving sessions between projects: Pi, Codex, and OpenCode.
- Harness commands: all six.
- Native subagent activity: all except Pi and Antigravity.
- File-change events from the harness: all except Pi.

Usage notes:

- Claude uses `claude -p`. Anthropic's guidance has been mixed on whether this usage counts toward subscription limits or separate limits.
- Antigravity's terms of service are unclear about using its harness in third-party apps. Use it at your own risk.

Sources: [Pi](../src/modules/agents/adapter/pi/mod.rs), [Codex](../src/modules/agents/adapter/codex/mod.rs), [Cursor](../src/modules/agents/adapter/cursor/mod.rs), [OpenCode](../src/modules/agents/adapter/opencode/mod.rs), [Antigravity](../src/modules/agents/adapter/antigravity/mod.rs) ([shared ACP capabilities](../src/modules/agents/adapter/acp/backend.rs)), [Claude](../src/modules/agents/adapter/claude/mod.rs).
