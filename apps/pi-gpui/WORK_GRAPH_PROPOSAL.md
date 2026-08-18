# Pi GPUI durable work graph proposal

**Status:** proposal

## Goal

Add a durable issue graph to Pi GPUI so work can continue across sessions and
agents without relying on transcript history.

A user starts a Pi session and asks for work. Once the model has investigated
enough to understand the concrete task, it can create an issue or link an
existing one, do the work, record the result, and mark the issue done.

If a prerequisite appears during the work, the model can create or link that
issue as a dependency, work on it first, then return to the original issue. The
model can keep adding useful follow-up work to the graph as it learns more.

The graph should make these actions direct:

- hand work to another session or agent;
- continue an existing issue without restating it;
- see ready, blocked, active, and completed work;
- view the same state as a dependency graph or Kanban board; and
- answer what concrete work was done in a project.

This is structured project memory: it keeps intent, progress, dependencies,
results, and useful project facts outside any one Pi session.

## Product boundary

Pi continues to own agents, sessions, prompts, tools, transcripts, the sandbox,
and execution. The work graph stores durable project and task state used by Pi
and the user.

The graph includes:

- projects;
- issues and their state;
- dependency links;
- progress and handoff notes;
- recorded outcomes;
- links between issues and Pi sessions;
- evidence attached to work; and
- reusable project facts linked to their source work.

Issue state does not depend on a session staying alive. A completed issue should
record what was done so project-history queries return concrete results rather
than only issue titles.

## Model interaction

Pi exposes native tools that let the model query and update the graph. They
support the product flow without making the model restate issue context in the
conversation.

The intended flow is:

1. The user asks for work in a Pi session.
2. The model investigates until the task is concrete.
3. The model links an existing issue or creates one.
4. The model records progress while it works.
5. When it finds a prerequisite, it links or creates the dependency and works
   on that first.
6. After the dependency is done, it returns to the prior issue.
7. When the requested work is complete, it records the outcome and marks the
   issue done.

Linking an existing issue should load enough durable context to continue it
without a new explanation. Parking work should leave enough current state for a
later session or agent to resume it.

## Native UI

Pi GPUI presents the graph as part of the same app as its sessions. The native
UI should allow the user to:

- see which issue a session is working on;
- link a session to an existing issue;
- open issue details and dependencies;
- move between related work;
- inspect project history; and
- switch between graph and Kanban views of the same state.

The exact placement and layout are not decided by this proposal.

## What to retain from Issues

Reuse or adapt the parts of Issues that support the durable graph:

- issue records and states;
- dependencies and ready/blocked/next planning;
- notes, history, and recorded outcomes;
- durable writes and events;
- issue UI work; and
- useful native UI patterns already adapted by Pi GPUI.

## What to remove from Issues

The combined product does not retain Issues' source-control and workspace
product:

- Rift;
- isolated and shared work modes;
- workspace preparation, observation, review, restoration, landing, and
  cleanup;
- workspace claims and workspace context files;
- Git and JJ repository discovery or commands;
- base and source revision management; and
- review snapshots derived from managed workspaces.

Pi may display files, diffs, checks, commits, or other evidence produced during
a session, but the work graph does not run source-control or workspace actions.

## Open decisions

This proposal does not yet decide:

- the exact storage and module layout;
- how Pi's model tools reach the graph implementation;
- the exact native UI layout;
- the final issue states and tool schemas;
- when Pi should create an issue automatically.

Those choices require a separate design based on this product boundary.
