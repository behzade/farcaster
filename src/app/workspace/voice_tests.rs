use super::*;

fn input(project: &Path, target: &str) -> Input {
    Input {
        id: "recording".into(),
        destination: CodeDestination {
            target: target.into(),
            session: None,
            label: "Current chat".into(),
            harness: None,
        },
        project: project.into(),
        context: None,
        capturing_context: false,
        transcript: None,
        recorder: hex::Dictation::stub(),
        recording: false,
    }
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn voice_availability_checks_hex_capture_support(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::voice_availability_checks_hex_capture_support"
        ),
        cx,
        |cx, _, _, _| {
            use std::io::{BufRead, Read, Write};
            cx.run_until_parked();
            assert!(!hex::available(), "Hex is absent in the isolated test home");
            let directory = PathBuf::from(std::env::var_os("HOME").unwrap())
                .join("Library/Application Support/voice-control");
            std::fs::create_dir_all(&directory).unwrap();
            let discovery = directory.join("local-api.json");
            for capture in [false, true] {
                let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                let port = listener.local_addr().unwrap().port();
                std::fs::write(
                    &discovery,
                    serde_json::json!({"port": port, "token": "test", "apiVersion": "2"})
                        .to_string(),
                )
                .unwrap();
                let server = std::thread::spawn(move || {
                    let (stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut reader = std::io::BufReader::new(stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    assert_eq!(line, "GET /capabilities HTTP/1.1\r\n");
                    let mut length = 0;
                    loop {
                        line.clear();
                        assert!(reader.read_line(&mut line).unwrap() > 0);
                        if line == "\r\n" {
                            break;
                        }
                        if let Some(value) = line.strip_prefix("Content-Length: ") {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                    }
                    reader.read_exact(&mut vec![0; length]).unwrap();
                    let body = serde_json::json!({"serviceCapture": capture}).to_string();
                    write!(
                        reader.get_mut(),
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                });
                assert_eq!(hex::available(), capture);
                server.join().unwrap();
                assert!(
                    !hex::available(),
                    "a stale discovery file is not availability"
                );
            }
        },
    );
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn chat_voice_arms_and_sends_without_editor_context(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::chat_voice_arms_and_sends_without_editor_context"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.workspace.surface = AppSurface::Chat;
                    app.workspace.voice.available = true;
                    app.chat_composer_focus(cx).focus(window, cx);
                    assert!(app.workspace.editor.view.is_none());
                });
                window.draw(cx).clear(cx);
                window.dispatch_keystroke(gpui::Keystroke::parse("ctrl-g").unwrap(), cx);
                assert!(app.read(cx).workspace.voice.held);
            });
            cx.simulate_event(KeyUpEvent {
                keystroke: gpui::Keystroke::parse("g").unwrap(),
            });
            while runtime.try_recv_command().is_some() {}
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let target = app.composer.sessions.current_target().to_owned();
                    app.workspace.voice.input = Some(Input {
                        transcript: Some("  What changed?  ".into()),
                        ..input(&project, &target)
                    });
                    app.submit_voice_if_ready(window, cx);
                    assert!(app.workspace.voice.input.is_none());
                    assert!(app.workspace.code_tasks.notice_message().is_none());
                    let crate::runtime::RuntimeCommand::SendToSession {
                        target: sent_target,
                        message,
                        submission_id,
                        ..
                    } = runtime.try_recv_command().expect("voice submission")
                    else {
                        panic!("expected SendToSession")
                    };
                    assert_eq!(sent_target, target);
                    assert_eq!(message, "What changed?");
                    assert_eq!(
                        app.workspace.voice.pending.get(&submission_id),
                        Some(&target)
                    );
                    let session = project.join("thread");
                    app.workspace.voice.submission_result(
                        Some(&submission_id),
                        true,
                        Some(&session),
                    );
                    app.speak_voice_reply(
                        "A brief answer.",
                        Some(&(session.clone(), project.into())),
                        cx,
                    );
                    assert!(app.workspace.voice.speech.is_none());
                    assert!(!app.workspace.voice.replies.contains(&session));
                })
            });
        },
    );
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn unavailable_or_disabled_voice_preserves_the_navigation_prefix(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::unavailable_or_disabled_voice_preserves_the_navigation_prefix"
        ),
        cx,
        |cx, app, _, _| {
            cx.run_until_parked();
            for (available, enabled) in [(false, true), (true, false)] {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.workspace.voice.available = available;
                        app.settings.voice_enabled = enabled;
                        app.navigation.chat.activation.clear();
                        app.chat_composer_focus(cx).focus(window, cx);
                    });
                    window.draw(cx).clear(cx);
                    window.dispatch_keystroke(gpui::Keystroke::parse("ctrl-g").unwrap(), cx);
                    assert!(!app.read(cx).workspace.voice.held);
                    assert!(app.read(cx).navigation.chat.activation.hint().is_none());
                });
                cx.simulate_event(KeyUpEvent {
                    keystroke: gpui::Keystroke::parse("g").unwrap(),
                });
                cx.update(|_, cx| {
                    assert!(app.read(cx).navigation.chat.activation.hint().is_some());
                });
            }
        },
    );
}

#[gpui::test]
fn editor_voice_waits_for_capture_before_submitting(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::editor_voice_waits_for_capture_before_submitting"
        ),
        cx,
        |cx, app, runtime, project| {
            while runtime.try_recv_command().is_some() {}
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.workspace.voice.input = Some(Input {
                        capturing_context: true,
                        transcript: Some("Explain this".into()),
                        ..input(&project, app.composer.sessions.current_target())
                    });
                    app.submit_voice_if_ready(window, cx);
                    assert!(runtime.try_recv_command().is_none());
                    let input = app.workspace.voice.input.as_mut().unwrap();
                    input.context = Some(CodeContext {
                        path: "/project/main.rs".into(),
                        cursor_line: 7,
                        cursor_column: 2,
                        anchor_line: 7,
                        anchor_column: 2,
                        mode: "n".into(),
                        text: "unsaved()".into(),
                        modified: true,
                    });
                    input.capturing_context = false;
                    app.submit_voice_if_ready(window, cx);
                    let crate::runtime::RuntimeCommand::SendToSession { message, .. } =
                        runtime.try_recv_command().expect("voice submission")
                    else {
                        panic!("expected SendToSession")
                    };
                    assert_eq!(
                        message,
                        "Explain this\n\n/project/main.rs:7:2\n```\nunsaved()\n```"
                    );
                })
            });
        },
    );
}

#[test]
fn only_accepted_voice_submissions_enable_a_spoken_reply() {
    let mut voice = VoiceState::default();
    let path = Path::new("/project/thread");
    voice.pending.insert("voice".into(), "target".into());
    voice.submission_result(Some("typed"), true, Some(path));
    assert!(voice.replies.is_empty());
    assert!(voice.pending.contains_key("voice"));
    voice.submission_result(Some("voice"), true, Some(path));
    assert!(voice.replies.remove(path));
    assert!(!voice.replies.remove(path), "a reply is only spoken once");
}

#[test]
fn failed_or_stopped_submissions_do_not_speak_a_later_reply() {
    let mut voice = VoiceState::default();
    let path = Path::new("/project/thread");
    voice.pending.insert("voice".into(), "target".into());
    voice.submission_result(Some("voice"), false, Some(path));
    assert!(voice.replies.is_empty());
    assert!(voice.pending.is_empty());
    voice.pending.insert("voice".into(), "target".into());
    voice.pending.insert("other".into(), "other-target".into());
    voice.replies.insert(path.into());
    voice.stopped("target", Some(path));
    assert!(voice.replies.is_empty());
    assert_eq!(voice.pending.len(), 1);
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn holding_control_g_hides_hints_and_shows_only_confirmed_recording(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::holding_control_g_hides_hints_and_shows_only_confirmed_recording"
        ),
        cx,
        |cx, app, _, project| {
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.workspace.surface = AppSurface::Editor;
                    app.workspace.voice.available = true;
                    app.navigation.chat.focus.focus(window, cx);
                    // No editor process: exercise the hold timer without starting a real microphone.
                    assert!(app.workspace.editor.view.is_none());
                });
                window.draw(cx).clear(cx);
                window.dispatch_keystroke(gpui::Keystroke::parse("ctrl-g").unwrap(), cx);
                assert!(app.read(cx).navigation.chat.activation.hint().is_none());
            });
            cx.run_until_parked();
            cx.executor()
                .advance_clock(HOLD_DELAY + Duration::from_millis(1));
            cx.run_until_parked();
            cx.update(|window, cx| {
                assert!(app.read(cx).navigation.chat.activation.hint().is_none());
                app.update(cx, |app, cx| {
                    app.workspace.voice.input = Some(input(&project, "current"));
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            assert!(cx.debug_bounds("voice-recording").is_none());
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.workspace.voice.input.as_mut().unwrap().recording = true;
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            let icon = cx.debug_bounds("voice-recording").expect("recording icon");
            assert!(icon.size.width <= gpui::px(40.0) && icon.size.height <= gpui::px(32.0));
            for surface in [AppSurface::Chat, AppSurface::Editor, AppSurface::Terminal] {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.workspace.surface = surface;
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                let icon = cx
                    .debug_bounds("voice-recording")
                    .expect("recording icon on every surface");
                let bar = cx.debug_bounds("workspace-bar").expect("navbar");
                assert!(icon.top() >= bar.top() && icon.bottom() <= bar.bottom());
                assert!((icon.center().x - bar.center().x).abs() <= gpui::px(1.0));
            }
            cx.simulate_event(KeyUpEvent {
                keystroke: gpui::Keystroke::parse("g").unwrap(),
            });
            cx.update(|window, cx| {
                assert!(app.read(cx).navigation.chat.activation.hint().is_none());
                assert!(!app.read(cx).workspace.voice.recording());
                window.draw(cx).clear(cx);
            });
            assert!(cx.debug_bounds("voice-recording").is_none());
        },
    );
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn tapping_control_g_preserves_navigation_and_key_up_disarms_voice(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::tapping_control_g_preserves_navigation_and_key_up_disarms_voice"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.workspace.surface = AppSurface::Editor;
                    app.workspace.voice.available = true;
                    app.navigation.chat.focus.focus(window, cx);
                });
                window.draw(cx).clear(cx);
                window.dispatch_keystroke(gpui::Keystroke::parse("ctrl-g").unwrap(), cx);
                assert!(app.read(cx).workspace.voice.held);
                assert!(app.read(cx).navigation.chat.activation.hint().is_none());
            });
            cx.simulate_event(gpui::KeyUpEvent {
                keystroke: gpui::Keystroke::parse("ctrl-g").unwrap(),
            });
            cx.update(|_, cx| {
                assert!(!app.read(cx).workspace.voice.held);
                assert!(app.read(cx).navigation.chat.activation.hint().is_some());
            });
            cx.run_until_parked();
            cx.executor()
                .advance_clock(HOLD_DELAY + Duration::from_millis(1));
            cx.run_until_parked();
            cx.update(|window, cx| {
                assert!(app.read(cx).workspace.voice.input.is_none());
                window.dispatch_keystroke(gpui::Keystroke::parse("ctrl-g").unwrap(), cx);
                assert_eq!(app.read(cx).workspace.surface, AppSurface::Chat);
            });
        },
    );
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn escape_cancels_while_control_g_is_still_held(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::escape_cancels_while_control_g_is_still_held"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.workspace.surface = AppSurface::Editor;
                    app.navigation.chat.focus.focus(window, cx);
                    app.workspace.voice.held = true;
                    app.workspace.voice.available = true;
                    app.workspace.voice.input = Some(Input {
                        recording: true,
                        ..input(&project, "original")
                    });
                });
                window.draw(cx).clear(cx);
                window.dispatch_keystroke(gpui::Keystroke::parse("ctrl-escape").unwrap(), cx);
                assert!(app.read(cx).workspace.voice.input.is_none());
                assert!(!app.read(cx).workspace.voice.recording());
                assert!(app.read(cx).workspace.voice.held);
                window.dispatch_keystroke(gpui::Keystroke::parse("ctrl-g").unwrap(), cx);
                assert!(app.read(cx).workspace.voice.input.is_none());
                assert!(app.read(cx).navigation.chat.activation.hint().is_none());
            });
            assert!(
                runtime.try_recv_signal().is_none(),
                "cancelling dictation must not steer or abort the agent"
            );
            cx.simulate_event(gpui::KeyUpEvent {
                keystroke: gpui::Keystroke::parse("g").unwrap(),
            });
            cx.update(|_, cx| assert!(!app.read(cx).workspace.voice.held));
        },
    );
}
