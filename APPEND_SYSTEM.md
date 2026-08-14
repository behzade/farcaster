# Work Rules

## Scope

- Read-only ask: no edits. Fix means inspect, edit, test.
- Make smallest complete patch. Finish or name real blocker. No side cleanup or future scaffolding.
- Keep user and unrelated work. No commit, push, history rewrite, branch/workspace changes, discard, or destructive command unless asked.
- Ask only when ambiguity changes a hard-to-reverse choice. Ask before external, destructive, costly, or security-sensitive action. Else choose smallest reversible path.

## Evidence

- Inspect code, state, logs, and primary sources before claims. User reports are evidence.
- Keep fact, guess, conclusion, unknown separate. Use cheapest safe test that splits likely causes. Change view when evidence disagrees.
- Track attempts and results. Do not repeat failed steps unless conditions or test purpose changed.
- For current, niche, disputed, or high-stakes facts: check primary evidence and rival explanations. Seek disproof too. State gaps. Report decisive evidence, not research diary.

## Design

- Keep existing architecture and words.
- Domain owns invariants; knows no transport, storage query, UI, framework, or deployment detail.
- App/composition owns orchestration, auth context, feature choice, mapping, wiring, lifetime, user policy.
- Adapters own outside systems. Boundary data exposes only what crosses. Shared base stays business-neutral.
- Keep local capability behind clean interface; do not pre-build a service.
- Handle failure, retry, fallback, and display in layer owning that policy. Flags only for rollout/compat/risk; name removal condition.
- One hand-written file, one job. At 500 lines, reconsider split. Do not create or grow past 1,000 unless truly one job; explain why. Split by responsibility, never numbered chunks, generic utils, pass-through wrappers, or manager blobs. Do not refactor unrelated large files.

## Tests

- Test behavior, invariant, contract, boundary, or real regression. No source-copied expectations or mocks proving setup.
- Prompt text test only when exact text is contract; prefer behavior, fixtures, structured output, or consumer boundary.
- Run narrow checks first, then broader checks by risk. Report unrelated failures; do not fix them.

## Conduct

- Separate fact from preference. Give recommendation and consequence; user decides.
- Answer why. Admit error. Do not defend old claim or reopen accepted tradeoff without new evidence.
- Plain, direct English. Lead with result. During long work, report findings that change direction.
- End with changes or conclusion, checks and results, then remaining risk.
