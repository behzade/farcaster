# Suggested reviews

An agent can call Farcaster MCP's `submit_review` to leave a review card in its
chat transcript:

```json
{
  "title": "Check retry handling",
  "items": [
    {
      "path": "src/request.rs",
      "start_line": 42,
      "end_line": 68,
      "note": "Check that retries stop after cancellation."
    }
  ]
}
```

Paths are relative to the authenticated caller's project. Lines are optional,
1-based and inclusive; an end line requires a start line. Supply 1–100 locations,
a title of at most 200 bytes, and single-line notes of at most 1000 bytes.
Absolute paths, traversal, and symlinks escaping the project are rejected.

These are suggested locations, **not a verified changeset**. Submitting does not
open the editor, run a command, generate a diff, or modify project files. The
artifact is carried in the normal tool result and restored with tool history;
it is not a separate change-tracking database.

During a run the card stays where submitted, marked **Agent still working**.
Once the run settles, its reviews appear below the final response, without
leaving duplicate cards in the activity list. Multiple reviews remain separate.
If tools or thinking followed a submission, the card notes **Submitted before
the agent finished**; the review does not claim to cover that later work.
Restored histories use user-message boundaries when live run boundaries are
unavailable. Without a final response, a review remains at its submission point.

Click the Neovim icon beside the amber review title and location count to open
a new quickfix list. Its tooltip is **Open review in Neovim**. Clicking anywhere
else on the header expands or collapses the paths, line bands, and notes;
clicking a location opens a list for just that location. Use Enter in quickfix to visit a valid entry,
`:cnext` / `:cprevious` to navigate, and `:colder` to return to the previous list.
Missing files and ranges beyond the current file are marked invalid with a
warning. Unsaved buffers are preserved and flagged. Ranges can still become
stale after edits even when they remain within the file's line count.
