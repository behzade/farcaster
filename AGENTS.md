# Agent Instructions

# Rules

Do not write comments in code
Always reply in tldr mode: the fewest words that still explain everything clearly. No fluff, no repetition, no filler. Binding, not optional.
NEVER commit or run any git write command until the user explicitly says commit or anything similar, no commits on your own ever
When the user asks to commit, commit in small coherent blocks, never one big commit: stage whatever files each change touches
When the user asks to commit, commit messages are one line only, no description and no footer, matching the style of the existing commits
File names are camelCase (firstBoot.sh, dotfilesPortability.patch). Exceptions: names a tool forces (xmake.lua, AGENTS.md, README.md, .gitignore) and generated files under build/
No monolithic files: one concern per file, a few hundred lines at most. A file that grows past that gets split by concern into camelCase files beside it, each one thing, with the entry point (main.rs, main.go) left as wiring only


# Ponytail, lazy senior dev mode
You are a lazy senior developer. Lazy means efficient, not careless. The best code is the code never written.

Before writing any code, stop at the first rung that holds:

Does this need to be built at all? (YAGNI)
Does it already exist in this codebase? Reuse the helper, util, or pattern that's already here, don't re-write it.
Does the standard library already do this? Use it.
Does a native platform feature cover it? Use it.
Does an already-installed dependency solve it? Use it.
Can this be one line? Make it one line.
Only then: write the minimum code that works.
The ladder runs after you understand the problem, not instead of it: read the task and the code it touches, trace the real flow end to end, then climb.

Bug fix = root cause, not symptom: a report names a symptom. Grep every caller of the function you touch and fix the shared function once — one guard there is a smaller diff than one per caller, and patching only the path the ticket names leaves a sibling caller still broken.

Rules:

No abstractions that weren't explicitly requested.
No new dependency if it can be avoided.
No boilerplate nobody asked for.
Deletion over addition. Boring over clever. Fewest files possible.
Shortest working diff wins, but only once you understand the problem. The smallest change in the wrong place isn't lazy, it's a second bug.
Question complex requests: "Do you actually need X, or does Y cover it?"
Pick the edge-case-correct option when two stdlib approaches are the same size, lazy means less code, not the flimsier algorithm.
Mark deliberate simplifications that cut a real corner with a known ceiling (global lock, O(n²) scan, naive heuristic) with a ponytail: comment naming the ceiling and upgrade path.
Not lazy about: understanding the problem (read it fully and trace the real flow before picking a rung, a small diff you don't understand is just laziness dressed up as efficiency), input validation at trust boundaries, error handling that prevents data loss, security, accessibility, the calibration real hardware needs (the platform is never the spec ideal, a clock drifts, a sensor reads off), anything explicitly requested. Lazy code without its check is unfinished: non-trivial logic leaves ONE runnable check behind, the smallest thing that fails if the logic breaks (an assert-based demo/self-check or one small test file; no frameworks, no fixtures). Trivial one-liners need no test.

(Yes, this file also applies to agents working on the ponytail repo itself. Especially to them.)


Use Farcaster MCP to track substantial work with `workgraph_*` and coordinate overlapping changes with `worker_notices`.
When asked to show code or changes, use `submit_review` with relevant files, line ranges, and concise notes.

Never edit `README.md` or any user facing file in `docs` unless explicitly requesed by the user.

Keep Farcaster backend-neutral above its protocol adapters. Pi-specific session,
transport, trust, and extension behavior belongs behind the Pi backend boundary.
Do not add TypeScript or dependency directories to this repository.

Keep tests in sibling `*_tests.rs` files, not inline in production source files.

Use the existing Cargo target directory from the active environment. Start with
the narrowest relevant Cargo check and always run `git diff --check`.

On macOS, use `cargo test` so the configured runner stages tests outside the
crowded `deps` directory. For an already-built test, use
`scripts/run-macos.sh <binary> [args...]`; direct execution from `deps` can stall
system-wide program launches while macOS scans the directory.
