use super::destinations::{choices, cycle_destination};
use super::*;
use crate::sessions::SessionSummary;
use std::path::Path;

#[cfg(target_os = "macos")]
#[gpui::test]
fn native_send_to_chat_keeps_text_focus_and_closes_in_owner_window(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::native_send_to_chat_keeps_text_focus_and_closes_in_owner_window"
        ),
        cx,
        |cx, app, _, project| {
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            let input = cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let (input, subscription) = create_submit_textarea(
                        window,
                        cx,
                        |input| input.auto_grow(1, 8),
                        FarcasterApp::confirm_send_to_chat,
                    );
                    app.workspace.surface = AppSurface::Editor;
                    app.workspace.send_to_chat = Some(SendToChat {
                        focus: cx.focus_handle(),
                        input: input.clone(),
                        context: CodeContext {
                            path: "file.rs".into(),
                            cursor_line: 1,
                            cursor_column: 1,
                            anchor_line: 1,
                            anchor_column: 1,
                            mode: "n".into(),
                            text: "code".into(),
                            modified: true,
                        },
                        settings: TaskSettings {
                            project: project.into(),
                            harness: app.active_harness(),
                            model: None,
                            effort: None,
                            access_mode: app.snapshot.access_mode_for_new_session(),
                        },
                        destination: None,
                        destinations: Vec::new(),
                        picker: None,
                        error: None,
                        buffer: app.composer.sessions.current_buffer().into(),
                        return_focus: window.focused(cx),
                        _subscription: subscription,
                    });
                    app.cover_native_workspace_surface(cx);
                    cx.notify();
                    input
                })
            });
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let popup = cx.update(|_, cx| {
                app.read(cx)
                    .overlays
                    .modal
                    .window
                    .expect("Send to chat popup")
            });
            let popup_cx = gpui::VisualTestContext::from_window(popup.into(), cx).into_mut();
            popup_cx.update(|window, cx| {
                window.activate_window();
                window.draw(cx).clear(cx);
                assert!(input.read(cx).focus_handle(cx).is_focused(window));
            });
            popup_cx.simulate_input("Explain this");
            popup_cx.simulate_keystrokes("ctrl-n ctrl-p");
            popup_cx.update(|window, cx| {
                assert_eq!(input.read(cx).value().as_ref(), "Explain this");
                assert!(input.read(cx).focus_handle(cx).is_focused(window));
            });
            popup_cx.simulate_keystrokes("escape");
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(app.read(cx).workspace.send_to_chat.is_none());
                assert!(app.read(cx).overlays.modal.window.is_none());
                assert!(!app.read(cx).workspace.native_surface_covered);
                assert!(app.read(cx).workspace.native_surface_snapshot.is_none());
                assert_eq!(app.read(cx).workspace.surface, AppSurface::Editor);
            });
        },
    );
}

#[gpui::test]
fn sending_code_to_archived_history_restores_the_selected_record(cx: &mut gpui::TestAppContext) {
    use crate::{
        agents::Backend,
        runtime::{ArchiveTarget, RuntimeCommand},
        sessions::{AppSessionId, session_buffer},
    };

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::sending_code_to_archived_history_restores_the_selected_record"
        ),
        cx,
        |cx, app, runtime, project| {
            let path = project.join("shared.jsonl");
            let mut selected = SessionSummary::from_cached(
                "selected".into(),
                path.clone(),
                project.into(),
                "Selected".into(),
                String::new(),
                String::new(),
                None,
                std::time::UNIX_EPOCH,
                0,
                Default::default(),
                true,
                false,
                String::new(),
            )
            .with_app_session_id(31);
            selected.harness = Backend::Codex;
            let mut sibling = selected.clone().with_app_session_id(32);
            sibling.harness = Backend::Cursor;
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.selected_draft = None;
                    app.sessions.all = vec![selected.clone(), sibling].into();
                    app.switch_composer_target(session_buffer(selected.harness, &path), window, cx);
                    let snapshot = std::sync::Arc::make_mut(&mut app.snapshot);
                    snapshot.project = project.into();
                    snapshot.harness = Some(selected.harness);
                    snapshot.selected_session = Some(path);
                    snapshot.document_ready = true;
                    snapshot.session = None;

                    let current = app.current_code_destination().expect("selected history");
                    assert_eq!(current.harness, Some(selected.harness));
                    assert_eq!(current.session, Some(selected.target()));
                    let snapshot = std::sync::Arc::make_mut(&mut app.snapshot);
                    snapshot.selected_session = None;
                    snapshot.harness = Some(Backend::Cursor);
                    let current = app.current_code_destination().expect("promoted composer");
                    app.submit_to_chat(current, project.into(), "Explain".into(), window, cx)
                        .expect("code submission");
                    let mut commands = std::iter::from_fn(|| runtime.try_recv_command());
                    let sent = commands.find_map(|command| match command {
                        RuntimeCommand::SendToSession { session, .. } => session,
                        _ => None,
                    });
                    assert_eq!(sent, Some(selected.target()));
                    let restores: Vec<_> = commands
                        .filter_map(|command| match command {
                            RuntimeCommand::SetSessionArchived { request, .. } => {
                                Some((request.target, request.archived))
                            }
                            _ => None,
                        })
                        .collect();
                    assert_eq!(
                        restores,
                        [(ArchiveTarget::App(AppSessionId::new(31).unwrap()), false)]
                    );
                    assert!(!app.sessions.all.by_app_id(31).unwrap().archived);
                    assert!(app.sessions.all.by_app_id(32).unwrap().archived);
                });
            });
        },
    );
}

#[gpui::test]
fn current_code_destination_waits_for_the_selected_draft(cx: &mut gpui::TestAppContext) {
    use crate::{
        agents::Backend,
        runtime::{RuntimeEvent, RuntimeSnapshot},
    };
    use std::sync::Arc;

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::current_code_destination_waits_for_the_selected_draft"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    let draft = app.sessions.selected_draft.clone().expect("initial draft");
                    app.lifecycle.awaiting_draft_snapshot = Some(draft.clone());
                    let previous = Arc::make_mut(&mut app.snapshot);
                    previous.harness = Some(Backend::Codex);
                    previous.selected_session = Some(project.join("previous.jsonl"));
                    previous.session = Some(serde_json::from_value(serde_json::json!({
                        "sessionId": "previous", "isStreaming": false, "isCompacting": false,
                        "autoCompactionEnabled": false, "messageCount": 0, "pendingMessageCount": 0
                    })).unwrap());
                    assert!(previous.session_target().is_some());
                    assert!(app.current_code_destination().is_none());

                    runtime.send_event(RuntimeEvent::Snapshot {
                        generation: app.runtime_generation + 1,
                        snapshot: Arc::new(RuntimeSnapshot {
                            project: project.into(),
                            harness: Some(Backend::Pi),
                            ..Default::default()
                        }),
                    });
                    app.drain_runtime(cx);
                    let current = app.current_code_destination().expect("routed draft");
                    assert_eq!(current.target, crate::sessions::draft_target(&draft));
                    assert_eq!(current.harness, Some(Backend::Pi));
                    assert!(current.session.is_none());
                });
            });
        },
    );
}

#[test]
fn message_preserves_unsaved_code_and_nested_fences() {
    let context = CodeContext {
        path: "/project/it's code.md".into(),
        cursor_line: 2,
        cursor_column: 1,
        anchor_line: 4,
        anchor_column: 3,
        mode: "v".into(),
        text: "```rust\nسلام\n```".into(),
        modified: true,
    };
    let prompt = context.prompt("  Explain this  ");
    assert_eq!(
        prompt,
        "Explain this\n\n/project/it's code.md:2:1–4:3\n````\n```rust\nسلام\n```\n````"
    );
}

#[test]
fn destination_cycling_wraps_through_new_task_and_chats_in_both_directions() {
    let mut selected = None;
    for expected in [Some(0), Some(1), Some(2), None] {
        selected = cycle_destination(selected, 3, true);
        assert_eq!(selected, expected);
    }
    for expected in [Some(2), Some(1), Some(0), None] {
        selected = cycle_destination(selected, 3, false);
        assert_eq!(selected, expected);
    }
}

#[test]
fn destinations_keep_current_then_recent_chats_in_this_project() {
    let project = Path::new("/project");
    let session = |name: &str, project: &Path, archived, age| {
        SessionSummary::from_cached(
            name.into(),
            Path::new("/sessions").join(name),
            project.into(),
            name.into(),
            String::new(),
            String::new(),
            None,
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(age),
            0,
            Default::default(),
            archived,
            false,
            name.into(),
        )
    };
    let current = session("current", project, false, 1);
    let destination = CodeDestination {
        target: "draft:current".into(),
        session: Some(current.target()),
        label: "Current chat".into(),
        harness: Some(current.harness),
    };
    let recent = session("recent", project, false, 5);
    let sessions = vec![
        current,
        session("old", project, false, 2),
        recent.clone(),
        recent,
        session("archived", project, true, 8),
        session("other", Path::new("/elsewhere"), false, 9),
    ];
    let choices = choices(project, destination, &sessions);
    assert_eq!(
        choices
            .iter()
            .map(|choice| choice.label.as_str())
            .collect::<Vec<_>>(),
        ["Current chat", "recent", "old"]
    );
}
