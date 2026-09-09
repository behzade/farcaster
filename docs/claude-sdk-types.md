# Claude SDK types

`crates/claude-sdk-types` contains Rust JSON types for the Claude CLI's streaming
stdin/stdout protocol, derived from Claude Agent SDK
`0.3.257`, which reports Claude Code `2.1.257`. It is a standalone crate with only
Serde dependencies. Farcaster's `claude` adapter uses these types to run the
regular Claude CLI with `-p` and bidirectional streaming JSON. It resolves
`FARCASTER_CLAUDE_PATH` (default: `claude`) in the project environment and leaves
authentication to Claude. It does not use `claude-agent-acp` or migrate saved
`claude-acp` session identities.

The adapter initializes and selects models, effort and permission modes through
typed control messages. It streams text, thinking and tool results, queues prompts
locally, and answers tool approvals over stdin. Full access requires the explicit
full-access launch setting. Main sessions receive the Farcaster MCP caller token;
child sessions do not. Catalog probes initialize an unpersisted process without
sending a prompt. History and session discovery read native JSONL files without
starting Claude or changing those files.

Fork, rename, move, delete, steering, native notifications and interactive question tools are not
supported. Unsupported control requests receive an error; unsupported interactive
tools receive a denial. This backend uses a new `claude` identity, so it does not
need to preserve ACP test-session compatibility.

The source reference is
<https://github.com/anthropics/claude-agent-sdk-typescript>, cloned at
`246f936602d5344f7efe560d9563acebab00a358`. That repository publishes examples and
a changelog, not the SDK implementation or full declarations. The actual type
source is the npm package's `sdk.d.ts`. The pinned version matches the installed ACP dependency, rather
than the newer version at the repository's head.

`source-manifest.json` records the declaration hash, dependency versions, the
selected protocol roots, and reachable named declarations with their source lines.
The importer starts from stdin/stdout envelopes and named control/callback reply
bodies, then follows their type dependencies. It retains 191 SDK declarations
and emits 844 Rust definitions including nested and imported types.

Control success envelopes declare their payload as `Record<string, unknown>`.
The importer therefore explicitly includes named initialization, interrupt,
context, usage, file, plugin, skill, MCP and rewind responses, plus permission,
hook, elicitation and dialog replies. This keeps their fields typed even though
the envelope does not directly refer to them.

Standalone tool schemas, full settings schemas, session-storage APIs, browser
authentication, bridge APIs and JavaScript runtime objects are outside this
scope. They are omitted, not replaced with untyped stand-ins. No protocol message
variants or fields are removed from the selected roots to reduce the size.

The data types include:

- `StdoutMessage`, including control requests, responses, cancellation and keep-alive.
- Every `SDKControlRequestInner` variant and the SDK's named response payloads.
- `SDKMessage`, assistant content and stream-event types from the Anthropic API.
- Hook and permission messages, plus named control-response bodies.

Types preserve the source's property names and literal discriminants. By default,
required nullable fields must exist. `Presence<T>` distinguishes an absent optional
property from a present value, including `null` where the source allows it.
Numbers use `serde_json::Number`; open JSON fields use `Value` only where the
source declares `any` or `unknown`. Each such slot appears in the manifest.
Typed record values retain their types. Extra object properties survive a
decode/encode round trip, while unknown enum values fail to decode.

Farcaster enables the crate's `cli-compat` feature. Real CLI/provider output can
omit nullable Anthropic API metadata (`citations`, container details, usage
details), including fields the SDK makes required through `NonNullableUsage`.
Thinking blocks may also omit their signature until a later streaming delta.
The generator marks these source-derived fields with `WireMetadata<T>`: strict
builds keep `T`; compatibility builds use `Presence<T>`. Missing values stay
missing, with no fabricated metadata. Present values still use their declared
types. Input/output token counts, message content, discriminants, and control
envelopes remain required.

Structural unions try more specific shapes first. This matters for user replay
messages, whose shape extends ordinary user messages. Some SDK unions have
overlapping shapes without a discriminator; Rust cannot infer an intent that the
source type does not express. The adapter reports decoding failures and leaves
native session files intact for recovery. The adapter ignores `command_lifecycle` native
queue-status frames, which the SDK documents on `SDKControlInterruptRequest`
but omits from `StdoutMessage`. Farcaster queues prompts locally; it does not
use those frames for turn completion or permission decisions. A typed interrupt
receipt that cancels the active prompt before execution settles that turn, since
Claude sends no result for such a prompt. All other frames
still go through the typed decoder. The strict declarations alone do not prove
runtime compatibility; test the adapter against the installed CLI too.

Tool arguments remain untrusted data. A `Read` tool-use message can carry
`{"file_path":905}` because the SDK types its input as unknown. The adapter checks
path values before using them. It does not need every built-in tool's
input/output schema to exchange messages with Claude. A malformed tool argument
must not prevent it from accepting the surrounding message.

## Regeneration

Run from the Farcaster root. Keep the SDK, Node dependencies, and upstream clone
outside this repository. No Claude invocation is needed.

```sh
reference_dir=$(mktemp -d /private/tmp/farcaster-claude-sdk.XXXXXX)
git clone https://github.com/anthropics/claude-agent-sdk-typescript.git "$reference_dir/upstream"
git -C "$reference_dir/upstream" switch --detach 246f936602d5344f7efe560d9563acebab00a358
npm install --prefix "$reference_dir/tooling" --ignore-scripts --omit=optional --no-audit --no-fund \
  @anthropic-ai/claude-agent-sdk@0.3.257 @anthropic-ai/sdk@0.124.0 \
  @modelcontextprotocol/sdk@1.30.0 zod@4.5.4 typescript@5.9.3 \
  @types/node@22.18.6 json-schema-typed@8.0.2 undici-types@6.21.0
node scripts/import_claude_sdk_types.mjs \
  "$reference_dir/tooling/node_modules/@anthropic-ai/claude-agent-sdk" \
  "$reference_dir/tooling/node_modules/typescript" \
  246f936602d5344f7efe560d9563acebab00a358 | apply_patch
```

Use the same import command with `--check` and without `| apply_patch` to check
that checked-in files match the declarations. The script uses the TypeScript
compiler to resolve unions, intersections, mapped types, and imported types.
Unsupported types and unresolved imports fail generation. It also checks the
shared JSON fixtures against the source TypeScript types using an in-memory
compiler file; it writes no TypeScript into Farcaster.

For an update, change the pinned package/dependency versions, inspect the coverage
manifest and generated diff, and run the checks below. Do not discover types by
launching a live Claude session.

## Checks

Use the active Cargo target directory:

```sh
cargo test --manifest-path crates/claude-sdk-types/Cargo.toml --offline
cargo test --manifest-path crates/claude-sdk-types/Cargo.toml --features cli-compat --offline
cargo fmt --manifest-path crates/claude-sdk-types/Cargo.toml -- --check
cargo test --bin farcaster modules::agents::adapter::claude --offline
git diff --check
```

The ignored live conformance test runs only Claude when selected explicitly:

```sh
FARCASTER_E2E_HARNESS=claude cargo test --bin farcaster \
  live_harnesses_conform_to_session_outcomes --offline -- --ignored --nocapture
```

Claude-specific live checks cover exact-once rendered text, image retention,
nonzero token usage, Read,
queued prompts, native Agent delegation, history/resume, steering rejection,
and interrupt/recovery before execution and during a Bash tool:

```sh
cargo test --bin farcaster real_claude_ --offline -- --ignored --nocapture --test-threads=1
```

This consumes real model usage and leaves test sessions in Claude's native store.
`adapter/claude/fixtures/cli-2.1.236.jsonl` preserves a real minimal turn's stream,
assistant message and result, with session/message identifiers replaced.

Fixtures exercise initialization, permissions, control errors, user replay,
partial JSON, assistant tool use, and result usage. Rust tests
also check absent/null distinctions, literal validation, extra fields, and
malformed control-request paths. Regeneration follows only the selected protocol
roots. This is declaration and serialization coverage, not live CLI
or Farcaster UI coverage. Non-live adapter tests use a local shell fixture for initialization,
permissions, streaming, queueing, interrupt, process failure, configuration and
resume launch; they do not call Claude or a model API. Storage tests cover branch
selection, preserved compaction segments, bounded catalog reads and partial final
records.
