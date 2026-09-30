use super::*;

#[test]
fn only_pastes_longer_than_1000_characters_become_files() -> Result<(), String> {
    assert!(long_paste(&"é".repeat(1000)).is_none());
    assert!(long_paste(&format!("{}\n", "x".repeat(999))).is_none());

    let (normalized, line_count) = long_paste(&format!("{}\r\nz", "é".repeat(1000)))
        .ok_or_else(|| "expected a long paste".to_owned())?;
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let paste = store_long_paste_in(&normalized, line_count, directory.path())?;
    assert_eq!(paste.line_count, 2);
    assert_eq!(
        std::fs::read_to_string(&paste.path).map_err(|error| error.to_string())?,
        format!("{}\nz", "é".repeat(1000))
    );
    Ok(())
}

#[test]
fn display_links_do_not_copy_pasted_contents() {
    let paste = ComposerPaste {
        path: PathBuf::from("/tmp/pasted.txt"),
        content: "secret".into(),
        line_count: 4,
    };

    let display = append_pasted_file_links("$commit", &[paste]);

    assert_eq!(
        display,
        "$commit\n\nPasted text files:\n- [pasted.txt](</tmp/pasted.txt>)"
    );
    assert!(!display.contains("secret"));
}

#[test]
fn pasted_payload_keeps_file_order_delimiters_and_empty_prompt_behavior() {
    let pastes = [
        ComposerPaste {
            path: "/tmp/界.txt".into(),
            content: "first\n".into(),
            line_count: 1,
        },
        ComposerPaste {
            path: "/tmp/second.txt".into(),
            content: "".into(),
            line_count: 1,
        },
    ];
    let attachments = concat!(
        "Pasted text files:\n- [界.txt](</tmp/界.txt>)\n- [second.txt](</tmp/second.txt>)",
        "\n\n--- BEGIN PASTED FILE 界.txt ---\nfirst\n\n--- END PASTED FILE 界.txt ---",
        "\n\n--- BEGIN PASTED FILE second.txt ---\n\n--- END PASTED FILE second.txt ---",
    );
    assert_eq!(append_pasted_files("", &pastes), attachments);
    assert_eq!(
        append_pasted_files("check", &pastes),
        format!("check\n\n{attachments}")
    );
    assert_eq!(append_pasted_files("check", &[]), "check");
}
