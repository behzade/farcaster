# Pi GPUI implementation handoff

## User request

Build a native Rust/GPUI client for Pi as a module in this repository. Use the
useful product/UI work from these local references:

- `/mnt/fast/Projects/issues` has the mature GPUI UI and interaction patterns.
- `/home/behzad/Projects/personal/codex/native` is an earlier native coding
  client attempt. Its overall product is not the target, but its UI/runtime seam
  and transcript work are useful references.

The client must use Pi as-is. Do not change Pi or any extension to support the
GUI. The approved integration is the public `pi --mode rpc` subprocess
protocol.

The user approved the first usable checkpoint described below and approved
GPL-3.0-or-later for this module so suitable Issues code can be adapted
lawfully.

## Repository and current state

Work only in:

- `/home/behzad/Projects/personal/pi/apps/pi-gpui`
- `/home/behzad/Projects/personal/pi/README.md` for the final repository-map and
  build-command update

Do not edit either reference repository, upstream Pi, or existing extensions.
Do not commit or stage files.

The enclosing repository is MIT. `apps/pi-gpui` is a distinct
GPL-3.0-or-later module and must contain its own complete license and an
attribution/modification notice for adapted Issues code.

The working tree was clean before implementation started. A stopped worker
created only these untracked partial files:

- `apps/pi-gpui/Cargo.toml`
- `apps/pi-gpui/LICENSE`

Review and correct them rather than assuming they are final. In particular,
replace the short license notice with the complete GPL-3.0-or-later text (the
reference copy is `/mnt/fast/Projects/issues/LICENSE`). Add `NOTICE.md` with
source paths/commit and a clear modification statement for adapted code.

The stopped worker session is only a historical artifact:

- run: `9a302c74-f426-4a5e-a796-c356b2b34e3b`
- session: `/home/behzad/.pi/agent/sessions/--home-behzad-nix-config--/2026-08-14T18-06-57-032Z_01a00174-a608-7ffd-a926-0966336f4d2b.jsonl`

Do not resume it. It was stopped because its parent session needed approval for
writes outside the original working directory.

## Approved checkpoint

Deliver a usable root-session client with:

1. One project-scoped Pi RPC process for the active root session.
2. Strict JSONL transport and typed request/event handling.
3. Existing-session list, search, and resume plus new-session creation.
4. Root-session history and live text, thinking, tool, error, queue, retry, and
   compaction rendering.
5. Composer modes for normal prompt, steer, and follow-up, plus abort.
6. Model and thinking-level controls.
7. Generic support for every RPC extension UI request listed below.
8. Responsive, accessible GPUI presentation adapted from Issues.

The binary accepts an optional project path and defaults to the current working
directory. Spawn Pi through `direnv exec` for that directory so its environment,
global/project settings, context, skills, prompt templates, packages, and
extensions match the terminal client. Do not inject product prompts or bypass
Pi.

## Explicit non-goals for this checkpoint

- Subagent/fleet transcript navigation or direct child messaging
- Arbitrary in-place session-tree navigation
- Diff review
- Embedded terminal
- A web app, HTTP bridge, WebSocket bridge, daemon, or Pi fork
- Packaging/installers or root-flake integration
- Changes to Pi RPC, Pi sources, or extension sources

Design the boundaries so those can be added without replacing the transport or
root application model.

## Pi primary sources already researched

Installed Pi is 0.84.2 at:

`/nix/store/ya9wzbgajv5dvivwxynfkg378l3yaxxi-pi-terminal-0.84.2/lib/pi-terminal/node_modules/@earendil-works/pi-coding-agent`

Read these before coding against the protocol:

- `README.md`
- `docs/rpc.md`
- `docs/session-format.md`
- `dist/modes/rpc/rpc-types.d.ts`
- `dist/modes/rpc/jsonl.js`
- `dist/modes/rpc/rpc-client.js` (its source map contains the TS source)
- `dist/modes/json-event.d.ts`
- `dist/core/session-manager.d.ts`
- `examples/rpc-extension-ui.ts`
- `examples/extensions/rpc-demo.ts`

Important findings:

- RPC uses stdin/stdout strict JSONL. LF (`0x0a`) is the only frame delimiter.
  Payloads can contain U+2028 and U+2029. Strip one optional CR before LF.
  Implement byte/chunk framing; do not use a generic Unicode line reader.
- Commands can carry an `id`. A successful command response echoes it. Events
  generally carry no request ID and no session ID.
- Use `get_state` as the startup/readiness handshake.
- `prompt` success means accepted/queued/handled, not that the run succeeded.
- `agent_end` is a low-level run end and can be followed by retry, compaction,
  or queued work. Only `agent_settled` means the session will not continue
  automatically.
- While streaming, a prompt needs `streamingBehavior: "steer"` or
  `"followUp"`, or use the dedicated `steer` / `follow_up` commands.
- `abort` does not clear queued messages. RPC has no clear-queue command.
- One Pi process cannot safely multiplex concurrent live sessions because
  events have no session identity. Use one process per concurrent live session.
  This checkpoint has one active process; replacing it on session selection is
  valid. Keep process ownership isolated so concurrent sessions can be added
  later.
- Do not let two processes own the same session JSONL file. Track the selected
  absolute path and prevent duplicate local ownership.
- `switch_session` exists, but replacing the process with
  `pi --mode rpc --session <absolute-path>` gives explicit lifecycle ownership
  and preserves the one-process/one-live-session invariant.
- RPC exposes `get_entries` and `get_tree` for the current session but no
  session-list command. Discover sessions from the documented v3 JSONL format.
- Arbitrary `/tree` navigation is not exposed through RPC. It is a non-goal.
- Installed declarations and prose docs differ in a few places. Decode
  defensively from `serde_json::Value` at the boundary and preserve unknown
  fields/events. One known difference is `get_commands`: installed declarations
  use `sourceInfo`, while prose examples show `location`/`path`. Another is
  compaction/session retained-tail detail. Do not make metadata listing depend
  on either disputed shape.

### RPC commands needed by the UI

At minimum type and exercise:

- `prompt`, `steer`, `follow_up`, `abort`
- `new_session`
- `get_state`, `get_messages`, `get_session_stats`
- `get_available_models`, `set_model`
- `get_available_thinking_levels`, `set_thinking_level`
- `compact`, `set_auto_compaction`
- `set_auto_retry`, `abort_retry`
- `get_commands`
- `extension_ui_response`

The transport may have a generic typed/raw command path for the remaining
public commands, but do not add irrelevant UI for direct RPC bash, HTML export,
fork/clone/tree, or other non-goals.

### Events to reduce/render

Handle these event families:

- `agent_start`, `agent_end`, `agent_settled`
- `turn_start`, `turn_end`
- `message_start`, `message_update`, `message_end`
- `tool_execution_start`, `tool_execution_update`, `tool_execution_end`
- `queue_update`
- `compaction_start`, `compaction_end`
- `auto_retry_start`, `auto_retry_end`
- `summarization_retry_scheduled`,
  `summarization_retry_attempt_start`, `summarization_retry_finished`
- `extension_error`
- `extension_ui_request`

For `message_update`, assemble deltas by `contentIndex`:

- `text_start` / `text_delta` / `text_end`
- `thinking_start` / `thinking_delta` / `thinking_end`
- `toolcall_start` / `toolcall_delta` / `toolcall_end`

`message_end.message` is authoritative. Replace the partial projection rather
than appending a duplicate final message. Tool update partial results are
accumulated snapshots, not deltas; replace displayed partial output on update.
Correlate tool lifecycle by `toolCallId`.

Treat unknown events as non-fatal diagnostics, not protocol corruption.
Malformed JSON frames, impossible response correlation, EOF, stderr/process
exit, and a failed readiness handshake are real transport failures and must be
shown clearly.

### Generic extension UI contract

Support all current requests without extension-specific adapters:

Dialog requests, which block the extension and require a matching response:

- `select`: title, options, optional timeout
- `confirm`: title, message, optional timeout
- `input`: title, optional placeholder, optional timeout
- `editor`: title, optional prefill

Responses:

- selected/input/editor value: `{type:"extension_ui_response", id, value}`
- confirmation: `{type:"extension_ui_response", id, confirmed}`
- cancel: `{type:"extension_ui_response", id, cancelled:true}`

Fire-and-forget requests:

- `notify`: message and info/warning/error tone
- `setStatus`: keyed set/clear
- `setWidget`: keyed set/clear, aboveEditor or belowEditor, string lines only
- `setTitle`: set native window title
- `set_editor_text`: replace composer text

Keep dialog focus inside the surface and restore it to the prior control on
close. A timeout is owned by Pi; the client need not run a second timer, but it
must safely ignore a late response after Pi has resolved it.

Keep approval dialogs inside the chat pane so the session rail remains usable.
The dialog belongs to the live session: park it while history is visible and
restore it unchanged when the user returns to that session.

RPC mode reports `ctx.hasUI = true`. TUI-only custom component methods are not
supported by RPC and need no GUI emulation.

## Session discovery contract

Pi session files are documented at:

`~/.pi/agent/sessions/--<encoded-cwd>--/<timestamp>_<uuid>.jsonl`

Resolution order:

1. `PI_CODING_AGENT_SESSION_DIR` when set
2. Otherwise `${PI_CODING_AGENT_DIR:-$HOME/.pi/agent}/sessions`

Do not rely only on reconstructing the encoded folder name. Scan bounded JSONL
candidates off the GPUI thread and parse the first `session` header. Use its
normalized `header.cwd` to group sessions from every project. A custom session
dir may not use the default folder layout.

For list/search metadata, tolerate unknown entry types and parse only what is
needed:

- header: id, cwd, timestamp, optional parentSession
- latest `session_info.name`
- first user-visible user text for fallback title/search
- modified time, path, and a bounded message count/search corpus

Never rewrite session files. Let Pi migrate/open them. Use temporary session
roots in tests. Search should be bounded and case-insensitive. Perform file IO
and parsing off the GPUI thread and guard late results by a generation/session
identifier.

History may be viewed while the live session is running. Keep live RPC events
in a parked snapshot, never publish them into the visible history, and restore
that snapshot when the user returns to the live session.

Create and show the FPS monitor only when the process has `DEBUG=true`.

## Native project and draft state

The new-session button lists known projects. “Add project” owns the native
folder picker; starting a session never opens a picker. Known projects come
from session headers, the launch path, and the small GUI registry at
`${PI_CODING_AGENT_DIR:-$HOME/.pi/agent}/gui-state.json`.

An unsubmitted new session is a GUI draft. Keep it in the left rail with a
`Draft` badge, including while another session's history is visible and across
app restarts. Once Pi accepts its first prompt, keep the row until discovery
finds Pi's real session file, then replace the draft through the normal session
list. Ask `get_state` after prompt acceptance and settlement so the runtime
learns that file path and refreshes discovery.

## Recommended architecture and seam contracts

Use one Rust crate with responsibility-based modules. A suitable shape is:

```text
apps/pi-gpui/
  Cargo.toml
  Cargo.lock
  LICENSE
  NOTICE.md
  README.md
  src/
    main.rs              # argument edge and app launch only
    launch.rs            # project/path validation and window setup
    protocol.rs          # wire DTO decode/encode; no process or GPUI
    framing.rs           # strict LF byte framing only
    rpc_process.rs       # child lifecycle, correlation, stderr, handshake
    runtime.rs           # UI-neutral commands/events and active-session owner
    sessions.rs          # read-only Pi v3 session discovery
    conversation.rs      # pure transcript/stream reducer
    layout.rs            # pure wide/compact/narrow policy
    theme.rs             # all visual tokens and component-theme install
    primitives/
      mod.rs
      button.rs
      dialog.rs
      feedback.rs
      content.rs
    transcript.rs        # GPUI projection of conversation state
    extension_ui.rs      # modal/toast/status/widget projection
    app.rs               # composition and top-level GPUI state
  tests/
    fixtures/fake-pi.sh
    rpc_process.rs
```

Adjust names when the code makes a better split, but preserve these seams:

- Protocol/framing/process/session/conversation modules know no GPUI.
- GPUI types never cross into the runtime boundary.
- Runtime owns retry/failure/process policy. UI owns display/focus/layout policy.
- Session files are read-only metadata inputs. Pi remains authoritative for
  opening and mutating sessions.
- Theme owns every visual literal. Small primitives are domain-neutral.
- Keep handwritten files near 500 lines and never above 1,000. Do not create
  numbered chunks, generic utility bags, pass-through wrappers, or a manager
  blob.

A portable fake child can be a POSIX `sh` fixture because supported targets are
macOS and Linux. Make the process command injectable in tests. The fixture must
speak strict JSONL and permit deterministic handshake/event/error/EOF cases.

## Reference code: what to reuse

### Issues (GPL source; direct adaptation is allowed within this GPL module)

Read its root `AGENTS.md` and `UI_DESIGN_CONTEXT.md` first. Useful files:

- `/mnt/fast/Projects/issues/crates/gpui/src/theme.rs`
- `/mnt/fast/Projects/issues/crates/gpui/src/primitives/mod.rs`
- `/mnt/fast/Projects/issues/crates/gpui/src/primitives/button.rs`
- `/mnt/fast/Projects/issues/crates/gpui/src/primitives/dialog.rs`
- `/mnt/fast/Projects/issues/crates/gpui/src/primitives/form.rs`
- `/mnt/fast/Projects/issues/crates/gpui/src/primitives/content.rs`
- `/mnt/fast/Projects/issues/crates/gpui/src/input.rs`
- `/mnt/fast/Projects/issues/crates/gpui/src/board_layout.rs`
- `/mnt/fast/Projects/issues/crates/gpui/src/lib.rs` around root layering,
  keymaps, and window setup

Reuse/adapt these patterns:

- typed theme -> domain-neutral primitive adapters -> product views
- pure responsive layout policy with boundary tests
- accessible dialog/sheet roles and tab groups
- semantic assertive/polite live feedback
- stable button labels with operation-local loading/disabled state
- focus ownership/return and scoped Escape behavior
- off-UI-thread blocking work with weak-entity/generation guards
- bounded/virtualized long surfaces

Do not import issue state, SQLite, JJ/Git automation, project registration,
issue badges, Board filters, or issue actions.

Reference commit seen during research:

`9642cbec726ed90f1aca8470b9bb068a33a3add4`

Use the actual current reference commit in `NOTICE.md` when implementation
starts (`git -C /mnt/fast/Projects/issues rev-parse HEAD`). Preserve any
third-party notices for assets. No icon asset is required for this checkpoint.

### Codex Native (architectural reference only; Apache source)

Useful files:

- `/home/behzad/Projects/personal/codex/native/crates/runtime/src/lib.rs`
  - UI-facing `RuntimeHandle` / `RuntimeEvent` seam around lines 90-315
  - do not reuse its Codex-core implementation or approval policy
- `/home/behzad/Projects/personal/codex/native/crates/ui/src/lib.rs`
  - per-session state and event reduction ideas
- `/home/behzad/Projects/personal/codex/native/crates/ui/src/transcript.rs`
  - follow/scroll and streaming transcript ideas
- `/home/behzad/Projects/personal/codex/native/crates/ui/src/app_shell.rs`
  - three-surface shell idea
- `/home/behzad/Projects/personal/codex/native/crates/ui/src/composer.rs`

Do not copy Codex runtime/auth/session/sandbox/approval code. Pi owns all of
that behind RPC.

## Visual direction (approved)

Use the repository's existing Gruvbox dark-hard identity from:

`/home/behzad/Projects/personal/pi/themes/gruvbox-dark-hard.json`

Core colors:

- canvas `#1d2021`
- panel `#282828`
- surface `#3c3836`
- primary text `#ebdbb2`
- accent `#8ec07c`
- warning `#fabd2f`
- errors may use the existing bright red `#fb4934`

Use UI sans for chrome and a platform monospace face for execution detail.
Keep assistant prose readable; code/tool output is monospace. Avoid bundling a
font in this checkpoint.

Responsive modes:

- Wide (about 1180 px and above): session rail, transcript/composer, and a
  quiet run/extension surface.
- Compact (about 960-1179 px): collapsed session rail and transcript; secondary
  details become an overlay/sheet.
- Narrow (below about 960 px): transcript/composer first; sessions and run
  details open as sheets.

The single signature element is a compact execution spine that visually joins
related tool activity. Spend visual emphasis there; keep the rest restrained.
Thinking is visually quieter and collapsible. Tool output must be bounded and
expandable. Do not rely on color alone for status.

Extension widgets must appear in their requested above/below-editor placement.
Keyed statuses belong in a bounded status area, not an unbounded footer.
Notifications are transient but errors also remain discoverable in session
state.

## Validation contract

Write behavior tests before or with implementation for:

1. Framing
   - arbitrary byte chunks
   - multiple frames in one chunk
   - U+2028/U+2029 inside JSON strings
   - CRLF input stripping only the CR adjacent to LF
   - unterminated final frame/EOF policy
   - malformed JSON reported without panicking
2. RPC process
   - unique request IDs and correlation
   - `get_state` readiness handshake
   - successful response vs asynchronous event routing
   - stderr and nonzero exit
   - EOF with pending requests
   - clean terminate with bounded escalation
   - deterministic fake Pi subprocess integration
3. Conversation reducer
   - text and thinking deltas by content index
   - tool-call argument assembly
   - tool start/update/end correlation
   - authoritative `message_end` replacement, no duplicate
   - queue projection
   - retry/compaction notices
   - `agent_end` not settled; `agent_settled` settled
4. Sessions
   - temporary v3 JSONL fixtures
   - discovery and project grouping across cwd values
   - environment override paths
   - name and first-user-message fallback
   - bounded case-insensitive search
   - malformed/unknown entries do not poison valid sessions
5. Extension UI
   - decode every request variant
   - serialize every valid response variant
   - keyed status/widget set and clear
   - late dialog response safely ignored
6. Layout
   - exact wide/compact/narrow boundaries

The root `.envrc` supplies Cargo and the native GPUI build environment. Use
those tools directly. Do not run a Nix command unless the user asks for that
exact check. Do not search for system executables or create another target
directory. Run the shared check target:

```sh
make check-gpui
```

For a focused check, invoke Cargo directly and keep the `CARGO_TARGET_DIR`
supplied by the environment. If the active environment lacks a required tool or
build variable, report that setup defect instead of constructing a second
environment. If dependencies require network, request only the exact host needed
or report the blocker. Never claim build/test success without command output.

Also run:

```sh
git -C /home/behzad/Projects/personal/pi diff --check
git -C /home/behzad/Projects/personal/pi status --short
```

## Definition of done for this checkpoint

- A user can start `pi-gpui [project]`, see session choices, create or resume a
  root session, and use the normal installed Pi configuration/extensions.
- History loads once and streaming does not duplicate final messages.
- Prompt/steer/follow-up/abort, model, and thinking controls work through RPC.
- Every supported extension UI request has a native projection and correct
  response behavior.
- Process/session/protocol failures are visible and recoverable where the
  protocol permits.
- GPUI remains responsive during process and filesystem work.
- Tests cover the contracts above and focused checks pass, or the exact external
  blocker is recorded.
- Root `README.md` lists `apps/pi-gpui`, its GPL boundary, and focused build/test
  commands.
- No Pi or extension files changed. No reference source changed. Nothing is
  staged or committed.

After implementation, use fresh-context reviewers for correctness/protocol,
UI/accessibility, and tests/maintainability. Apply accepted fixes with one
writer, rerun affected checks, and inspect the final diff.
