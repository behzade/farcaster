use super::*;
use crate::conversation::{ConversationState, ToolReview};
use gpui::px;
use serde_json::json;

#[gpui::test]
fn tool_text_keeps_its_column_across_execution_states(cx: &mut gpui::TestAppContext) {
    struct ToolRow(WeakEntity<FarcasterApp>, TranscriptItem, f32);
    impl gpui::Render for ToolRow {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            div()
                .w(px(self.2))
                .child(render_tool(1.0, 0, &self.1, false, self.0.clone(), cx))
        }
    }
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::tool_text_keeps_its_column_across_execution_states"
        ),
        cx,
        |cx, app, _, _| {
            for width in [220.0, 330.0, 640.0] {
                let mut expected = None;
                for (running, failed) in [(false, false), (false, true), (true, false)] {
                    let mut item = write_item();
                    item.tool_presentation = None;
                    item.streaming = running;
                    item.is_error = failed;
                    cx.update(|window, cx| {
                        window.replace_root(cx, |_, _| ToolRow(app.downgrade(), item, width));
                        window.draw(cx).clear(cx);
                    });
                    let label = cx.debug_bounds("tool-summary-text").expect("tool label");
                    assert_eq!(*expected.get_or_insert(label.left()), label.left());
                    assert!(label.right() <= px(width));
                }
            }
        },
    );
}

#[test]
fn command_labels_keep_arguments_beyond_activity_summary_and_bound_large_scripts() {
    let prefix = "/run/current-system/sw/bin/zsh -lc 'cargo test --locked --offline --quiet ";
    let command = format!("{prefix}--bin farcaster app::views::transcript -- --test-threads=1'");
    assert!(command.len() > 96);
    assert_eq!(command_label(&command), command);
    assert_eq!(
        command_label("\n  cargo test\ngit diff --check "),
        "cargo test\ngit diff --check"
    );
    let script = "متن".repeat(180);
    let preview = command_label(&script);
    assert_eq!(preview.chars().count(), 513);
    assert!(preview.ends_with('…'));
}

#[test]
fn live_activity_keeps_its_row_except_when_a_single_item_becomes_a_group() {
    use super::super::{project_rows, update_rows_from};

    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"message_start","message":{"role":"assistant","content":[]}}));
    state.reduce(&json!({"type":"message_update", "assistantMessageEvent":{"type":"thinking_start", "contentIndex":0}}));
    let mut rows = project_rows(&state.items);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        activity_header(state.items.iter().map(AsRef::as_ref)),
        "Thinking…"
    );

    for (event, header) in [
        (
            json!({"type":"message_update", "assistantMessageEvent":{"type":"thinking_delta", "contentIndex":0, "delta":"Inspect"}}),
            "Thinking…",
        ),
        (
            json!({"type":"message_end", "message":{"role":"assistant", "content":[{"type":"thinking", "thinking":"Inspect"}]}}),
            "Thinking",
        ),
        (
            json!({"type":"tool_execution_start", "toolCallId":"read-1", "toolName":"read", "args":{"path":"src/main.rs"}}),
            "1 read · Reading…",
        ),
        (
            json!({"type":"tool_execution_end", "toolCallId":"read-1", "isError":false, "result":{"content":[]}}),
            "1 read",
        ),
        (
            json!({"type":"message_start","message":{"role":"assistant","content":[]}}),
            "1 read",
        ),
        (
            json!({"type":"message_update", "assistantMessageEvent":{"type":"thinking_start", "contentIndex":0}}),
            "1 read · Thinking…",
        ),
        (
            json!({"type":"message_end", "message":{"role":"assistant", "content":[{"type":"thinking", "thinking":"Reviewed"}]}}),
            "1 read",
        ),
    ] {
        let before = state.items.clone();
        let previous_row = rows[0];
        state.reduce(&event);
        rows = update_rows_from(&rows, &before, &state.items, None);
        assert_eq!(rows, project_rows(&state.items), "{event}");
        assert_eq!(rows.len(), 1, "{event}");
        if before.len() == 1 && state.items.len() > 1 {
            assert!(!rows[0].same_position(&previous_row), "{event}");
            let states = std::collections::HashMap::from([(previous_row.disclosure_key(), true)]);
            assert!(super::super::resolved_expanded(
                rows[0],
                &state.items,
                &states
            ));
        } else {
            assert!(rows[0].same_position(&previous_row), "{event}");
            assert_eq!(rows[0].disclosure_key(), previous_row.disclosure_key());
        }
        assert_eq!(rows[0].item_end(), state.items.len());
        assert_eq!(
            activity_header(state.items.iter().map(AsRef::as_ref)),
            header,
            "{event}"
        );
    }
}

pub(super) fn write_item() -> TranscriptItem {
    let mut state = ConversationState::default();
    state.reduce(&json!({
        "type": "tool_execution_start", "toolCallId": "write-1", "toolName": "write",
        "args": {"path": "src/main.rs", "content": "fn main() {}\n"}
    }));
    state.reduce(&json!({
        "type": "tool_execution_end", "toolCallId": "write-1", "isError": false,
        "result": {"content": [{"type": "text", "text": "Wrote src/main.rs"}]}
    }));
    (*state.items[0]).clone()
}

#[test]
fn file_links_use_each_targets_first_changed_line() {
    let mut item = write_item();
    let details = Arc::make_mut(
        item.tool_details
            .as_mut()
            .expect("test operation should succeed"),
    );
    details.metadata.targets = vec!["src/main.rs".into(), "src/other.rs".into()];
    details.arguments = json!({"changes": [
        {"path": "src/main.rs", "diff": "@@ -37,3 +37,3 @@\n context\n-old\n+new\n tail"},
        {"path": "src/other.rs", "diff": "@@ -80 +80 @@\n-before\n+after"}
    ]});
    assert_eq!(file_target_line(&item, "src/main.rs", None), Some(38));
    assert_eq!(file_target_line(&item, "src/other.rs", None), Some(80));
    assert_eq!(file_target_line(&item, "missing.rs", None), None);
}

#[test]
fn file_links_handle_insertions_deletions_and_missing_positions() {
    for (diff, expected) in [
        ("@@ -0,0 +1,2 @@\n+one\n+two", Some(1)),
        ("@@ -37,2 +36,0 @@\n-one\n-two", Some(37)),
        ("@@ -1 +1 @@\n same\n@@ -9 +9 @@\n-old\n+new", Some(9)),
        ("@@ -1 +1 @@\n same", None),
        ("+new\n-old", None),
    ] {
        let mut item = write_item();
        Arc::make_mut(
            item.tool_details
                .as_mut()
                .expect("test operation should succeed"),
        )
        .result = Some(json!({"details": {"unifiedDiff": diff}}));
        assert_eq!(
            file_target_line(&item, "src/main.rs", None),
            expected,
            "{diff}"
        );
    }
}

#[test]
fn only_successful_file_changes_open_editor() {
    let mut item = write_item();
    assert!(file_targets(&item).next().is_some());
    item.streaming = true;
    assert!(file_targets(&item).next().is_none());
    item.streaming = false;
    item.is_error = true;
    assert!(file_targets(&item).next().is_none());
    item.is_error = false;
    Arc::make_mut(
        item.tool_details
            .as_mut()
            .expect("test operation should succeed"),
    )
    .metadata
    .targets = vec![
        "other.rs".into(),
        String::new(),
        "other.rs".into(),
        "last.rs".into(),
    ];
    assert_eq!(
        file_targets(&item).collect::<Vec<_>>(),
        ["other.rs", "last.rs"]
    );
    Arc::make_mut(
        item.tool_details
            .as_mut()
            .expect("test operation should succeed"),
    )
    .metadata
    .targets
    .clear();
    assert_eq!(file_targets(&item).collect::<Vec<_>>(), ["src/main.rs"]);
    item.tool_presentation = None;
    assert!(file_targets(&item).next().is_none());
}

#[test]
fn approval_replaces_execution_status_instead_of_coexisting() {
    let mut item = write_item();
    item.streaming = true;
    item.tool_review = Some(ToolReview {
        state: ToolReviewState::Reviewing,
        detail: None,
    });
    assert_eq!(item_status(&item), Some(ToolStatus::Reviewing));
    assert!(file_targets(&item).next().is_none());
    item.is_error = true;
    item.tool_review
        .as_mut()
        .expect("test operation should succeed")
        .state = ToolReviewState::Blocked;
    assert_eq!(item_status(&item), Some(ToolStatus::Rejected));
    assert!(file_targets(&item).next().is_none());
    item.tool_review
        .as_mut()
        .expect("test operation should succeed")
        .state = ToolReviewState::Approved;
    assert_eq!(item_status(&item), Some(ToolStatus::Failed));
    item.is_error = false;
    assert_eq!(item_status(&item), Some(ToolStatus::Running));
    item.streaming = false;
    assert_eq!(item_status(&item), Some(ToolStatus::Succeeded));
    assert!(file_targets(&item).next().is_some());
}

#[test]
fn activity_summary_counts_calls_not_events_or_claimed_files() {
    let mut state = ConversationState::default();
    for (id, name, metadata) in [
        (
            "a",
            "read",
            json!({"category":"read", "targets":["same.rs"]}),
        ),
        (
            "b",
            "read",
            json!({"category":"read", "targets":["same.rs"]}),
        ),
        ("c", "bash", json!({"category":"execute"})),
    ] {
        state.reduce(&json!({"type":"tool_execution_start", "toolCallId":id, "toolName":name, "args":{}, "toolMetadata":metadata}));
        state.reduce(&json!({"type":"tool_execution_update", "toolCallId":id, "partialResult":{"content":[]}}));
        state.reduce(&json!({"type":"tool_execution_end", "toolCallId":id, "result":{"content":[]}, "isError":false}));
    }
    assert_eq!(
        activity_summary(state.items.iter().map(AsRef::as_ref)),
        "2 reads · 1 command"
    );
    assert_eq!(item_status(&state.items[2]), Some(ToolStatus::Succeeded));
    state.reduce(&json!({"type":"tool_execution_start", "toolCallId":"custom", "toolName":"mcp_database", "args":{"path":"not-a-file"}}));
    assert_eq!(
        activity_summary(state.items.iter().map(AsRef::as_ref)),
        "2 reads · 1 command · 1 other action"
    );
}

#[test]
fn native_file_targets_are_openable_without_a_diff_preview() {
    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"tool_execution_start", "toolCallId":"acp", "toolName":"Inspect", "args":{}, "toolMetadata":{"category":"read", "targets":["src/main.rs"]}}));
    assert!(file_targets(&state.items[0]).next().is_none());
    state.reduce(&json!({"type":"tool_execution_end", "toolCallId":"acp", "result":{"content":[]}, "isError":false}));
    assert!(state.items[0].tool_presentation.is_none());
    assert!(file_targets(&state.items[0]).next().is_some());
}

#[test]
fn details_preserve_readable_output_and_do_not_repeat_commands() {
    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"tool_execution_start", "toolCallId":"shell", "toolName":"bash", "args":{"command":"cargo check"}}));
    state.reduce(&json!({"type":"tool_execution_end", "toolCallId":"shell", "isError":true, "result":{"content":[{"type":"text","text":"error: missing trait\n  src/main.rs:5"}]}}));
    let detail = tool_body_text(&state.items[0]);
    assert_eq!(detail.matches("cargo check").count(), 1);
    assert!(detail.contains("error: missing trait\n  src/main.rs:5"));
}

#[test]
fn expanded_details_bound_each_source() {
    let huge = "a".repeat(1_500_000);
    for source in ["text", "output", "command", "arguments", "result", "native"] {
        let mut item = write_item();
        item.text.clear();
        item.tool_output.clear();
        let details = Arc::make_mut(
            item.tool_details
                .as_mut()
                .expect("test operation should succeed"),
        );
        details.arguments = json!({});
        details.result = None;
        details.metadata.native = None;
        match source {
            "text" => item.text = huge.clone(),
            "output" => item.tool_output = huge.clone(),
            "command" => details.arguments = json!({"command": huge}),
            "arguments" => details.arguments = json!({"data": huge}),
            "result" => details.result = Some(json!({"data": huge})),
            "native" => details.metadata.native = Some(json!({"data": huge})),
            _ => unreachable!(),
        }
        let preview = tool_body_text(&item);
        assert!(preview.len() < 5 * 1024, "{source}");
        assert!(preview.contains("Preview truncated"), "{source}");
    }
}

#[gpui::test]
fn hovering_short_tool_output_does_not_notify_the_view(cx: &mut gpui::TestAppContext) {
    use std::{cell::Cell, rc::Rc};

    struct ToolOutput(TranscriptItem);
    impl gpui::Render for ToolOutput {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            div()
                .w(px(300.0))
                .child(expanded_tool_body(1.0, "output", &self.0))
        }
    }

    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| ToolOutput(write_item()));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    let notifications = Rc::new(Cell::new(0));
    let observed = notifications.clone();
    let _subscription =
        cx.update(|_, cx| cx.observe(&view, move |_, _| observed.set(observed.get() + 1)));
    for y in (50..130).step_by(10) {
        for x in (20..150).step_by(10) {
            cx.simulate_mouse_move(
                gpui::point(px(x as f32), px(y as f32)),
                None,
                Default::default(),
            );
        }
    }
    assert_eq!(notifications.get(), 0);
}

#[gpui::test]
fn grouped_images_wrap_stay_visible_and_open_in_group_order(cx: &mut gpui::TestAppContext) {
    struct Group {
        app: WeakEntity<FarcasterApp>,
        items: PersistentVec<Arc<TranscriptItem>>,
        expanded: bool,
        width: f32,
    }
    impl gpui::Render for Group {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            div().w(px(self.width)).child(render_activity_group(
                1.0,
                0,
                &self.items,
                0,
                self.items.len(),
                self.expanded,
                &Default::default(),
                None,
                self.app.clone(),
                cx,
            ))
        }
    }
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::grouped_images_wrap_stay_visible_and_open_in_group_order"
        ),
        cx,
        |cx, app, _, _| {
            let mut state = ConversationState::default();
            for id in ["settings", "composer"] {
                state.reduce(&json!({"type":"tool_execution_start", "toolCallId":id, "toolName":"js", "args":{"title":format!("Inspect {id}")}}));
                state.reduce(&json!({"type":"tool_execution_end", "toolCallId":id, "isError":false,"result":{"content":[]}}));
            }
            let mut images = Vec::new();
            for width in [64, 32, 16] {
                let mut bytes = std::io::Cursor::new(Vec::new());
                image::DynamicImage::new_rgb8(width, 16)
                    .write_to(&mut bytes, image::ImageFormat::Png)
                    .unwrap();
                images.push(Arc::new(
                    crate::conversation::EncodedImage::new(bytes.into_inner(), "image/png")
                        .unwrap(),
                ));
            }
            let mut first = state.items[0].as_ref().clone();
            first.images = Arc::new(images[..2].to_vec());
            state.items.set(0, Arc::new(first));
            let mut second = state.items[1].as_ref().clone();
            second.images = Arc::new(images[2..].to_vec());
            state.items.set(1, Arc::new(second));

            for expanded in [false, true] {
                for width in [600., 250., 130.] {
                    cx.update(|window, cx| {
                        window.replace_root(cx, |_, _| Group {
                            app: app.downgrade(),
                            items: state.items.clone(),
                            expanded,
                            width,
                        });
                        window.draw(cx).clear(cx);
                    });
                    let first = cx
                        .debug_bounds("message-image-0-0")
                        .expect("first image visible");
                    let second = cx
                        .debug_bounds("message-image-0-1")
                        .expect("second image visible");
                    let third = cx
                        .debug_bounds("message-image-1-0")
                        .expect("third image visible");
                    for bounds in [first, second, third] {
                        assert!(
                            bounds.right() <= px(width),
                            "thumbnail fits narrow transcript"
                        );
                        assert!(
                            bounds.size.height > theme().size(100.0),
                            "image reserves height before loading"
                        );
                    }
                    if width == 600. {
                        assert_eq!(first.top(), second.top());
                        assert_eq!(second.top(), third.top());
                    } else {
                        assert!(second.top() >= first.bottom());
                        assert!(third.top() >= second.bottom());
                    }
                }
            }
            let third = cx.debug_bounds("message-image-1-0").unwrap();
            cx.simulate_click(third.center(), Default::default());
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let preview = app
                        .overlays
                        .image_preview
                        .as_ref()
                        .expect("image click opens preview");
                    assert_eq!((preview.index, preview.total), (2, 3));
                    assert_eq!(preview.image.bytes(), images[2].bytes());
                    app.close_image_preview(window, cx);
                });
            });
        },
    );
}
