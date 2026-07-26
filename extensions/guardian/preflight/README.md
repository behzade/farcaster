# Preflight

Preflight adds a tool approval layer to Pi.

It summarizes each tool call, marks destructive actions, applies deterministic and natural-language policy rules, and asks for approval when needed.

## Install

Install from npm:

```bash
pi install npm:@yevhen.b/pi-preflight
```

From a local checkout:

```bash
pi install /absolute/path/to/bo-pi/preflight
```

Temporary run without installing:

```bash
pi -e npm:@yevhen.b/pi-preflight
```

## Commands

- `/preflight` opens the interactive settings menu.
- `/preflight status` prints active settings.
- `/preflight approvals all|destructive|off` sets approval mode.
- `/preflight context full|<N>` sets context for explain and rule suggestions.
- `/preflight model current|provider/model` sets the preflight model.
- `/preflight policy-model current|provider/model` sets the policy/rule-suggestion model.
- `/preflight debug on|off` toggles debug logs.
- `/preflight reset-session` clears session overrides.

## Highlights

- Human-readable summaries for tool calls before you approve them.
- Three approval modes: ask for everything, only destructive actions, or turn approvals off.
- Deterministic permissions plus plain-language policy rules.
- Inline rule authoring with suggested rules and conflict warnings.
- Explain mode for a deeper description and risk assessment.
- Separate models for preflight classification and policy evaluation.

## Config files

- Persistent settings: `~/.pi/agent/extensions/bo-pi/preflight.json`
- Workspace rules: `.pi/preflight/settings.local.json`
- Global rules: `~/.pi/preflight/settings.json`
- Debug log: `.pi/preflight/logs/preflight-debug.log`

## More docs

- [Full preflight guide](../docs/preflight.md)
- [Repo README](../README.md)
- [Releasing](../RELEASING.md)
