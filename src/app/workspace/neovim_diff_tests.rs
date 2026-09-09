use super::*;

struct Project(tempfile::TempDir);

impl Project {
    fn new() -> Self {
        Self(tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap())
    }

    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(self.0.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn head_base_handles_nested_added_deleted_and_unborn_files() {
    let project = Project::new();
    project.git(&["init", "-q"]);
    let path = project.0.path().join("nested/it's | file.rs");
    std::fs::create_dir(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "original\n").unwrap();
    assert_eq!(head_contents(&path).unwrap(), b"");
    project.git(&["add", "."]);
    project.git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "base",
    ]);
    std::fs::write(&path, "changed\n").unwrap();
    assert_eq!(head_contents(&path).unwrap(), b"original\n");
    let added = project.0.path().join("nested/added.rs");
    std::fs::write(&added, "new\n").unwrap();
    assert_eq!(head_contents(&added).unwrap(), b"");
    project.git(&["add", "."]);
    assert_eq!(head_contents(&added).unwrap(), b"");
    std::fs::remove_file(&added).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
    assert_eq!(head_contents(&path).unwrap(), b"original\n");
    let outside = Project::new();
    assert!(head_contents(&outside.0.path().join("file.rs")).is_err());
}

#[test]
#[ignore = "requires a Neovim executable; exercises real diff windows"]
fn diff_windows_preserve_edits_and_plain_open_restores_normal_view() {
    let project = Project::new();
    std::fs::write(project.0.path().join("work.rs"), "working\nsecond\n").unwrap();
    std::fs::write(project.0.path().join("base"), "original\nsecond\n").unwrap();
    std::fs::write(
        project.0.path().join("activate.lua"),
        format!("return {}", include_str!("neovim_session.lua")),
    )
    .unwrap();
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
    let script_path = project.0.path().join("test.lua");
    std::fs::write(&script_path, script).unwrap();
    let executable = std::env::var_os("FARCASTER_NVIM")
        .or_else(|| std::env::var_os("GPUI_NVIM"))
        .unwrap_or_else(|| "nvim".into());
    let output = Command::new(executable)
        .current_dir(project.0.path())
        .args(["--clean", "--headless", "-i", "NONE", "-l"])
        .arg(script_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
