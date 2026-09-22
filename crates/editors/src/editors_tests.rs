use super::*;
use std::os::unix::fs::PermissionsExt as _;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn parse(command: &str) -> EditorCommand {
    EditorCommand::parse(command).expect("command should parse")
}

fn fake_directory_with(programs: &[&str]) -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    for program in programs {
        let path = directory.path().join(program);
        std::fs::write(&path, "#!/bin/sh\n")?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(directory)
}

#[test]
fn parse_splits_program_and_arguments() {
    assert_eq!(
        parse("nvim"),
        EditorCommand {
            program: "nvim".into(),
            arguments: Vec::new(),
        }
    );
    assert_eq!(
        parse("  micro   -p  "),
        EditorCommand {
            program: "micro".into(),
            arguments: vec!["-p".into()],
        }
    );
}

#[test]
fn parse_keeps_quoted_and_escaped_arguments_together() {
    assert_eq!(
        parse("code --user-data-dir '/tmp/a b'"),
        EditorCommand {
            program: "code".into(),
            arguments: vec!["--user-data-dir".into(), "/tmp/a b".into()],
        }
    );
    assert_eq!(
        parse("hx \"--config /tmp/c\""),
        EditorCommand {
            program: "hx".into(),
            arguments: vec!["--config /tmp/c".into()],
        }
    );
    assert_eq!(
        parse("micro /tmp/with\\ space"),
        EditorCommand {
            program: "micro".into(),
            arguments: vec!["/tmp/with space".into()],
        }
    );
    assert_eq!(
        parse("micro ''"),
        EditorCommand {
            program: "micro".into(),
            arguments: vec![String::new()],
        }
    );
}

#[test]
fn parse_rejects_empty_and_unclosed_commands() {
    assert!(EditorCommand::parse("").is_err());
    assert!(EditorCommand::parse("   ").is_err());
    assert!(EditorCommand::parse("nvim '/tmp/a").is_err());
}

#[test]
fn command_reports_the_catalog_name_and_icon_it_matches() {
    let nvim = parse("/opt/homebrew/bin/nvim");
    assert_eq!(nvim.name(), "Neovim");
    assert_eq!(nvim.icon(), EditorIcon::Neovim);
    assert!(nvim.is_neovim());

    let helix = parse("hx");
    assert_eq!(helix.name(), "Helix");
    assert_eq!(helix.icon(), EditorIcon::Helix);
    assert!(!helix.is_neovim());

    let custom = parse("/usr/local/bin/my-editor --wait");
    assert_eq!(custom.name(), "my-editor");
    assert_eq!(custom.icon(), EditorIcon::Generic);
    assert!(!custom.is_neovim());
}

#[test]
fn only_editors_that_accept_a_line_argument_claim_one() {
    for (command, expected) in [
        ("nvim", true),
        ("vim", true),
        ("vi", true),
        ("micro", true),
        ("nano", true),
        ("emacs", true),
        ("hx", false),
        ("kak", false),
        ("my-editor", false),
    ] {
        assert_eq!(
            parse(command).supports_line_argument(),
            expected,
            "{command}"
        );
    }
}

#[test]
fn command_line_round_trips_through_parse() {
    let command = parse("micro -p");
    assert_eq!(command.command_line(), "micro -p");
    assert_eq!(parse(&command.command_line()), command);
}

#[test]
fn availability_uses_the_path_for_bare_programs_and_stat_for_paths() -> TestResult {
    let directory = fake_directory_with(&["micro"])?;
    assert!(!program_available_in(Path::new("micro"), None));
    assert!(program_available_in(
        Path::new("micro"),
        Some(directory.path().as_os_str())
    ));

    let path = directory.path().join("micro");
    assert!(parse(&path.to_string_lossy()).available());
    assert!(!parse("/nope/nope-editor").available());
    Ok(())
}

#[test]
fn detection_follows_the_catalog_order() -> TestResult {
    let only_micro = fake_directory_with(&["micro"])?;
    assert_eq!(
        installed_text_editor_in(Some(only_micro.path().as_os_str())).map(|editor| editor.id),
        Some("micro")
    );

    let vim_and_micro = fake_directory_with(&["micro", "vim"])?;
    assert_eq!(
        installed_text_editor_in(Some(vim_and_micro.path().as_os_str())).map(|editor| editor.id),
        Some("vim")
    );

    let statuses = text_editor_statuses_in(Some(vim_and_micro.path().as_os_str()));
    assert_eq!(statuses.len(), TEXT_EDITORS.len());
    assert!(
        statuses
            .iter()
            .any(|status| status.id == "vim" && status.available && status.icon == EditorIcon::Vim)
    );
    assert!(
        statuses
            .iter()
            .any(|status| status.id == "helix" && !status.available)
    );

    let empty = tempfile::tempdir()?;
    assert!(installed_text_editor_in(Some(empty.path().as_os_str())).is_none());
    assert!(installed_text_editor_in(None).is_none());
    Ok(())
}

#[test]
fn resolve_prefers_the_saved_command_over_detection() {
    let saved = resolve_text_editor(Some("  memacs --no-splash ")).expect("command should resolve");
    assert_eq!(saved.program, "memacs");
    assert_eq!(saved.arguments, vec!["--no-splash".to_owned()]);
    assert!(resolve_text_editor(Some("nvim '/tmp/a")).is_err());
    assert_eq!(
        resolve_text_editor(Some("   ")).is_err(),
        installed_text_editor().is_none()
    );
}

#[test]
fn resolve_accepts_every_catalog_program() {
    for editor in TEXT_EDITORS {
        let command = EditorCommand::of(&editor);
        assert_eq!(command.program, editor_program(&editor));
        assert!(command.arguments.is_empty());
        assert_eq!(
            text_editor(editor.id).map(|known| known.name),
            Some(editor.name)
        );
        assert_eq!(
            text_editor(editor.program).map(|known| known.id),
            Some(editor.id)
        );
    }
}
