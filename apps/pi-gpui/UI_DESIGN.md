# Pi GPUI interface redesign

**Status:** aligned design specification

**Scope:** visual hierarchy, proportions, information design, and component behavior

**Constraint:** preserve the existing left / center / right structure and the composer's bottom placement

## 1. Agreed direction

This is a refinement of the current interface, not a new product model.

Keep:

- the left session sidebar;
- the central chronological event timeline;
- the right session-information sidebar;
- the composer at the bottom of the center;
- all current composer input, queue, attachment, picker, and submission behavior;
- the current dark theme and its existing color character;
- the current selected/unselected session treatment;
- the existing Done, Working, and Needs input session states.

Change:

- stop the sidebars from becoming disproportionately wide;
- remove the center header completely;
- remove the right-sidebar header completely;
- redesign the left header around search and project selection;
- group adjacent sessions by project instead of repeating the folder icon and name;
- make the center read clearly as a hierarchy of timeline events;
- remove the right-sidebar Status section;
- compress Usage into a glanceable context summary;
- make active subagents operationally useful;
- separate completed subagents similarly to Archived sessions;
- add working-copy changed files that open in embedded Neovim;
- use a clear sans-serif for interface and prose, reserving monospace for technical content;
- rebuild the composer as a three-band terminal console with session context, a generous input surface, and labeled controls;
- correct the composer's cursor-reveal behavior for long input.

## 2. Information hierarchy

### Center timeline

The visual priority is fixed:

1. **User messages**
2. **Pi's user-facing messages**
3. **Diffs**
4. **Tool calls and tool output**

Chronology remains intact. Lower-priority events are not moved elsewhere; they are presented more quietly.

### Left sidebar

1. Search and new session
2. Project filter and add project
3. Project identity
4. Session title
5. Done / Working / Needs input state
6. Relative time and other metadata
7. Archived sessions

### Right sidebar

1. Compact context usage
2. Active subagents
3. Session-wide file changes
4. Completed subagents
5. Detailed token/accounting data on demand

There is no standalone session Status section. Session state is already obvious from the left list, active events, composer controls, and subagent activity.

## 3. Overall layout

### Wide desktop wireframe

```text
┌──────────────────────┬──────────────────────────────────────────────────────────────┬────────────────────────┐
│ ⌕  Search         ◧  │                                                              │  CONTEXT               │
│ ▣  All projects ▾  + │  YOU · 10:42                                                │   ◜ 42% ◝   113k / 272k │
│──────────────────────│  Won't this cause the rebuild of the entire dependency      │   ◟     ◞   $6.53       │
│ ▾ pi                 │  graph on every change?                                     │   Usage details ›      │
│                      │                                                              │────────────────────────│
│   Working       2m   │  PI · 10:43                                                 │  ACTIVE SUBAGENTS  2   │
│   If you create a…   │  No. The package now resolves the local source directly…    │                        │
│                      │                                                              │  ◉ Reviewer       1m   │
│   Done         18m   │  ┌ nix/pi-gpui.nix                         +8  −4  Expand ┐  │  Checking package      │
│   Archived session…  │  │ …diff content…                                         │  │  ◈ Read · active       │
│                      │  └──────────────────────────────────────────────────────────┘  │  12 tools · 139.9k tok │
│ ▾ behzad             │                                                              │                        │
│                      │  ▸ Bash  cargo test --manifest-path…            ✓ 14s        │  ◉ Scout          42s  │
│   Needs input  21m   │  ▸ Read  nix/pi-gpui.nix                       ✓             │  Tracing dependency    │
│   nix-config on…     │                                                              │  ◈ Search · active     │
│                      │  PI                                                          │  5 tools · 31.2k tok   │
│                      │  The focused test passed. The full rebuild was not run…      │────────────────────────│
│                      │                                                              │  CHANGES  3 FILES      │
│                      │                                                              │  +83  −21              │
│                      │                                                              │  M shell.rs    +21 −8  │
│                      │                                                              │  M session.rs  +54 −9  │
│                      │                                                              │  A tests.rs     +8 −4  │
│                      │                                                              │────────────────────────│
│ Archived · 59        ├──────────────────────────────────────────────────────────────┤  Completed agents · 3 ›│
│                      │ Working · Session 01a00f0b · turn 1                          │                        │
│                      │ Ask Pi                                                       │                        │
│                      │ Provider · Model · Effort                     Abort   Steer   │                        │
└──────────────────────┴──────────────────────────────────────────────────────────────┴────────────────────────┘
```

The center and right content begin at the top of the window. They do not reserve vertical space for headings that repeat context.

### Proportions

- Left sidebar: `272px` preferred, `248–304px` resizable.
- Right sidebar: `312px` preferred, `288–344px` resizable.
- Center: consumes all remaining width.
- Composer: `248px` minimum height, organized as a `40px` status strip, flexible input body, and `64px` control footer.
- Standard outer padding: `16px` in sidebars, `20–24px` in the timeline.
- Base spacing unit: `4px`; use `8 / 12 / 16 / 24 / 32px` steps.

The sidebars use fixed preferred widths and maximums. They must not scale proportionally with a large window. Extra width belongs to the central timeline and wide diff content.

## 4. Left sidebar

### 4.1 Utility header

Replace the current standalone Pi row and oversized search treatment with two compact action rows. The information hierarchy follows the supplied reference, not its colors.

```text
┌────────────────────────────────────┐
│ ⌕  Search sessions              ◧ │
│ ▣  All projects                ▾ + │
└────────────────────────────────────┘
```

First row:

- search icon;
- `Search sessions` label or placeholder;
- New session action aligned right.

Second row:

- folder/project icon;
- active filter, normally `All projects`;
- disclosure arrow;
- Add/open project action aligned right.

Rules:

- Each row is `40–44px` high.
- The entire Search area is interactive, not only the icon.
- New session and Add project use distinct icons and tooltips.
- Search can expand in place or focus a search field without changing the list's position.
- The project selector filters the grouped list; it does not duplicate a second navigation structure.
- Remove the standalone `Pi` label. The window and application already provide product identity.

### 4.2 Project grouping

Do not repeat a folder icon and project name in every consecutive session row.

```text
▾ pi

  Working                                      2m
  If you create a new session and immediately…

  Done                                        18m
  Archived session should not show Done

▾ behzad

  Needs input                                 21m
  nix-config on HEAD…
```

Project-group rules:

- One project heading owns the sessions beneath it.
- Project heading contains folder icon, project name, and collapse control.
- Project headings are visually quieter than selected session titles but stronger than metadata.
- Collapsing a project does not change session state.
- Project groups with active sessions sort ahead of entirely inactive groups unless the user has chosen another explicit sort.
- When one project is selected from the filter, the heading remains visible for context.

### 4.3 Session rows

Preserve the existing selected/unselected treatment and state vocabulary.

Each row contains:

- Done, Working, or Needs input state;
- session title;
- relative update time.

Generate a semantic title after the first completed exchange when the session has no explicit name. Manual inline edits always take precedence.

Presentation rules:

- Two lines maximum.
- State and time share the quiet metadata line.
- Title is the stronger line.
- Working and Needs input may use the existing semantic colors.
- Done remains visible for open/recent sessions.
- Do not add the project name or folder icon inside the row.
- Selected state uses the current surface treatment; avoid adding another heavy border or badge.

### 4.4 Archived sessions

Keep Archived as the quiet bottom section it is now.

```text
Archived · 59                                      +
```

Its current separation from live/recent sessions is useful. Do not mix archived rows into active project groups unless the user explicitly opens Archived.

## 5. Center event timeline

### 5.1 Remove the center header

Remove the current title/project header completely.

Reasons:

- it repeats information already available in session navigation;
- the current title is not yet a generated semantic title;
- it delays access to the timeline;
- its height is disproportionate to its usefulness.

The first event begins after normal timeline padding at the top of the window.

### 5.2 Timeline structure

Keep one chronological stream. Do not split conversation or tools into separate views.

Use spacing, typography, surfaces, and expansion state to communicate importance. A faint timeline guide or aligned event markers may reinforce chronology, but should not become decoration.

```text
YOU · 10:42
┌──────────────────────────────────────────────────────────────┐
│ Won't this cause the rebuild of the entire dependency graph │
│ on every change?                                            │
└──────────────────────────────────────────────────────────────┘

PI · 10:43
No. The package now resolves the real local path sources directly…

Edit · nix/pi-gpui.nix                                      +8 −4

▸ Bash · focused regression test                         ✓ 14s
▸ Read · nix/pi-gpui.nix                                  ✓

PI
The focused test passed. The full rebuild was not run because…
```

### 5.3 User messages

User messages are the strongest timeline event.

- Use a subtle full-width tinted surface from the existing theme.
- Add a clear `You` label and optional timestamp.
- Use the interface sans-serif, not monospace.
- Use medium weight and comfortable prose line height.
- Maintain enough separation before and after the message to define a new turn.
- Do not use a rounded chat bubble or right alignment.

The treatment should communicate “this initiated the work,” not “this is casual chat.”

### 5.4 Pi user-facing messages

Pi's actual response is the second-highest event.

- Use the interface sans-serif.
- Give it a clear `Pi` label.
- Use primary text color and a readable line length when practical.
- Do not place normal prose inside a heavy card.
- Separate final conclusions from preceding tool activity with additional vertical space.
- Links, file paths, and inline code remain visually distinct and actionable.

Streaming should not cause surrounding events to jump horizontally or shift their alignment.

### 5.5 File changes

Edit and write tools show a compact path and change-count row. They do not render diffs inline. The file action opens the target in embedded Neovim.

### 5.6 Tool calls

Tool calls are supporting evidence, not the main story.

Default row:

```text
▸ Bash · focused GPUI regression test         cargo test …       ✓ 14s
```

Expanded row:

```text
▾ Bash · focused GPUI regression test                            ✓ 14s
  cargo test --manifest-path apps/pi-gpui/Cargo.toml …
  18 passed · 0 failed
```

Rules:

- Closed by default after successful completion.
- Active tools remain open enough to show what is happening.
- Failures and permission/attention requests remain expanded.
- Human-readable tool purpose uses sans-serif.
- Command, raw arguments, and output use monospace.
- Tool name, purpose, status, duration, and disclosure control fit on one compact row.
- Use borders or spacing rather than filled cards for routine tools.
- Multiple small completed calls may collapse into a chronological summary, but their original positions remain recoverable.

## 6. Right sidebar

### 6.1 Remove header and Status

Remove:

- the `SESSION` header;
- the full Status section;
- the State / Ready or State / Working row.

The right sidebar starts directly with compact usage. This preserves vertical space for subagents and file changes.

### 6.2 Compact context usage

Rename `Main context` to `Context`.

Recommended design:

```text
CONTEXT
┌──────────────────────────────────────┐
│   ◜ 42% ◝   113k / 272k             │
│   ◟     ◞   159k remaining          │
│             $6.53       Details ›   │
└──────────────────────────────────────┘
```

Use a small circular progress indicator because it communicates context fill at a glance within a narrow panel. The exact count remains adjacent for precision.

Rules:

- Percentage is the strongest value.
- Show used/total and remaining context.
- Cost is quiet but visible.
- The entire summary should occupy roughly `72–88px`, not a long table.
- Do not show input, output, cache, or cache-hit rate by default.
- `Details` expands cumulative token accounting in place or in a small popover.
- Context-window usage and cumulative session token usage remain clearly separated.
- Near-limit context uses the existing warning color; normal context uses the existing neutral/accent treatment.

Expanded details may contain:

```text
Total usage     7.5m
Input         418.5k
Output         31.1k
Cache read       7.0m
Cache hit        99.3%
```

### 6.3 Active subagents

Active subagents are the most detailed persistent section in the right sidebar.

```text
ACTIVE SUBAGENTS · 2

┌──────────────────────────────────────┐
│ ◉  Reviewer                 Working │
│    Checking package behavior        │
│                                      │
│ ◈  Read · nix/pi-gpui.nix           │
│ 12 tools · 139.9k tokens · 1m 11s   │
└──────────────────────────────────────┘

┌──────────────────────────────────────┐
│ ◉  Scout                    Working │
│    Tracing dependency sources       │
│                                      │
│ ◈  Search · active                  │
│ 5 tools · 31.2k tokens · 42s        │
└──────────────────────────────────────┘
```

Each active subagent exposes:

- role;
- role icon;
- current state;
- useful current activity;
- current or most recent tool;
- tool-call count;
- token usage;
- elapsed time.

Rules:

- Use distinct, simple role icons: reviewer, scout, worker, researcher, or other actual roles.
- Icons supplement labels; they never replace role text.
- Current activity gets two lines maximum.
- Current tool is a compact secondary row.
- Token and tool counts are visible but visually subordinate to role and activity.
- Needs input, blocked, and failed agents rise to the top of the active list.
- Clicking an agent opens its detailed activity without replacing the root timeline.
- Avoid a separate card for the root/Main agent unless it is genuinely represented as an agent with independent activity.

### 6.4 Completed subagents

Treat completed subagents similarly to Archived sessions: separate, quiet, and collapsed by default.

```text
Completed agents · 3                              ›
```

When expanded:

```text
Reviewer     Complete · 1m 11s · 139.9k
Scout        Complete · 42s · 31.2k
Worker       Failed · 18s · 8.4k
```

- Preserve outcome, role, duration, and token usage.
- Do not continue displaying a completed agent's last tool as if it were active.
- Failed or incomplete outcomes remain visually distinguishable.
- Completed agents remain inspectable.

### 6.5 Working-copy changes

Show the complete working copy rather than a session-filtered repository view.

```text
JJ / Git    +83  −21    uyzmnoqm*

M  apps/pi-gpui/src/shell.rs
M  apps/pi-gpui/src/session.rs
A  apps/pi-gpui/src/shell_tests.rs
                                      18 more files  ›
```

Rules:

- Keep the header flat, without a card, border, scope selector, or `Working` label.
- `JJ / Git` is a two-state text toggle; the active backend is strong and the inactive backend is muted but clickable.
- Header totals describe the complete working copy.
- Show the Jujutsu change ID or Git branch followed immediately by `*` only when the working copy is dirty.
- Show five file rows initially. Each expansion appends at most twenty more below the existing rows.
- Use monospace for paths and counts.
- Truncate from the middle when a path is too long so the filename remains visible.
- A deleted file, rename, or binary file receives a clear textual state.
- Clicking anywhere on a file row opens it in embedded Neovim; there is no separate pencil action.
- Show session-only additions and deletions in the composer immediately before cost.

## 7. Composer

Use a restrained terminal-console treatment built from three edge-to-edge bands:

```text
┌───────────────────────────────────────────────────────────────────────┐
│ ● WORKING · project · Session 01a01be7 · turn 1                     │
├───────────────────────────────────────────────────────────────────────┤
│ Ask Pi                                                                │
│                                                                       │
├───────────────────────────────────────────────────────────────────────┤
│ PROVIDER        MODEL                 EFFORT                Abort/Send │
│ openai-codex    GPT-5.6 Sol           High                            │
└───────────────────────────────────────────────────────────────────────┘
```

Rules:

- The status strip shows semantic state, project, compact session ID, and user-turn count from real runtime data.
- Do not invent memory, sandbox, or lock metadata that the runtime does not expose.
- The input body is the dominant region and keeps the existing textarea behavior and `Ask Pi` placeholder.
- Provider, model, and effort are vertically labeled cells with their existing keyboard-accessible dropdown behavior.
- Keep Abort and Send/Steer at the far right; Abort uses the existing Phosphor stop icon and danger color.
- Use the theme's system font for labels and Lilex for technical metadata, input, and selected values.
- Preserve queues, extension widgets and status, attachments, slash commands, file mentions, image paste, history, and submission behavior.
- Empty composers use the complete rounded frame. Attached composers retain the edge treatment needed to join the transcript cleanly.

### Cursor and scrolling correction

The current long-input behavior is incorrect: after enough text is pasted or typed, the caret is not visible, then each new keystroke scrolls the editor incrementally toward it.

Required behavior:

- After paste, immediately reveal the caret line.
- While typing, keep the caret fully visible within the editor viewport.
- Scroll only when the caret crosses the visible viewport boundary.
- Do not move the viewport a small amount on every keystroke.
- Preserve the user's manual scroll position until typing or navigation requires caret reveal.
- Up/down and page navigation keep the caret and selection visible.
- Resizing the composer recalculates the minimum scroll needed to reveal the caret once.

This is a behavior fix, not a visual redesign.

## 8. Typography

Use a clear sans-serif for all nontechnical interface and user-facing prose.

### Sans-serif

Use for:

- navigation;
- project and session labels;
- status labels;
- user messages;
- Pi's user-facing messages;
- section headings;
- buttons and controls;
- subagent roles and activity descriptions;
- usage labels.

Recommended family: the platform system UI sans, with Inter as a consistent cross-platform fallback if needed.

Suggested scale:

- `15px / 22px` — user and Pi prose;
- `14px / 20px`, medium — session titles and active agent role/activity;
- `13px / 18px` — navigation and controls;
- `12px / 16px` — metadata;
- `11px / 14px`, medium — uppercase section labels.

### Monospace

Keep the current coding monospace for:

- code;
- diffs;
- commands;
- raw tool output;
- file paths;
- token counts where alignment helps;
- technical IDs.

A tool row may mix both: human-readable purpose in sans-serif and raw command in monospace.

## 9. Theme and visual treatment

Keep the current theme as-is for this redesign.

Do not introduce a new palette, brand accent, gradient, shadow system, or radically different corner treatment.

Improve hierarchy using the existing theme through:

- text weight;
- type family;
- opacity and contrast;
- spacing;
- one-pixel dividers;
- subtle selected/event surfaces;
- restrained semantic status color.

Guidelines:

- User messages receive the strongest nonsemantic surface treatment.
- Pi prose uses strong text without a heavy enclosing card.
- Diffs retain existing red/green semantics.
- Tool calls use quieter text and borders.
- Usage accounting is visually muted.
- Active and Needs input states remain recognizable without coloring entire panels.
- Do not rely on color alone; keep state words and icons.

## 10. Responsive behavior

### Wide windows

- Both sidebars remain visible at their preferred widths.
- Sidebars stop growing at their maximum widths.
- Center receives extra space.
- Timeline prose may retain a comfortable reading width while diffs use more of the center.

### Medium windows

- Left sidebar remains visible.
- Right sidebar may narrow to its minimum.
- Diff defaults to unified view if split columns become unreadable.

### Narrow windows

- Preserve the three sections conceptually.
- Right sidebar becomes an overlay/drawer before compressing the center below a usable width.
- Left sidebar may collapse to a temporary overlay.
- The composer remains attached to the center timeline.

## 11. Information removed or demoted

Remove entirely:

- center title/project header;
- right `SESSION` header;
- right Status section;
- repeated project icon/name in consecutive session rows;
- standalone Pi row in the left header.

Collapse or demote:

- input/output/cache/cache-hit details;
- completed subagents;
- successful tool output;
- cumulative accounting relative to context health.

Keep prominent:

- session states in the left list;
- user messages;
- Pi's user-facing responses;
- active subagent role and activity;
- context percentage;
- working-copy changed files;
- Needs input, blocked, and failed states.

## 12. Delivery order

### Phase 1 — structure and proportions

- Remove center and right headers.
- Replace the left header with Search/New session and Project/Add project rows.
- Add fixed/min/max sidebar widths.
- Group sessions by project.

### Phase 2 — timeline hierarchy

- Introduce sans-serif UI/prose typography.
- Strengthen user-message treatment.
- Improve Pi response spacing and labels.
- Keep file-change actions compact and open them in embedded Neovim.
- Compact and collapse tool rows.

### Phase 3 — right sidebar

- Remove Status.
- Build compact circular Context summary.
- Expand active subagent information.
- Add completed-subagents section.
- Add the flat working-copy list with rows that open in embedded Neovim.

### Phase 4 — composer behavior and polish

- Build the three-band composer and widen the empty-state frame.
- Correct caret reveal and scrolling for long input.
- Verify focus restoration from embedded Neovim.
- Test long paths, many agents, many projects, and many changed files.
- Tune contrast and spacing without changing the theme.

## 13. Acceptance criteria

The redesign is aligned when:

1. The major sections remain left sessions, center timeline, right session information, and bottom composer.
2. There is no center header and no right-sidebar header.
3. The left header clearly presents Search/New session and Project/Add project as two action rows.
4. Consecutive sessions do not repeat the same project name and folder icon.
5. Existing Done, Working, and Needs input states remain visible.
6. The center hierarchy reads as user message, Pi response, then tool activity.
7. Tool calls remain chronological but do not compete visually with user-facing messages.
8. The right sidebar contains no redundant Status section.
9. Context usage is understandable at a glance and detailed usage remains available on demand.
10. Every active subagent shows role, activity, current/recent tool, tool count, token usage, and elapsed time.
11. Completed subagents are separated and collapsed similarly to Archived sessions.
12. The working-copy section has a flat JJ/Git toggle, aggregate totals, dirty identity, and paged file rows; session totals appear before composer cost.
13. Clicking a changed file row opens it in embedded Neovim without a separate pencil button.
14. The composer has a status strip, dominant input body, and labeled control footer without losing existing behavior.
15. Long composer input keeps the caret visible without per-keystroke scroll creep.
16. Nontechnical interface and prose use sans-serif; code, commands, and raw tool output use monospace.
17. The existing theme remains recognizable and materially unchanged.
18. Sidebars remain proportional on very wide windows because they stop growing at defined maximums.
