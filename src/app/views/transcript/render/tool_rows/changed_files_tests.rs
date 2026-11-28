use super::*;

#[gpui::test]
fn collapsing_transcript_folders_keeps_totals_on_the_file_columns(cx: &mut gpui::TestAppContext) {
    struct Tree {
        app: WeakEntity<FarcasterApp>,
        items: PersistentVec<Arc<TranscriptItem>>,
        state: ChangeTreeState,
        width: f32,
    }
    impl gpui::Render for Tree {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            div()
                .w(gpui::px(self.width))
                .font_family(crate::app::ui::theme::UI_FONT_FAMILY)
                .text_size(theme().type_scale.body_small)
                .child(render(
                    0,
                    &self.items,
                    0,
                    self.items.len(),
                    Some(&self.state),
                    self.app.clone(),
                    cx,
                ))
        }
    }
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::collapsing_transcript_folders_keeps_totals_on_the_file_columns"
        ),
        cx,
        |cx, app, _, project| {
            let mut item = super::super::tests::write_item();
            let details = Arc::make_mut(item.tool_details.as_mut().unwrap());
            details.metadata.targets = vec!["src/one/main.rs".into(), "src/two/mod.rs".into()];
            details.arguments = serde_json::json!({"changes":[
                {"path":"src/one/main.rs", "diff":"@@ -1 +1 @@\n-old\n+new"},
                {"path":"src/two/mod.rs", "diff":"@@ -0,0 +1,2 @@\n+one\n+two"}
            ]});
            for width in [220.0, 332.0, 640.0] {
                let mut expected = None;
                for expanded in [false, true] {
                    cx.update(|window, cx| {
                        window.replace_root(cx, |_, _| Tree {
                            app: app.downgrade(),
                            items: std::iter::once(Arc::new(item.clone())).collect(),
                            state: ChangeTreeState::with_default(project, expanded),
                            width,
                        });
                        window.draw(cx).clear(cx);
                    });
                    let edges = ["diff-added-column", "diff-removed-column"]
                        .map(|s| cx.debug_bounds(s).unwrap().right());
                    assert_eq!(*expected.get_or_insert(edges), edges);
                    assert_eq!(edges[1], gpui::px(width));
                }
            }
        },
    );
}

#[test]
fn multi_file_edits_keep_independent_net_counts() {
    let mut first = super::super::tests::write_item();
    let details = Arc::make_mut(
        first
            .tool_details
            .as_mut()
            .expect("test operation should succeed"),
    );
    details.metadata.targets = vec!["src/main.rs".into(), "src/other.rs".into()];
    details.arguments = serde_json::json!({"changes":[
        {"path":"src/main.rs", "diff":"@@ -1 +1 @@\n-old\n+middle"},
        {"path":"src/other.rs", "diff":"@@ -0,0 +1,2 @@\n+one\n+two"}
    ]});
    let mut second = first.clone();
    let details = Arc::make_mut(
        second
            .tool_details
            .as_mut()
            .expect("test operation should succeed"),
    );
    details.metadata.targets = vec!["src/main.rs".into()];
    details.arguments = serde_json::json!({"changes":[
        {"path":"/repo/src/main.rs", "diff":"@@ -1 +1 @@\n-middle\n+final"}
    ]});
    let files = collect(
        [(0, &first), (1, &second)].into_iter(),
        Some(Path::new("/repo")),
        None,
    );
    assert_eq!(files[0].counts, Some((1, 1)));
    assert_eq!(files[1].counts, Some((2, 0)));
    assert_eq!(files[0].line, Some(1));
    assert_eq!(files[1].line, Some(1));
}

#[test]
fn repeated_files_keep_history_without_summing_patch_counts() {
    let first = super::super::tests::write_item();
    let mut second = first.clone();
    let details = Arc::make_mut(
        second
            .tool_details
            .as_mut()
            .expect("test operation should succeed"),
    );
    details.metadata.targets = vec!["/repo/src/main.rs".into(), "src/other.rs".into()];
    let files = collect(
        [(7, &first), (9, &second)].into_iter(),
        Some(Path::new("/repo")),
        None,
    );
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].label, "src/main.rs");
    assert_eq!(files[0].last_operation, Some(9));
    assert_eq!(files[0].counts, None);
    assert_eq!(files[1].label, "src/other.rs");
    assert_eq!(files[1].counts, None);
}

#[test]
fn native_multi_file_patch_shows_every_target_without_aggregate_counts() {
    use crate::conversation::ConversationState;
    use serde_json::json;
    let mut state = ConversationState::default();
    state.reduce(&json!({
            "type":"tool_execution_start", "toolCallId":"patch", "toolName":"edit",
            "args":{"path":"src/one/mod.rs", "changes":[
                {"path":"src/one/mod.rs","diff":"+one\n-two"},
                {"path":"src/two/mod.rs","diff":"+three"}
            ]},
            "toolMetadata":{"category":"change", "targets":["src/one/mod.rs", "src/two/mod.rs", "/outside/config"]}
        }));
    state.reduce(&json!({"type":"tool_execution_end", "toolCallId":"patch", "isError":false, "result":{"content":[]}}));
    let files = collect(
        [(3, state.items[0].as_ref())].into_iter(),
        Some(Path::new("/repo")),
        None,
    );
    assert_eq!(
        files
            .iter()
            .map(|file| file.label.as_str())
            .collect::<Vec<_>>(),
        ["/outside/config", "src/one/mod.rs", "src/two/mod.rs"]
    );
    assert!(files.iter().all(|file| file.counts.is_none()));
}
