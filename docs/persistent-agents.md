# Persistent agents

Choose **New agent** in a project, give it a name, and return to the same sidebar entry weeks later. One searchable timeline spans harness sessions. Stop interrupts the current turn. Reopening shows where work stopped and waits for the next message.

**Continuity.** Give the agent a stable ID and link its existing Farcaster session IDs in order. Resume the active session when possible. At a context limit, on failed resume, or when the user chooses **Fresh context**, start a session from a bounded, editable handoff: goal, decisions, unfinished work, and links to the original messages. Save the handoff and last completed turn together before switching. After a crash, mark an unfinished action interrupted and let the user decide whether to retry.

**Attachments.** Keep the thumbnail on its original message. Reuse the image store and send only a text reference scoped to this agent. When asked to inspect it, resolve that reference in an isolated vision call and return text to the main thread. Clicking **Use again** selects the original file. If the adapter cannot inspect it, say so. Deleting an attachment revokes its reference and frees the blob after its last reference disappears.

**First slice.** Local Codex. Attach a screenshot, ask about it, force a context rollover, restart Farcaster, then ask about a detail the handoff omitted. Verify the same sidebar entry opens, the agent can inspect the original again, and unrelated model requests contain no image payload.
