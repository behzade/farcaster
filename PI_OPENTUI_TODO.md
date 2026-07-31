# Pi OpenTUI tasks

This file is the handoff for the next work session. Keep
`PI_OPENTUI_PLAN.md` for design choices and use this file for open work.

## Current state

- `apps/pi-opentui` is the canonical Pi client.
- The working tree has an uncommitted prompt-history fix. Recalled messages
  now place the cursor at the end.
- Before this file was added, `bun run typecheck` and all 97 tests passed.
- Do not discard unrelated working-tree changes.

## 1. Fix selection copying

Selected transcript text can be highlighted, but copying it does not work.

Tasks:

- Trace OpenTUI's renderer-level selection event and its clipboard contract.
- Copy the exact selected plain text. Do not copy ANSI color codes, hidden
  markdown state, clipped tool data, or unrelated rows.
- Respect Pi's keybinding manager. Do not add another raw key table.
- Keep the terminal's normal copy binding working when OpenTUI has no active
  selection.
- Check transcript prose, markdown, code blocks, tool output, and composer
  text.
- Add mouse-selection and copy tests with OpenTUI's mock mouse input where the
  test renderer supports them. Keep host clipboard commands behind a small
  service so tests do not touch the real clipboard.

Done when:

- Dragging over text and using the configured copy key places that text on the
  clipboard.
- Copy does not quit, clear, submit, or change the draft.
- Empty selection leaves terminal copy behavior alone.

## 2. Restore composer focus after mouse use

After clicking the transcript or another non-input area, typing does nothing
until the composer is clicked again.

Tasks:

- Define one focus-owner state for composer, dialog, command menu, subagent
  list, and other future interactive views.
- When no modal or list owns input, route a normal typing key back to the
  composer and apply that same key. The first character must not be lost.
- A plain click outside the composer may also return focus when it does not
  begin or extend text selection.
- Do not steal focus from dialogs, secret input, command and model search,
  active text selection, or the subagent list.
- Add tests for click-away then type, selection then copy, dialog focus, and
  command-menu focus.

Done when:

- A click elsewhere never leaves the app unable to accept a prompt.
- The first key after focus recovery appears once in the composer.
- Existing modal and menu keyboard tests still pass.

## 3. Restore the subagent list below the composer

The installed `pi-subagents` v0.36.0 extension still contains the prior
behavior:

- `src/tui/fleet-status.ts` registers a `belowEditor` widget.
- Down enters the roster when the composer is empty.
- Up and Down select `main` or a child.
- Enter opens the fleet inspector for the selected child.
- Escape returns to the composer.
- `src/tui/fleet.ts` contains the live roster and transcript inspector.

The full source comes from `nix/pi-subagents.nix`; this repository stores the
pin and local patches, while the installed source is linked at
`~/.pi/agent/extensions/subagent`.

The OpenTUI extension bridge currently blocks this feature:

- `onTerminalInput` returns an inert unsubscribe function.
- `setWidget` does nothing.
- `custom` rejects every custom view.

Tasks:

- Add a typed input-owner and extension-input path. Extension handlers must be
  able to consume a key before the composer handles it.
- Add an OpenTUI below-composer roster with the same selection rules and live
  elapsed-time and token updates.
- Add an OpenTUI fleet inspector with live child state and transcript output.
- Reuse the extension's run data and rules. Do not copy subagent process,
  session, or status logic into the frontend.
- Do not import old `pi-tui` render components into the OpenTUI view layer.
  Add a frontend-neutral state contract to the pinned extension through a
  local patch if the current API cannot expose the data cleanly.
- Make Up and Down precedence explicit: completion menu, multiline cursor,
  prompt history, then subagent roster when the composer is empty.
- Add tests for activation, selection, inspection, escape, child completion,
  and focus return.

Done when:

- Active subagents appear below the composer.
- Down reaches the list, Enter opens the selected child, and Escape returns to
  the composer.
- The roster updates without rebuilding the transcript or blocking input.

## 4. Finish the extension UI contract

Audit every method in `apps/pi-opentui/src/services/extension-ui.ts`. Silent
no-ops hide missing features. Each installed extension UI call must do one of
these:

- map to a typed app command or state value;
- render through an owned OpenTUI component;
- return a clear unsupported-operation error.

Start with `onTerminalInput`, `setWidget`, and `custom`, since the subagent UI
needs them. Then check `setFooter`, `setHeader`, working-indicator methods,
autocomplete providers, and editor replacement.

Add a contract test that records every UI method called by the installed
extensions during smoke flows and fails when a call reaches an unmarked
silent stub.

## Checks

Run from `apps/pi-opentui`:

```sh
bun run typecheck
bun test
bun src/smoke.ts
```

Also run from the repository root:

```sh
git diff --check
```

Do not use a Nix build or temporary Nix cache for routine OpenTUI checks.
