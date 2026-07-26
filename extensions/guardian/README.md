# Pi Guardian

Pi Guardian reviews only actions selected by the declarative command and path
gate. Safe rules still run without a model call, and hard denies never reach
the model.

The reviewer uses `openai-codex/gpt-5.6-luna` by default. It receives a bounded
user and assistant transcript as untrusted evidence and returns:

- risk: low, medium, high, or critical;
- user authorization: high, medium, low, or unknown;
- outcome: allow or deny;
- one short reason.

The extension fails closed if Luna is missing or review fails without an
interactive UI. It does not fall back to the main Pi model.

For a shell command with a literal write path outside the project, Guardian
reviews the command before it runs. An allow result gives the sandbox that
exact path for that one tool call. It does not turn off the sandbox or widen
network access. Protected credential paths remain blocked.

When the latest user message names different verdicts for sibling actions,
Guardian sends the whole sibling set to Luna once and keeps each verdict by
tool-call ID.

Home Manager writes the active reviewer mode, model, and repeat limit to
`~/.pi/agent/extensions/guardian.json`. This managed config takes priority
over mutable user defaults. Session-only changes still work.

After Luna approves the same exact action twice in one session, Pi asks whether
to save an allow rule for that project. Saved rules live in:

```text
.pi/preflight/settings.local.json
```

The rule key includes the tool, exact arguments, project path, and, for a local
script call, the script SHA-256. Changing the script makes the saved rule stop
matching. Guardian reads and writes project rules only when Pi trusts the
project.

Source and license details are in `UPSTREAM.md` and `LICENSE`.
