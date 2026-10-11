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
                        target: app.composer.sessions.current_target().into(),
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
