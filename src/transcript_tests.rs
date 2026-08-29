use super::*;
use gpui::{Image, ImageFormat};

fn item(kind: TranscriptKind, label: &str, text: &str) -> Arc<TranscriptItem> {
    Arc::new(TranscriptItem {
        kind,
        label: label.into(),
        text: text.into(),
        images: Arc::default(),
        stream_chunks: Arc::default(),
        streaming: false,
        is_error: false,
        tool_call_id: None,
        tool_output: String::new(),
        tool_presentation: None,
        invocation: None,
    })
}

#[test]
fn transcript_copy_keeps_image_attachment_markers() {
    let mut attached = item(TranscriptKind::User, "", "look here");
    Arc::make_mut(&mut attached).images = Arc::new(vec![Arc::new(Image::from_bytes(
        ImageFormat::Png,
        vec![1, 2, 3],
    ))]);
    let mut items = PersistentVec::default();
    items.push(attached);

    assert_eq!(
        copy_transcript_items(&items, 0..=0),
        "look here\n\n[Image attachment]"
    );
}

#[test]
fn tail_reserve_is_responsive_but_bounded() {
    assert_eq!(tail_reserve(px(100.0)), px(72.0));
    assert_eq!(tail_reserve(px(500.0)), px(160.0));
    assert_eq!(tail_reserve(px(2_000.0)), px(280.0));
}

#[test]
fn expanded_latest_tools_keep_space_above_the_composer() {
    let mut items = PersistentVec::default();
    items.push(item(TranscriptKind::Tool, "Read", ""));

    assert!(latest_allows_tail_reserve(
        TranscriptRow::Item {
            index: 0,
            revision: 0,
        },
        &items,
        true,
    ));
    assert!(latest_allows_tail_reserve(
        TranscriptRow::ReadGroup {
            start: 0,
            len: 1,
            revision: 0,
        },
        &items,
        true,
    ));
}

#[test]
fn invocation_badges_distinguish_skills_prompts_and_stacks() {
    assert_eq!(
        invocation_kind("$review", "<skill name=\"review\">body</skill>"),
        "Skill"
    );
    assert_eq!(
        invocation_kind("please $review", "Review this change"),
        "Prompt"
    );
    assert_eq!(invocation_kind("$review $commit", "resolved"), "Stack");
    assert_eq!(
        invocation_kind("cost $100 then $review", "resolved"),
        "Prompt"
    );
    assert_eq!(invocation_kind("$review", ""), "Invocation");
}

#[test]
fn invocations_choose_standalone_or_user_message_treatment() {
    let skill = "<skill name=\"review\">body</skill>";

    assert!(!is_mixed_invocation_message("$review", skill));
    assert!(!is_mixed_invocation_message("$review $commit", skill));
    assert!(is_mixed_invocation_message("please $review this", skill));
    assert!(!is_mixed_invocation_message("$commit", "expanded prompt"));
    assert!(is_mixed_invocation_message(
        "$commit\nwhy did this happen",
        "expanded prompt"
    ));
    assert!(is_mixed_invocation_message(
        "$commit costs $100",
        "expanded prompt"
    ));
}

#[test]
fn mixed_user_messages_highlight_only_recognized_invocation_tokens() {
    let skill = "<skill name=\"review\">body</skill>\nPrompt body";

    assert_eq!(
        highlighted_invocation_markdown("Please $review then $commit", skill),
        "Please `$review` then $commit"
    );
    assert_eq!(
        highlighted_invocation_markdown("Use $skill:review\nnow", skill),
        "Use `$skill:review`\nnow"
    );
    assert_eq!(
        highlighted_invocation_markdown(
            "$commit\n\nwhy did this happen\n\nAttached image",
            "expanded prompt"
        ),
        "`$commit`\n\nwhy did this happen\n\nAttached image"
    );
    assert_eq!(
        highlighted_invocation_markdown("$commit costs $100", "expanded prompt"),
        "`$commit` costs $100"
    );
}

#[test]
fn invocation_treatment_uses_distinct_skill_and_prompt_palettes() {
    assert_ne!(THEME.colors.skill, THEME.colors.accent);
    assert_ne!(THEME.colors.skill, THEME.colors.success);
    let skill = invocation_transcript_markdown_style("<skill name=\"review\">body</skill>");
    assert_eq!(skill.inline_code.color, Some(THEME.colors.skill.into()));
    assert_eq!(skill.inline_code.background_color, None);

    let prompt = invocation_transcript_markdown_style("expanded prompt");
    assert_eq!(prompt.inline_code.color, Some(THEME.colors.accent.into()));
    assert_eq!(
        prompt.inline_code.background_color,
        Some(THEME.colors.panel.into())
    );
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
fn link_heavy_user_messages_become_independently_virtualized_rows() {
    let text = (0..200)
        .map(|line| {
            format!(
                "frame_{line} @ http://localhost:3000/ai/node_modules/.vite/deps/library.js?v=50efe34d:{line}\n"
            )
        })
        .collect::<String>();
    let user = item(TranscriptKind::User, "", &text);
    let rows = project_rows(std::slice::from_ref(&user));

    assert!(rows.len() > 1);
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
    let chunks = markdown_chunks(&text);

    assert!(chunks.len() > 1);
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| &text[chunk.start..chunk.end])
            .collect::<String>(),
        text
    );
}

#[test]
fn oversized_fenced_code_is_split_into_bounded_valid_markdown() {
    let code = "let value = 1;\n".repeat(1_000);
    let text = format!("before\n\n```rust\n{code}```\n\nafter");
    let chunks = markdown_chunks(&text);
    let fenced = chunks
        .iter()
        .copied()
        .filter(|chunk| chunk.fence.is_some())
        .collect::<Vec<_>>();

    assert!(fenced.len() > 1);
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| &text[chunk.start..chunk.end])
            .collect::<String>(),
        text
    );
    for chunk in fenced {
        let rendered = markdown_chunk_text(&text, chunk);
        assert!(rendered.starts_with("```rust\n"));
        assert!(rendered.trim_end().ends_with("```"));
        assert!(rendered.len() <= MARKDOWN_CHUNK_HARD_BYTES + 64);
    }
}

#[test]
fn unclosed_fences_keep_code_context_and_commonmark_boundaries() {
    let text = format!(
        "  ~~~~rust\r\n{}  ~~~~not-a-close\r\n{}",
        "line\r\n".repeat(100),
        "tail\r\n".repeat(100)
    );
    let chunks = markdown_chunks(&text);

    assert!(chunks.len() > 1);
    assert!(chunks.iter().all(|chunk| {
        let rendered = markdown_chunk_text(&text, *chunk);
        rendered.starts_with("  ~~~~rust\r\n") && rendered.trim_end().ends_with("~~~~")
    }));

    let indented = "    ```rust\n";
    assert!(markdown_fence(indented, 0, indented.len()).is_none());
    let opening = "````text\n";
    let fence = markdown_fence(opening, 0, opening.len()).expect("opening fence");
    assert!(!markdown_fence_closes("```\n", fence));
    assert!(!markdown_fence_closes("````not-a-close\n", fence));
    assert!(markdown_fence_closes("  `````\n", fence));
}

#[test]
fn item_prefix_accounting_counts_only_performed_comparisons() {
    let first = item(TranscriptKind::Assistant, "", "first");
    let second = item(TranscriptKind::Assistant, "", "second");

    assert_eq!(
        matching_item_prefix(&[first.clone(), second.clone()], &[first.clone(), second]),
        (2, 2)
    );
    assert_eq!(
        matching_item_prefix(
            &[first.clone(), item(TranscriptKind::Assistant, "", "old")],
            &[first.clone(), item(TranscriptKind::Assistant, "", "new")]
        ),
        (1, 2)
    );
    assert_eq!(
        matching_item_prefix(
            std::slice::from_ref(&first),
            &[first.clone(), item(TranscriptKind::User, "", "appended")]
        ),
        (1, 1)
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
fn reconstructed_equal_history_skips_the_row_update() {
    let previous_items = vec![
        item(TranscriptKind::User, "", "question"),
        item(TranscriptKind::Assistant, "", "answer"),
    ];
    let previous_rows = project_rows(&previous_items);
    let reconstructed = previous_items
        .iter()
        .map(|item| Arc::new(item.as_ref().clone()))
        .collect::<Vec<_>>();

    let update = update_rows_incremental(&previous_rows, &previous_items, &reconstructed, Some(0));

    assert!(update.rows.is_none());
}

#[test]
fn row_update_identifies_the_unchanged_prefix() {
    let previous_items = vec![
        item(TranscriptKind::User, "", "question"),
        item(TranscriptKind::Assistant, "", "first answer"),
        item(TranscriptKind::Assistant, "", "old tail"),
    ];
    let previous_rows = project_rows(&previous_items);
    let changed = vec![
        previous_items[0].clone(),
        previous_items[1].clone(),
        item(TranscriptKind::Assistant, "", "new tail"),
    ];
    let update = update_rows_incremental(&previous_rows, &previous_items, &changed, Some(2));

    assert_eq!(update.unchanged_prefix_rows, 2);
}

#[test]
fn equal_snapshot_recovers_items_missing_from_the_row_cache() {
    let rendered_items = vec![item(TranscriptKind::Assistant, "", "answer")];
    let previous_rows = project_rows(&rendered_items);
    let mut previous_items = rendered_items;
    previous_items.push(item(TranscriptKind::User, "", "follow up"));
    let reconstructed = previous_items
        .iter()
        .map(|item| Arc::new(item.as_ref().clone()))
        .collect::<Vec<_>>();

    let rows = update_rows_from(&previous_rows, &previous_items, &reconstructed, Some(1));

    assert!(matches!(
        rows.iter().copied().collect::<Vec<_>>().as_slice(),
        [
            TranscriptRow::Item { index: 0, .. },
            TranscriptRow::Item { index: 1, .. }
        ]
    ));
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
        rows.iter().copied().collect::<Vec<_>>().as_slice(),
        [TranscriptRow::ReadGroup { len: 2, .. }]
    ));
}

#[test]
fn markdown_row_height_estimates_reflect_wrapping_and_physical_lines() {
    let assistant = item(
        TranscriptKind::Assistant,
        "",
        &format!("```text\n{}```", "line\n".repeat(1_000)),
    );
    let items = vec![assistant];
    let rows = project_rows(&items);

    assert!(rows.len() > 10);
    assert!(
        rows.iter()
            .all(|row| estimated_row_height(*row, &items) > TRANSCRIPT_ROW_HEIGHT_HINT)
    );
}

#[test]
fn thinking_rows_have_details_only_when_body_exceeds_the_title() {
    let single = item(TranscriptKind::Thinking, "", "one line");
    let trailing = item(TranscriptKind::Thinking, "", "one line\n");
    let empty = item(TranscriptKind::Thinking, "", "");
    let multiline = item(TranscriptKind::Thinking, "", "first line\nmore");
    let mut continued = item(TranscriptKind::Thinking, "", " world");
    Arc::make_mut(&mut continued).stream_chunks = Arc::new(vec!["hello".into()]);
    let mut whitespace_tail = item(TranscriptKind::Thinking, "", "   ");
    Arc::make_mut(&mut whitespace_tail).stream_chunks = Arc::new(vec!["hello".into()]);

    assert!(!thinking_has_details(&single));
    assert!(!thinking_has_details(&trailing));
    assert!(!thinking_has_details(&empty));
    assert!(!thinking_has_details(&whitespace_tail));
    assert!(thinking_has_details(&multiline));
    assert!(thinking_has_details(&continued));
    assert_eq!(thinking_preview(&single), "one line");
    assert_eq!(thinking_preview(&empty), "Thinking…");
}

#[test]
fn agent_results_are_collapsed_by_default() {
    let result = item(
        TranscriptKind::AgentResult,
        "Subagent result",
        "Subagent child-1 (idle) returned:\n# Findings\nlong body",
    );
    let items = vec![result];
    let row = project_rows(&items)[0];

    assert!(!expanded_by_default(row, &items));
    assert_eq!(message_role_label(TranscriptKind::AgentResult), None);
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
fn streaming_chunk_growth_keeps_stable_row_position() {
    let original = TranscriptRow::MessageChunk {
        index: 3,
        start: 0,
        end: 100,
        block: 0,
        revision: 1,
        first: true,
        last: true,
        fence: None,
    };
    let streamed = TranscriptRow::MessageChunk {
        index: 3,
        start: 0,
        end: 180,
        block: 0,
        revision: 2,
        first: true,
        last: false,
        fence: None,
    };

    assert!(original.same_position(&streamed));
    assert_ne!(original, streamed);
}

#[test]
fn targets_use_the_first_readable_argument_value() {
    assert_eq!(tool_target("Path: src/main.rs\nOffset: 2"), "src/main.rs");
    assert_eq!(tool_target(""), "");
}

#[test]
fn host_script_targets_prefer_the_command_over_the_reason() {
    assert_eq!(
        tool_target("Need sudo to install docker\n\nCommand:\nsudo apt install docker"),
        "sudo apt install docker"
    );
}
