use super::*;
use crate::app::reviews::{Review, ReviewLocation};

#[test]
#[ignore = "requires a Neovim executable; exercises real quickfix windows"]
fn review_quickfix_preserves_history_buffers_and_advisory_ranges() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("it's code.rs"),
        "first\nsecond\nthird\n",
    )
    .unwrap();
    let review = Review {
        title: "Check `code` | سلام".into(),
        items: vec![
            ReviewLocation {
                path: "it's code.rs".into(),
                start_line: Some(1),
                end_line: Some(2),
                note: "Inspect | vim.cmd('quit')".into(),
            },
            ReviewLocation {
                path: "missing.rs".into(),
                start_line: None,
                end_line: None,
                note: "Deleted file".into(),
            },
            ReviewLocation {
                path: "it's code.rs".into(),
                start_line: Some(10),
                end_line: Some(12),
                note: "Old range".into(),
            },
        ],
    };
    std::fs::write(
        project.path().join("review.json"),
        serde_json::to_vec(&review).unwrap(),
    )
    .unwrap();
    let script = r#"
vim.cmd('edit ' .. vim.fn.fnameescape("it's code.rs"))
local work = vim.api.nvim_get_current_buf()
vim.api.nvim_buf_set_lines(work, 0, 1, false, {'unsaved'})
vim.fn.setqflist({}, ' ', {title = 'user list', items = {{filename = "it's code.rs", lnum = 3, text = 'previous'}}})
local original = vim.fn.getqflist({id = 0}).id
_A = 'review.json'
local navigation = vim.json.decode(dofile('review.lua'))
assert(navigation.selected == 0 and navigation.locations[2].valid == false)
assert(#vim.api.nvim_tabpage_list_wins(0) == 1)
local review = vim.fn.getqflist({items = 0, title = 0, context = 0, id = 0})
assert(review.id ~= original)
assert(review.title == 'Farcaster review: Check `code` | سلام')
assert(review.context.advisory)
assert(#review.items == 3)
assert(review.items[1].lnum == 1 and review.items[1].end_lnum == 2)
assert(review.items[1].text:find('unsaved edits', 1, true))
assert(review.items[1].text:find("vim.cmd('quit')", 1, true))
assert(review.items[2].valid == 0 and review.items[2].text:find('Missing', 1, true))
assert(review.items[3].valid == 0 and review.items[3].text:find('stale', 1, true))
assert(vim.bo.buftype == '')
assert(vim.api.nvim_buf_get_lines(work, 0, 1, false)[1] == 'unsaved')
assert(vim.bo[work].modified)
vim.cmd('colder')
assert(vim.fn.getqflist({id = 0}).id == original)
assert(vim.fn.filereadable('missing.rs') == 0)
vim.cmd('qa!')
"#;
    run_review_script(project.path(), script);
}

#[test]
#[ignore = "requires a Neovim executable; exercises real quickfix and editor windows"]
fn opening_targets_from_review_keeps_quickfix_out_of_the_editing_window() {
    let project = tempfile::tempdir().unwrap();
    for (name, contents) in [
        ("first.rs", "first\n"),
        ("second.rs", "second\nline two\n"),
        ("scratch.md", "# Transcript\n"),
        ("base", "base\n"),
    ] {
        std::fs::write(project.path().join(name), contents).unwrap();
    }
    std::fs::write(
        project.path().join("session.lua"),
        format!("return {}", include_str!("neovim_session.lua")),
    )
    .unwrap();
    let script = r#"
local function activate(path, scratch, base)
  _A = {1, path or vim.NIL, 2, scratch or vim.NIL, base or vim.NIL}
  dofile('session.lua')
end
activate('first.rs')
local main = vim.api.nvim_get_current_win()
local original = vim.api.nvim_get_current_buf()
vim.api.nvim_buf_set_lines(original, 0, 1, false, {'unsaved'})
vim.fn.writefile({vim.json.encode({title = 'Review', items = {{path = 'first.rs', note = 'Inspect'}}})}, 'review.json')
_A = 'review.json'
dofile('review.lua')
-- The user may still open the native list manually.
vim.cmd('copen')
local quickfix = vim.api.nvim_get_current_win()
local qfbuf = vim.api.nvim_get_current_buf()
local list = vim.fn.getqflist({id = 0}).id
assert(vim.bo.buftype == 'quickfix')
activate(nil)
assert(vim.api.nvim_get_current_win() == quickfix, 'resume should preserve quickfix focus')
for _, path in ipairs({'second.rs', 'first.rs'}) do
  vim.api.nvim_set_current_win(quickfix)
  activate(path)
  assert(vim.api.nvim_get_current_win() == main, 'file should open in editing window')
  assert(vim.fn.fnamemodify(vim.api.nvim_buf_get_name(0), ':t') == path)
end
assert(vim.api.nvim_buf_get_lines(original, 0, 1, false)[1] == 'unsaved')
assert(vim.bo[original].modified)
vim.api.nvim_set_current_win(quickfix)
activate(nil, 'scratch.md')
assert(vim.api.nvim_get_current_win() == main)
assert(vim.api.nvim_get_current_line() == '# Transcript')
vim.api.nvim_set_current_win(quickfix)
activate('second.rs', nil, 'base')
assert(vim.api.nvim_get_current_win() == main and vim.wo.diff)
assert(vim.api.nvim_win_get_buf(quickfix) == qfbuf)
assert(vim.bo[qfbuf].buftype == 'quickfix')
assert(vim.fn.getqflist({id = 0}).id == list)
assert(vim.fn.readfile('first.rs')[1] == 'first')
-- If the user closed every editing window, create one rather than reusing
-- the remaining quickfix window.
vim.api.nvim_set_current_win(quickfix)
vim.cmd('only!')
activate('second.rs')
assert(vim.api.nvim_get_current_win() ~= quickfix)
assert(vim.bo.buftype == '')
assert(vim.api.nvim_win_get_buf(quickfix) == qfbuf)
assert(vim.fn.getqflist({id = 0}).id == list)
vim.cmd('qa!')
"#;
    run_review_script(project.path(), script);
}

#[test]
#[ignore = "requires a Neovim executable; exercises hidden quickfix navigation"]
fn hidden_review_navigation_preserves_list_identity_and_revalidates_locations() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("first.rs"), "one\ntwo\n").unwrap();
    std::fs::write(project.path().join("second.rs"), "other\n").unwrap();
    let script = r#"
local function request(args)
  _A = args
  return vim.json.decode(dofile('review.lua'))
end
vim.fn.writefile({vim.json.encode({title = 'Review', items = {
  {path = 'missing.rs', note = 'Deleted'},
  {path = 'first.rs', start_line = 2, end_line = 2, note = 'First'},
  {path = 'second.rs', note = 'Second'},
}})}, 'review.json')
local opened = request('review.json')
assert(opened.selected == 1)
assert(not opened.locations[1].valid and opened.locations[1].warning:find('Missing'))
assert(vim.fn.getqflist({idx = 0}).idx == 2)
assert(vim.api.nvim_get_current_line() == 'two')
assert(#vim.api.nvim_tabpage_list_wins(0) == 1)
vim.cmd('cnext')
assert(vim.api.nvim_get_current_line() == 'other')
vim.cmd('cprevious')
assert(vim.api.nvim_get_current_line() == 'two')
local first = vim.api.nvim_get_current_buf()
-- A sidebar selection restores its own list after native list-history changes.
vim.fn.setqflist({}, ' ', {title = 'Other list', items = {{filename = 'first.rs', lnum = 1}}})
local history = vim.fn.getqflist({nr = '$'}).nr
local selected = request({opened.list_id, 3, vim.fn.fnamemodify('second.rs', ':p')})
assert(selected.list_id == opened.list_id and selected.selected == 2)
assert(vim.fn.getqflist({idx = 0}).idx == 3)
assert(vim.fn.getqflist({nr = '$'}).nr == history)
assert(vim.api.nvim_get_current_line() == 'other')
-- Revalidate stale ranges against unsaved buffers before jumping.
vim.api.nvim_buf_set_lines(first, 0, -1, false, {'unsaved'})
local stale = request({opened.list_id, 2})
assert(stale.selected == vim.NIL and not stale.locations[2].valid)
assert(stale.locations[2].warning:find('stale'))
assert(vim.api.nvim_get_current_line() == 'other')
vim.fn.delete('second.rs')
local deleted = request({opened.list_id, 3})
assert(deleted.selected == vim.NIL and not deleted.locations[3].valid)
assert(vim.fn.filereadable('second.rs') == 0)
-- Even an entirely unavailable review leaves the editor usable, not quickfix.
local unavailable = request('review.json')
assert(unavailable.selected == vim.NIL)
assert(#vim.api.nvim_tabpage_list_wins(0) == 1)
assert(vim.bo.buftype ~= 'quickfix')
vim.fn.setqflist({}, 'f')
assert(not pcall(request, {opened.list_id, 2}))
vim.cmd('qa!')
"#;
    run_review_script(project.path(), script);
}

#[test]
#[ignore = "requires a Neovim executable; exercises repurposed splits and review routing"]
fn review_targets_main_pane_even_when_file_is_already_in_small_split() {
    let project = tempfile::tempdir().unwrap();
    for name in ["original.rs", "first.rs", "second.rs", "base"] {
        std::fs::write(project.path().join(name), "one\ntwo\n").unwrap();
    }
    std::fs::write(
        project.path().join("session.lua"),
        format!("return {}", include_str!("neovim_session.lua")),
    )
    .unwrap();
    run_review_script(
        project.path(),
        r#"
_A = {1, 'original.rs'}
dofile('session.lua')
local main = vim.api.nvim_get_current_win()
local original = vim.api.nvim_get_current_buf()
vim.api.nvim_buf_set_lines(original, 0, 1, false, {'unsaved main'})
-- Reproduce the reported layout: a former quickfix split is now a file window.
vim.fn.setqflist({}, ' ', {items = {{filename = 'first.rs', lnum = 1}}})
vim.cmd('botright copen 3')
local small = vim.api.nvim_get_current_win()
vim.cmd('edit first.rs')
local first = vim.api.nvim_get_current_buf()
vim.api.nvim_buf_set_lines(first, 0, 1, false, {'unsaved small'})
vim.fn.writefile({vim.json.encode({title = 'Review', items = {
  {path = 'first.rs', start_line = 2, note = 'First'},
  {path = 'second.rs', note = 'Second'},
}})}, 'review.json')
local function request(args)
  _A = {1, vim.NIL, vim.NIL, vim.NIL, vim.NIL, true}
  dofile('session.lua')
  _A = args
  return vim.json.decode(dofile('review.lua'))
end
local opened = request('review.json')
assert(vim.api.nvim_get_current_win() == main, 'initial review must use main pane')
assert(vim.api.nvim_get_current_buf() == first and vim.fn.line('.') == 2)
assert(vim.api.nvim_win_get_buf(small) == first, 'leave user split intact')
assert(vim.bo[original].modified and vim.bo[first].modified)
assert(vim.api.nvim_buf_get_lines(original, 0, 1, false)[1] == 'unsaved main')
assert(#vim.api.nvim_tabpage_list_wins(0) == 2)
vim.api.nvim_set_current_win(small)
local selected = request({opened.list_id, 2})
assert(vim.api.nvim_get_current_win() == main, 'sidebar selection must use main pane')
assert(vim.fn.fnamemodify(vim.api.nvim_buf_get_name(0), ':t') == 'second.rs')
assert(selected.list_id == opened.list_id and vim.fn.getqflist({idx = 0}).idx == 2)
vim.cmd('hide cprevious')
assert(vim.api.nvim_get_current_line() == 'two', 'native navigation still works')
-- Opening a review leaves app-owned diff mode, without closing user splits.
vim.api.nvim_set_current_win(main)
_A = {1, 'second.rs', 1, vim.NIL, 'base'}
dofile('session.lua')
assert(vim.wo.diff)
request({opened.list_id, 1})
assert(vim.api.nvim_get_current_win() == main and not vim.wo.diff)
assert(#vim.api.nvim_tabpage_list_wins(0) == 2)
assert(vim.api.nvim_win_get_buf(small) == first)
vim.cmd('qa!')
"#,
    );
}

fn run_review_script(project: &Path, script: &str) {
    std::fs::write(
        project.join("review.lua"),
        format!("return {}", include_str!("neovim_review.lua")),
    )
    .unwrap();
    std::fs::write(project.join("test.lua"), script).unwrap();
    let output = Command::new(nvim_executable())
        .current_dir(project)
        .args(["--clean", "--headless", "-i", "NONE", "-l", "test.lua"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
