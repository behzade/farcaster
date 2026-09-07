# Transcript Vim archive

Source snapshot from 2026-09-07, taken before removing transcript cursor and
visual modes. Includes uncommitted edits present at the time. These files do
not form a Cargo target and the app does not compile them.

The original paths are kept under `src/`:

- `app/ui/navigation/vim.rs`: key sequences and search input, with tests.
- `app/ui/navigation/shortcuts.rs`: command mapping and shortcut help.
- `app/views/transcript/list/keyboard.rs`: cursor, selection, text geometry,
  clipboard, scrolling, and painting.
- `app/views/transcript/list/keyboard/motions.rs`: text motions and search.
- `app/views/transcript/list/keyboard/tests.rs`: motion and rendering tests.
- The other files show how focus, list layout, and composer hints connected.

For reuse, start with the motions and key parser. The list integration depends
on Farcaster's height index, theme, GPUI text-layout capture, and clipboard APIs;
this is reference source, not a standalone library. See the repository license.

The app keeps mouse text selection and Ctrl+F/B/U/D viewport scrolling.
