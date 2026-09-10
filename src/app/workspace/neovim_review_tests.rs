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
dofile('review.lua')
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
assert(vim.bo.buftype == 'quickfix')
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
