# Work graph proposal

**Status:** proposal

## Goal

A plan is a reusable DAG that outlives a session. Work is walking that graph.
A walker is stopped at a node. Someone else continues from that node.

Status is not stored. Position in the graph is the only state.

## Graph

A task is either part of an already existing graph, or it creates one.

The graph describes:

- current state as the initial node;
- final state as the final node;
- concrete steps in between.

In-between nodes are added one by one. The graph does not have to be complete
up front. Work can start as three nodes and grow as complexity is discovered.

The graph is a DAG. If a path is wrong, the walker goes back a few nodes,
reverts those commits, and continues on the other path.

Some nodes become detached. Detached nodes are cancelled.

## Node

A node has only enough information for that step:

- a minimal list of files or folders;
- what the node entails, which is when you can move one step forward.

## Session

A session is either handed a node, or it creates one if the work requires it.

From a handed node, the session can go forward, go backward, and read.
