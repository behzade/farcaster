use super::*;

#[test]
fn custom_commands_preserve_quoted_arguments_without_shell_expansion() -> Result<(), String> {
    let command = EditorCommand::parse(
        r#""/Applications/My Editor/micro" -p 'a "quote"' "it's fine" '' '$HOME; $(id)'"#,
    )?;
    assert_eq!(command.program, Path::new("/Applications/My Editor/micro"));
    assert_eq!(
        command.arguments,
        ["-p", "a \"quote\"", "it's fine", "", "$HOME; $(id)"]
    );
    assert_eq!(command.choice(), EditorChoice::Micro);
    for invalid in [
        "",
        "  ",
        "'' -p",
        "micro 'oops",
        "micro \"oops",
        "mi\0cro",
        "micro a\0b",
    ] {
        assert!(EditorCommand::parse(invalid).is_err(), "{invalid:?}");
    }
    Ok(())
}

#[test]
fn project_open_only_passes_a_directory_to_editors_that_support_it() -> Result<(), String> {
    let project = Path::new("/project with spaces");
    for source in ["micro -p", "nano", "emacs -nw", "hx", "my-editor --flag"] {
        assert!(
            EditorCommand::parse(source)?
                .project_arguments(project)
                .is_empty()
        );
    }
    for source in ["vim", "nvim --clean", "/usr/bin/vi"] {
        assert_eq!(
            EditorCommand::parse(source)?.project_arguments(project),
            [project.as_os_str()]
        );
    }
    Ok(())
}

#[test]
fn file_locations_use_each_editors_syntax_and_keep_custom_options() -> Result<(), String> {
    let path = Path::new("/project/a b.rs");
    for source in ["vim -u NONE", "micro -p", "nano", "emacs -nw"] {
        let command = EditorCommand::parse(source)?;
        assert_eq!(
            command.file_arguments(path, Some(12)),
            ["+12", "/project/a b.rs"]
        );
        assert_eq!(
            command.file_arguments(path, Some(0)),
            ["+1", "/project/a b.rs"]
        );
    }
    assert_eq!(
        EditorCommand::parse("hx")?.file_arguments(path, Some(12)),
        ["/project/a b.rs:12"]
    );
    assert_eq!(
        EditorCommand::parse("my-editor")?.file_arguments(path, Some(12)),
        ["/project/a b.rs"]
    );
    let custom = EditorChoice::Custom.terminal_command("micro -p", Path::new("/project"), None)?;
    assert_eq!(custom.arguments, ["-p"]);
    assert_ne!(
        custom,
        EditorCommand::parse("micro")?,
        "a changed command needs a distinct editor session"
    );
    assert_eq!(
        EditorChoice::Emacs
            .terminal_command("", Path::new("/project"), None)?
            .arguments,
        ["-nw"]
    );
    Ok(())
}

#[test]
fn micro_review_locations_do_not_apply_the_last_line_to_every_file() -> Result<(), String> {
    let command = EditorCommand::parse("micro -p")?;
    assert_eq!(
        command.review_arguments(&[
            ("/project/first file.rs".into(), Some(12)),
            ("/project/second.rs".into(), Some(35)),
            ("/project/third.rs".into(), None),
        ]),
        [
            "-parsecursor",
            "true",
            "/project/first file.rs:12:1",
            "/project/second.rs:35:1",
            "/project/third.rs:1:1"
        ]
    );
    Ok(())
}
