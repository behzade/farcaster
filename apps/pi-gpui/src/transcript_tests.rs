use super::*;

fn item(kind: TranscriptKind, label: &str, text: &str) -> Arc<TranscriptItem> {
    Arc::new(TranscriptItem {
        kind,
        label: label.into(),
        text: text.into(),
        streaming: false,
        is_error: false,
        tool_call_id: None,
        tool_output: String::new(),
        tool_presentation: None,
    })
}

#[test]
fn tail_reserve_is_responsive_but_bounded() {
    assert_eq!(tail_reserve(px(100.0)), px(72.0));
    assert_eq!(tail_reserve(px(500.0)), px(160.0));
    assert_eq!(tail_reserve(px(2_000.0)), px(280.0));
}

#[test]
fn markdown_inline_code_uses_the_reading_palette() {
    let style = transcript_markdown_style();

    assert_eq!(style.inline_code.color, Some(THEME.colors.code.into()));
    assert_eq!(
        style.inline_code.background_color,
        Some(THEME.colors.panel.into())
    );
}

#[test]
fn consecutive_reads_collapse_into_one_row() {
    let rows = project_rows(&[
        item(TranscriptKind::User, "", "question"),
        item(TranscriptKind::Tool, "Read", "Path: a"),
        item(TranscriptKind::Tool, "Read", "Path: b"),
        item(TranscriptKind::Tool, "Bash", "Command: true"),
    ]);
    assert_eq!(rows.len(), 3);
    assert!(matches!(&rows[1], TranscriptRow::ReadGroup { len: 2, .. }));
}

#[test]
fn long_assistant_messages_become_independently_virtualized_rows() {
    let text = format!(
        "{}\n\n{}\n\n{}",
        "first ".repeat(600),
        "second ".repeat(600),
        "third ".repeat(600)
    );
    let assistant = item(TranscriptKind::Assistant, "", &text);
    let rows = project_rows(std::slice::from_ref(&assistant));

    assert!(rows.len() >= 3);
    let reconstructed = rows
        .iter()
        .map(|row| match row {
            TranscriptRow::MessageChunk { start, end, .. } => &text[*start..*end],
            _ => panic!("expected only message chunks"),
        })
        .collect::<String>();
    assert_eq!(reconstructed, text);
}

#[test]
fn a_giant_plain_paragraph_is_split_at_word_boundaries() {
    let text = "word ".repeat(5_000);
    let chunks = markdown_chunk_ranges(&text);

    assert!(chunks.len() > 1);
    assert_eq!(
        chunks
            .iter()
            .map(|(start, end)| &text[*start..*end])
            .collect::<String>(),
        text
    );
}

#[test]
fn fenced_code_is_never_split_inside_the_fence() {
    let code = "let value = 1;\n".repeat(1_000);
    let text = format!("before\n\n```rust\n{code}```\n\nafter");
    let closing_end = text.find("```\n\n").expect("closing fence") + 4;
    let chunks = markdown_chunk_ranges(&text);

    assert!(chunks.iter().any(|(_, end)| *end == closing_end));
    assert!(
        !chunks
            .iter()
            .any(|(_, end)| text[..*end].ends_with("let value = 1;\n"))
    );
}

#[test]
fn row_updates_reproject_only_the_changed_shared_item_suffix() {
    let first = item(TranscriptKind::Assistant, "", "unchanged");
    let second = item(TranscriptKind::Assistant, "", "short");
    let previous_items = vec![first.clone(), second];
    let previous_rows = project_rows(&previous_items);
    let long = item(
        TranscriptKind::Assistant,
        "",
        &format!(
            "section\n\n{}\n\n{}",
            "updated ".repeat(700),
            "tail ".repeat(700)
        ),
    );
    let items = vec![first, long];

    let rows = update_rows(&previous_rows, &previous_items, &items);

    assert_eq!(rows[0], previous_rows[0]);
    assert!(matches!(
        rows[1],
        TranscriptRow::MessageChunk { index: 1, .. }
    ));
}

#[test]
fn changed_item_revision_invalidates_an_equal_length_row() {
    let previous_items = vec![item(TranscriptKind::Assistant, "", "old")];
    let previous_rows = project_rows(&previous_items);
    let items = vec![item(TranscriptKind::Assistant, "", "new")];

    let rows = update_rows(&previous_rows, &previous_items, &items);

    assert_ne!(rows, previous_rows);
}

#[test]
fn appended_reads_merge_with_the_existing_read_group() {
    let previous_items = vec![item(TranscriptKind::Tool, "Read", "Path: one")];
    let previous_rows = project_rows(&previous_items);
    let items = vec![
        previous_items[0].clone(),
        item(TranscriptKind::Tool, "Read", "Path: two"),
    ];

    let rows = update_rows(&previous_rows, &previous_items, &items);

    assert!(matches!(
        rows.as_slice(),
        [TranscriptRow::ReadGroup { len: 2, .. }]
    ));
}

#[test]
fn tool_rows_are_collapsed_by_default_even_while_running_or_failed() {
    let successful_mutation = item(TranscriptKind::Tool, "Edit", "Path: src/main.rs");
    let mut running = item(TranscriptKind::Tool, "Bash", "Command: sleep 1");
    Arc::make_mut(&mut running).streaming = true;
    let mut failed = item(TranscriptKind::Tool, "Write", "Path: denied");
    Arc::make_mut(&mut failed).is_error = true;
    let items = vec![successful_mutation, running, failed];
    let rows = project_rows(&items);

    assert!(!expanded_by_default(rows[0], &items));
    assert!(!expanded_by_default(rows[1], &items));
    assert!(!expanded_by_default(rows[2], &items));
}

#[test]
fn read_groups_stay_collapsed_when_a_call_needs_attention() {
    let successful = item(TranscriptKind::Tool, "Read", "Path: one");
    let mut failed = item(TranscriptKind::Tool, "Read", "Path: two");
    Arc::make_mut(&mut failed).is_error = true;
    let items = vec![successful, failed];
    let rows = project_rows(&items);

    assert!(!expanded_by_default(rows[0], &items));
}

#[test]
fn assistant_turn_after_tool_receives_conclusion_spacing_signal() {
    let items = vec![
        item(TranscriptKind::User, "", "question"),
        item(TranscriptKind::Tool, "Read", "Path: one"),
        item(TranscriptKind::Assistant, "", "answer"),
    ];
    let rows = project_rows(&items);

    assert_eq!(message_role_label(TranscriptKind::User), Some("You"));
    assert_eq!(message_role_label(TranscriptKind::Assistant), Some("Pi"));
    assert!(!message_follows_tool(rows[0], &items));
    assert!(message_follows_tool(rows[2], &items));
}

#[test]
fn explicit_disclosure_state_survives_default_changes() {
    let mut running = item(TranscriptKind::Tool, "Edit", "Path: src/main.rs");
    Arc::make_mut(&mut running).streaming = true;
    let row = project_rows(std::slice::from_ref(&running))[0];
    let states = std::collections::HashMap::from([(row.key(), false)]);
    assert!(!resolved_expanded(
        row,
        std::slice::from_ref(&running),
        &states
    ));

    Arc::make_mut(&mut running).streaming = false;
    assert!(!resolved_expanded(
        row,
        std::slice::from_ref(&running),
        &states
    ));
    assert_eq!(
        tool_state(false, 0, true),
        Some(ToolState {
            glyph: "✓",
            label: "Done"
        })
    );
    assert_eq!(
        tool_state(false, 1, true),
        Some(ToolState {
            glyph: "×",
            label: "Failed"
        })
    );
    assert_eq!(tool_state(false, 0, false), None);
}

#[test]
fn row_position_ignores_content_revision() {
    let original = TranscriptRow::Item {
        index: 3,
        revision: 1,
    };
    let streamed = TranscriptRow::Item {
        index: 3,
        revision: 2,
    };
    let next = TranscriptRow::Item {
        index: 4,
        revision: 2,
    };

    assert!(original.same_position(&streamed));
    assert!(!original.same_position(&next));
    assert_ne!(original, streamed);
}

#[test]
fn targets_use_the_first_readable_argument_value() {
    assert_eq!(tool_target("Path: src/main.rs\nOffset: 2"), "src/main.rs");
    assert_eq!(tool_target(""), "");
}
