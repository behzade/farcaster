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
    std::fs::write(
        project.path().join("review.lua"),
        format!("return {}", include_str!("neovim_review.lua")),
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
    std::fs::write(project.path().join("test.lua"), script).unwrap();
    let output = Command::new(nvim_executable())
        .current_dir(project.path())
        .args(["--clean", "--headless", "-i", "NONE", "-l", "test.lua"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
