# Transcript activity and changed files

Completed tool calls and thinking share an Activity summary between messages.
User messages, assistant messages, session notices, and calls that need attention
end the group. Running, failed, and approval-blocked calls stay visible.

Changed files remain visible beneath the summary. Larger sets share directory
headings; sets of one or two files show their relative paths. Files outside the
project retain their path context. Reads and searches remain in Activity.

Each file appears once per group. Repeated edits show an edit count, not a sum of
patch additions and deletions. Zero and unknown line counts stay blank. A
multi-file patch never assigns its aggregate line counts to one file.

Click a file to inspect the operations that touched it during that group. Shared
operations appear once in the detail area, in transcript order. Open Activity to
see the surrounding calls and thinking. Open current file is a separate action;
the current project file may differ from the recorded edit.

Details use bounded scroll areas and readable input/output. Commands remain
inside details. The transcript context menu can copy exact structured tool input,
output, and retained backend metadata. Activity, file, and detail controls support
keyboard focus and expose their expanded state without repeated chevrons.
