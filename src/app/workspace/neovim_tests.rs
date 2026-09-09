use super::*;

struct Server(std::process::Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "requires a Neovim executable; runs a real headless server"]
fn code_capture_preserves_normal_visual_and_unsaved_buffer_state() -> Result<(), String> {
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let socket = directory.path().join("nvim.sock");
    let executable = nvim_executable();
    let _server = Server(
        Command::new(&executable)
            .current_dir(directory.path())
            .args(["--clean", "--headless", "-i", "NONE", "--listen"])
            .arg(&socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?,
    );
    let lua = |body: &str| {
        run_remote(
            &executable,
            directory.path(),
            &socket,
            &format!(
                "luaeval({})",
                vim_string(&format!("(function() {body}; return 0 end)()"))
            ),
        )
    };
    lua(
        "vim.api.nvim_buf_set_name(0, 'capture.rs'); vim.api.nvim_buf_set_lines(0, 0, -1, false, {'alpha beta', 'αβγ world', 'third line'}); vim.api.nvim_win_set_cursor(0, {1, 2})",
    )?;
    let capture = || capture_code(&executable, directory.path(), &socket);
    let normal = capture()?;
    assert_eq!(normal.text, "alpha beta");
    assert_eq!((normal.cursor_line, normal.cursor_column), (1, 3));
    assert!(normal.modified);
    lua(
        "assert(vim.fn.mode() == 'n'); assert(vim.api.nvim_win_get_cursor(0)[2] == 2); vim.cmd('normal! vll')",
    )?;
    let visual = capture()?;
    assert_eq!(visual.text, "pha");
    assert_eq!(visual.mode, "v");
    lua(
        "assert(vim.fn.mode() == 'v'); vim.cmd('normal! ' .. vim.api.nvim_replace_termcodes('<Esc>', true, false, true)); vim.api.nvim_win_set_cursor(0, {2, 0}); vim.cmd('normal! Vj')",
    )?;
    assert_eq!(capture()?.text, "αβγ world\nthird line");
    lua(
        "assert(vim.fn.mode() == 'V'); vim.cmd('normal! ' .. vim.api.nvim_replace_termcodes('<Esc>', true, false, true)); vim.api.nvim_win_set_cursor(0, {1, 4}); vim.cmd('normal! vhh')",
    )?;
    assert_eq!(capture()?.text, "pha");
    lua(
        "vim.cmd('normal! ' .. vim.api.nvim_replace_termcodes('<Esc>', true, false, true)); vim.api.nvim_win_set_cursor(0, {2, 0}); vim.cmd('normal! vl')",
    )?;
    assert_eq!(capture()?.text, "αβ");
    lua(
        "vim.cmd('normal! ' .. vim.api.nvim_replace_termcodes('<Esc>', true, false, true)); vim.api.nvim_win_set_cursor(0, {1, 0}); vim.cmd('normal! ' .. vim.api.nvim_replace_termcodes('<C-v>jl', true, false, true))",
    )?;
    assert_eq!(capture()?.text, "al\nαβ");
    lua(
        "vim.cmd('normal! ' .. vim.api.nvim_replace_termcodes('<Esc>', true, false, true)); vim.o.selection = 'exclusive'; vim.api.nvim_win_set_cursor(0, {1, 0}); vim.cmd('normal! vll')",
    )?;
    assert_eq!(capture()?.text, "al");
    lua(
        "assert(vim.fn.mode() == 'v'); assert(vim.bo.modified); assert(vim.api.nvim_buf_get_lines(0, 1, 2, false)[1] == 'αβγ world'); vim.cmd('normal! ' .. vim.api.nvim_replace_termcodes('<Esc>', true, false, true)); vim.api.nvim_buf_set_lines(0, 0, -1, false, {string.rep('x', 131073)})",
    )?;
    assert!(
        capture()
            .expect_err("invalid test input must fail")
            .contains("128 KiB")
    );
    lua(
        "vim.api.nvim_buf_set_lines(0, 0, -1, false, vim.fn['repeat']({'x'}, 2001)); vim.api.nvim_win_set_cursor(0, {1, 0}); vim.cmd('normal! V2000j')",
    )?;
    assert!(
        capture()
            .expect_err("invalid test input must fail")
            .contains("2,000 lines")
    );
    lua(
        "vim.cmd('normal! ' .. vim.api.nvim_replace_termcodes('<Esc>', true, false, true)); vim.bo.buftype = 'nofile'",
    )?;
    assert!(
        capture()
            .expect_err("invalid test input must fail")
            .contains("Open a file buffer")
    );
    assert!(!directory.path().join("capture.rs").exists());
    Ok(())
}

#[test]
#[ignore = "requires a Neovim executable; runs two real headless servers"]
fn session_processes_isolate_buffers_and_preserve_views() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let a = tempfile::tempdir().map_err(|error| error.to_string())?;
    let b = tempfile::tempdir().map_err(|error| error.to_string())?;
    let executable = nvim_executable();
    let start = |state: &Path| {
        Command::new(&executable)
            .current_dir(project.path())
            .args(["--clean", "--headless", "-i"])
            .arg(state.join("shada"))
            .args(["--cmd", &state_setup(state), "--listen"])
            .arg(state.join("nvim.sock"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map(Server)
            .map_err(|error| error.to_string())
    };
    let _a = start(a.path())?;
    let server_b = start(b.path())?;
    let path = project.path().join("it's | shared.rs");
    std::fs::write(&path, "one\ntwo\nthree\nfour\nfive\n").map_err(|error| error.to_string())?;
    let request = |state: &Path, expression: String| {
        run_remote(
            &executable,
            project.path(),
            &state.join("nvim.sock"),
            &expression,
        )
    };
    let lua = |state: &Path, body: &str| {
        request(
            state,
            format!(
                "luaeval({})",
                vim_string(&format!("(function() {body}; return 0 end)()"))
            ),
        )
    };
    request(a.path(), session_expression(11, Some(&path), None))?;
    lua(
        a.path(),
        r#"
            vim.o.hidden = false
            vim.g.session_marker = 'a'
            vim.cmd('vsplit')
            vim.api.nvim_win_set_cursor(0, {4, 1})
            vim.api.nvim_buf_set_lines(0, 0, 1, false, {'unsaved a'})
        "#,
    )?;
    request(b.path(), session_expression(22, Some(&path), Some(1)))?;
    lua(
        b.path(),
        r#"
            assert(vim.api.nvim_get_current_line() == 'one')
            assert(not vim.bo.modified)
            assert(vim.g.session_marker == nil)
            assert(#vim.api.nvim_tabpage_list_wins(0) == 1)
            vim.api.nvim_buf_set_lines(0, 0, 1, false, {'unsaved b'})
        "#,
    )?;
    for state in [a.path(), b.path()] {
        lua(
            state,
            &format!(
                "assert(vim.o.directory == {0}); assert(vim.o.backupdir == {0}); assert(vim.o.undodir == {0})",
                vim_string(&format!("{}//", state.display()))
            ),
        )?;
    }
    request(a.path(), session_expression(11, None, None))?;
    lua(
        a.path(),
        r#"
            assert(vim.api.nvim_buf_get_lines(0, 0, 1, false)[1] == 'unsaved a')
            assert(vim.bo.modified)
            assert(vim.deep_equal(vim.api.nvim_win_get_cursor(0), {4, 1}))
            assert(#vim.api.nvim_tabpage_list_wins(0) == 2)
        "#,
    )?;
    drop(server_b);
    lua(a.path(), "assert(vim.g.session_marker == 'a')")?;
    lua(
        a.path(),
        "vim.g.original_buffer = vim.api.nvim_get_current_buf()",
    )?;
    let scratch = |text: &str| {
        open_target(
            &executable,
            project.path(),
            a.path(),
            11,
            EditorTarget::Transcript(text.to_owned()),
        )
    };
    scratch("# User\n\nIt's `code` | سلام\n")?;
    lua(
        a.path(),
        r#"
        assert(vim.bo.buftype == 'nofile')
        assert(vim.bo.filetype == 'markdown')
        assert(not vim.bo.swapfile and not vim.bo.buflisted)
        assert(not vim.bo.modified)
        assert(vim.api.nvim_buf_get_lines(0, 2, 3, false)[1] == "It's `code` | سلام")
        vim.g.first_scratch = vim.api.nvim_get_current_buf()
        vim.api.nvim_buf_set_lines(0, 0, 1, false, {'scratch edit'})
    "#,
    )?;
    scratch("# Latest snapshot")?;
    lua(
        a.path(),
        r#"
        assert(vim.api.nvim_get_current_buf() ~= vim.g.first_scratch)
        assert(vim.api.nvim_buf_get_lines(vim.g.first_scratch, 0, 1, false)[1] == 'scratch edit')
        assert(vim.api.nvim_get_current_line() == '# Latest snapshot')
        assert(vim.api.nvim_buf_get_lines(vim.g.original_buffer, 0, 1, false)[1] == 'unsaved a')
    "#,
    )?;
    scratch("")?;
    lua(a.path(), "assert(vim.api.nvim_get_current_line() == '')")?;
    assert!(lua(a.path(), "error('expected test error')").is_err());
    assert_eq!(
        std::fs::read_to_string(path).map_err(|error| error.to_string())?,
        "one\ntwo\nthree\nfour\nfive\n"
    );
    Ok(())
}

#[test]
fn session_request_quotes_file_data_separately_from_lua() {
    let expression = session_expression(7, Some(Path::new("/tmp/it's | tricky.rs")), Some(42));
    assert!(expression.ends_with(", [7, '/tmp/it''s | tricky.rs', 42])"));
    assert!(session_expression(8, None, None).ends_with(", [8, v:null, v:null])"));
    assert_eq!(
        shell_quote(Path::new("/tmp/it's nvim")),
        "'/tmp/it'\\''s nvim'"
    );
}
