# Remote agents

Status: design direction; connection method and implementation remain open.

Support agents running on another machine, such as Codex reached through SSH.

## Scope

- Add a machine registry. Each machine has connection settings.
- Scope projects to a machine and a path on that machine.
- Use the project's machine when connecting to its agents and sessions.

## Assumptions to revisit

Remote support affects more than process launch. Two machines can have different
versions of the same harness, accounts, settings, models, and supported features.

- **Harness installation and version:** launch settings currently resolve programs
  from the local environment (`src/modules/agents/adapter/mod.rs` and
  `process_command.rs`). A harness name alone does not identify the installation
  or establish protocol compatibility on another machine.
- **Models and defaults:** `HarnessConfigurationStore` keys model catalogs by
  harness and project path, but model and effort defaults by harness alone
  (`src/app/runtime/session_identity.rs`). These keys have no machine identity.
  Model availability must reflect the target environment; a saved preference
  does not prove that the target supports it. Keep project context, since
  settings can also differ between projects on one machine.
- **Capabilities:** some feature checks use backend descriptors, such as
  `supports_reasoning_effort` in `src/modules/agents/adapter/mod.rs`. Adapter
  support does not prove that every installed harness version supports a feature.
- **Paths and history:** session records use paths without machine identity
  (`src/modules/agents/contract.rs`), and Codex history metadata includes local
  SQLite reads (`src/modules/agents/adapter/codex/catalog.rs`). Paths and native
  session IDs need their machine context.

## Open questions

- How should connections work: SSH, Tailscale, or a combination?
- How should the local machine fit into the registry?
- How should agent availability, authentication, and session history be found
  on each machine?
- How should version compatibility and supported features be checked, and how
  should cached models and capabilities be refreshed after changes?
- Should model and effort preferences be shared or scoped? How should an
  unavailable saved choice be handled?
- What happens to running agents when the connection drops? Is continued work
  and later reconnection required?
- Which file, Git, terminal, and attachment operations need remote support?
- How should remote agents reach Farcaster's MCP service?

No remote service, file sync, or transport implementation is chosen yet.
