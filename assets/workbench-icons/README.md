# Workbench icons

Application and editor icons shown in the workspace bar and the editor picker.
Every icon is a single `currentColor` path so the theme decides its color.

| Icon | Source | License |
| --- | --- | --- |
| `emacs.svg`, `helix.svg`, `vim.svg` | [Simple Icons](https://github.com/simple-icons/simple-icons) | CC0 1.0 (`../../THIRD_PARTY_LICENSES/SIMPLE-ICONS-CC0-1.0.md`) |
| `nano.svg` | [file-icons](https://github.com/file-icons/icons) | ISC (`../../THIRD_PARTY_LICENSES/FILE-ICONS-ISC.txt`) |
| `micro.svg` | [Micro](https://github.com/zyedidia/micro) logo mark | MIT (`../../THIRD_PARTY_LICENSES/MICRO-MIT.txt`) |
| `ghostty.svg`, `neovim.svg` | Drawn or adapted in-tree | GPL-3.0-or-later, like the application |
| `antigravity.svg`, `claude.svg`, `codex.svg`, `cursor.svg` | [Lobe Icons](https://github.com/lobehub/lobe-icons) | MIT (`../../THIRD_PARTY_LICENSES/LOBE-ICONS-MIT.txt`) |
| `opencode.svg`, `pi.svg` | Official brand assets | MIT (`../../THIRD_PARTY_LICENSES/OPENCODE-MIT.txt`, `../../THIRD_PARTY_LICENSES/PI-WEBSITE-MIT.txt`) |

Upstream editor logos are reduced to one path with `fill="currentColor"` and an
unedited path data string. `micro.svg` keeps only the logo mark and crops the
`viewBox` to it, because the upstream file draws the mark on a filled disc.
`nano.svg` is the GNU nano mark; Simple Icons' `nano` is the Nano cryptocurrency
and must not be used here.

`src/app/ui/assets_tests.rs` enforces the `currentColor` rule, so a new icon
must be tinted the same way. Check a new glyph at 16 px before committing it: a
detailed trace can pass the rule and still render as an unreadable mesh.

The catalog these icons map to is in `src/modules/editors/mod.rs`, and the
path mapping is in `src/app/ui/assets.rs`.
