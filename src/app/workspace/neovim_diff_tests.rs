use super::*;

#[test]
#[ignore = "requires a Neovim executable; exercises real diff windows"]
fn diff_windows_preserve_edits_and_plain_open_restores_normal_view() {
    let project = tempfile::tempdir().expect("create project");
    std::fs::write(project.path().join("work.rs"), "working\nsecond\n")
        .expect("test operation should succeed");
    std::fs::write(project.path().join("base"), "original\nsecond\n")
        .expect("test operation should succeed");
    std::fs::write(
        project.path().join("activate.lua"),
        format!("return {}", include_str!("neovim_session.lua")),
    )
    .expect("test operation should succeed");
    let script = r#"
local function activate(path, base)
  _A = {7, path, 2, vim.NIL, base}
  dofile('activate.lua')
end
activate('work.rs')
local work = vim.api.nvim_get_current_buf()
vim.api.nvim_buf_set_lines(work, 0, 1, false, {'unsaved'})
for _ = 1, 2 do
  activate('work.rs', 'base')
  assert(#vim.api.nvim_tabpage_list_wins(0) == 2)
  assert(vim.api.nvim_get_current_buf() == work)
  assert(vim.bo.modified and vim.wo.diff)
  assert(vim.api.nvim_buf_get_lines(work, 0, 1, false)[1] == 'unsaved')
  for _, win in ipairs(vim.api.nvim_tabpage_list_wins(0)) do
    if win ~= vim.api.nvim_get_current_win() then
      local buf = vim.api.nvim_win_get_buf(win)
      assert(vim.bo[buf].readonly and not vim.bo[buf].modifiable)
      assert(vim.api.nvim_buf_get_lines(buf, 0, 1, false)[1] == 'original')
      assert(vim.wo[win].diff)
      assert(vim.api.nvim_win_get_position(win)[2] == 0)
    end
  end
end
activate('work.rs')
assert(#vim.api.nvim_tabpage_list_wins(0) == 1)
assert(not vim.wo.diff and vim.bo.modified)
assert(vim.api.nvim_get_current_buf() == work)
assert(vim.fn.readfile('work.rs')[1] == 'working')
-- An unrelated split must survive opening and leaving the diff.
vim.cmd('vsplit')
local unrelated = vim.api.nvim_get_current_win()
activate('work.rs', 'base')
assert(#vim.api.nvim_tabpage_list_wins(0) == 3)
activate('work.rs')
assert(#vim.api.nvim_tabpage_list_wins(0) == 2)
assert(vim.api.nvim_win_is_valid(unrelated))
-- Closing the working window by hand must not prevent reopening the file.
vim.cmd('close')
activate('work.rs', 'base')
vim.cmd('hide close')
activate('work.rs')
assert(not vim.wo.diff and vim.bo.modified)
assert(vim.api.nvim_get_current_buf() == work)
vim.cmd('qa!')
"#;
    let script_path = project.path().join("test.lua");
    std::fs::write(&script_path, script).expect("test operation should succeed");
    let output = Command::new(nvim_executable())
        .current_dir(project.path())
        .args(["--clean", "--headless", "-i", "NONE", "-l"])
        .arg(script_path)
        .output()
        .expect("test operation should succeed");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
