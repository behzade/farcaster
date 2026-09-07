# Harness Feature Table

This table shows the support declared by Farcaster's adapters. Actual support can depend on the installed harness version and selected model.

All four adapters support session history and resume, images, stopping runs, queued follow-ups, model selection, MCP servers, approvals, and tool activity.

| Feature | Pi | Codex | Cursor | OpenCode |
| --- | --- | --- | --- | --- |
| Fork sessions | Yes | Yes | No | Yes |
| Move sessions between projects | Yes | Yes | No | Yes |
| Steer an active run | Yes | Yes | No | Yes |
| Compact context | Yes | Yes | No | Yes |
| Set reasoning effort | Yes | Yes | No | Yes |
| Select agent modes | No | Yes | Yes | Yes |
| Harness-provided commands | Yes | No | Yes | Yes |
| Usage reporting | Yes | Yes | No | Yes |
| Native subagent activity | No | Yes | Yes | Yes |
| File-change events from the harness | No | Yes | Yes | Yes |

Native subagent activity is separate from Farcaster's worker tools. File-change events are separate from the app's Git change list.

Sources: [Pi](../src/modules/agents/adapter/pi/mod.rs), [Codex](../src/modules/agents/adapter/codex/mod.rs), [Cursor](../src/modules/agents/adapter/cursor/mod.rs), [OpenCode](../src/modules/agents/adapter/opencode/mod.rs).
