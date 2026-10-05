use super::*;
use gpui::KeyUpEvent;

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
                app.update(cx, |app, cx| {
                    app.voice_right_shift_changed(true, false, window, cx);
                });
                assert!(matches!(
                    app.read(cx).workspace.voice.gesture,
                    Gesture::Pressed { .. }
                ));
            });
            right_shift(cx, app, false);
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
                        app.voice_right_shift_changed(true, false, window, cx);
                        assert!(app.workspace.voice.key_down.is_none());
                        app.voice_right_shift_changed(false, false, window, cx);
                    });
                    window.draw(cx).clear(cx);
                    window.dispatch_keystroke(gpui::Keystroke::parse("ctrl-g").unwrap(), cx);
                    assert!(!matches!(
                        app.read(cx).workspace.voice.gesture,
                        Gesture::Pressed { .. }
                    ));
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

fn prepare_voice_editor(cx: &mut gpui::VisualTestContext, app: &gpui::Entity<FarcasterApp>) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.workspace.surface = AppSurface::Editor;
            app.workspace.voice.available = true;
            app.navigation.chat.focus.focus(window, cx);
            // No editor process, so the timer cannot start a real microphone.
            assert!(app.workspace.editor.view.is_none());
        });
    });
}

fn right_shift(cx: &mut gpui::VisualTestContext, app: &gpui::Entity<FarcasterApp>, down: bool) {
    // Match GPUI's modifier-only keystroke before the native release callback.
    if !down {
        chord(cx, "shift", true);
    }
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.voice_right_shift_changed(down, false, window, cx)
        });
    });
}

fn chord(cx: &mut gpui::VisualTestContext, key: &str, down: bool) {
    if down {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.dispatch_keystroke(gpui::Keystroke::parse(key).unwrap(), cx);
        });
    } else {
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse(key).unwrap(),
        });
    }
}

#[gpui::test]
fn holding_shortcut_shows_confirmed_recording_in_every_navbar(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::holding_shortcut_shows_confirmed_recording_in_every_navbar"
        ),
        cx,
        |cx, app, _, project| {
            prepare_voice_editor(cx, app);
            right_shift(cx, app, true);
            cx.run_until_parked();
            cx.executor()
                .advance_clock(HOLD_DELAY + Duration::from_millis(1));
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    assert!(matches!(app.workspace.voice.gesture, Gesture::Holding));
                    assert!(app.navigation.chat.activation.hint().is_none());
                    app.workspace.voice.input = Some(input(&project, "current"));
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            assert!(cx.debug_bounds("voice-recording").is_none());
            for surface in [AppSurface::Chat, AppSurface::Editor, AppSurface::Terminal] {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.workspace.voice.input.as_mut().unwrap().recording = true;
                        app.workspace.surface = surface;
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                let icon = cx.debug_bounds("voice-recording").expect("recording icon");
                let bar = cx.debug_bounds("workspace-bar").expect("navbar");
                assert!(icon.top() >= bar.top() && icon.bottom() <= bar.bottom());
                assert!((icon.center().x - bar.center().x).abs() <= gpui::px(1.0));
            }
            let (recorder, finished) = hex::Dictation::stub_with_finish();
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    app.workspace.voice.input.as_mut().unwrap().recorder = recorder;
                })
            });
            right_shift(cx, app, false);
            assert!(finished(), "release finishes the recording");
            cx.update(|window, cx| {
                assert!(!app.read(cx).workspace.voice.recording());
                window.draw(cx).clear(cx);
            });
            assert!(cx.debug_bounds("voice-recording").is_none());
        },
    );
}

#[gpui::test]
fn double_tap_locks_until_the_next_shortcut_press(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::double_tap_locks_until_the_next_shortcut_press"
        ),
        cx,
        |cx, app, _, project| {
            prepare_voice_editor(cx, app);
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    app.workspace.voice.gesture =
                        Gesture::Tapped(Instant::now() - DOUBLE_TAP - Duration::from_millis(1));
                })
            });
            right_shift(cx, app, true);
            right_shift(cx, app, false);
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    assert!(matches!(app.workspace.voice.gesture, Gesture::Tapped(_)));
                    app.workspace.voice.gesture = Gesture::Idle;
                })
            });
            for _ in 0..2 {
                right_shift(cx, app, true);
                right_shift(cx, app, false);
            }
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    assert!(matches!(app.workspace.voice.gesture, Gesture::Locked));
                    assert!(app.workspace.voice.key_down.is_none());
                    app.workspace.voice.input = Some(Input {
                        recording: true,
                        ..input(&project, "current")
                    });
                    assert!(app.workspace.voice.recording());
                    assert!(!app.voice_key_down(&gpui::Keystroke::parse("a").unwrap(), window, cx));
                    assert!(app.workspace.voice.recording());
                })
            });
            let (recorder, finished) = hex::Dictation::stub_with_finish();
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    app.workspace.voice.input.as_mut().unwrap().recorder = recorder;
                })
            });
            right_shift(cx, app, false);
            assert!(!finished(), "locked recording continues after release");
            right_shift(cx, app, true);
            assert!(finished(), "the next press finishes locked recording");
            cx.update(|_, cx| {
                assert!(!app.read(cx).workspace.voice.recording());
                assert!(
                    app.read(cx).workspace.voice.input.is_some(),
                    "keep input while Hex transcribes"
                );
            });
            right_shift(cx, app, false);
            cx.run_until_parked();
            cx.executor()
                .advance_clock(HOLD_DELAY + Duration::from_millis(1));
            cx.run_until_parked();
            cx.update(|_, cx| {
                assert!(matches!(
                    app.read(cx).workspace.voice.gesture,
                    Gesture::Idle
                ))
            });
        },
    );
}

#[gpui::test]
fn unrelated_keys_and_escape_cancel_without_consuming_typing(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::unrelated_keys_and_escape_cancel_without_consuming_typing"
        ),
        cx,
        |cx, app, runtime, project| {
            prepare_voice_editor(cx, app);
            right_shift(cx, app, true);
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    assert!(!app.voice_key_down(
                        &gpui::Keystroke::parse("shift-a").unwrap(),
                        window,
                        cx
                    ))
                })
            });
            right_shift(cx, app, false);
            cx.run_until_parked();
            cx.executor()
                .advance_clock(HOLD_DELAY + Duration::from_millis(1));
            cx.run_until_parked();
            cx.update(|_, cx| assert!(app.read(cx).workspace.voice.input.is_none()));
            for key in ["A", "escape"] {
                right_shift(cx, app, true);
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.workspace.voice.gesture = Gesture::Holding;
                        app.workspace.voice.input = Some(Input {
                            recording: true,
                            ..input(&project, "current")
                        });
                        assert_eq!(
                            app.voice_key_down(&gpui::Keystroke::parse(key).unwrap(), window, cx),
                            key == "escape"
                        );
                        assert!(app.workspace.voice.input.is_none());
                        assert!(!app.workspace.voice.recording());
                        assert!(matches!(app.workspace.voice.gesture, Gesture::Used));
                    });
                    window.draw(cx).clear(cx);
                });
                right_shift(cx, app, false);
            }
            assert!(runtime.try_recv_signal().is_none());
            cx.update(|window, cx| {
                window.dispatch_keystroke(gpui::Keystroke::parse("ctrl-g").unwrap(), cx);
                assert!(!matches!(
                    app.read(cx).workspace.voice.gesture,
                    Gesture::Pressed { .. }
                ));
            });
            cx.simulate_event(KeyUpEvent {
                keystroke: gpui::Keystroke::parse("g").unwrap(),
            });
            cx.update(|_, cx| assert!(app.read(cx).navigation.chat.activation.hint().is_some()));
        },
    );
}

#[gpui::test]
fn voice_custom_shortcut_ignores_repeat_and_releases_without_modifiers(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::voice_custom_shortcut_ignores_repeat_and_releases_without_modifiers"
        ),
        cx,
        |cx, app, runtime, _| {
            prepare_voice_editor(cx, app);
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    app.settings.voice_shortcut = Some("ctrl-alt-v".into());
                });
            });
            chord(cx, "ctrl-.", true);
            chord(cx, ".", false);
            right_shift(cx, app, true);
            right_shift(cx, app, false);
            cx.update(|_, cx| {
                assert!(matches!(
                    app.read(cx).workspace.voice.gesture,
                    Gesture::Idle
                ))
            });
            chord(cx, "ctrl-alt-v", true);
            let generation = cx.update(|_, cx| app.read(cx).workspace.voice.press);
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    // A custom chord still holds when one of its modifiers changes.
                    app.voice_right_shift_changed(false, true, window, cx);
                });
            });
            chord(cx, "ctrl-alt-v", true);
            cx.update(|_, cx| {
                assert_eq!(app.read(cx).workspace.voice.press, generation);
                assert!(matches!(
                    app.read(cx).workspace.voice.gesture,
                    Gesture::Pressed { second_tap: false }
                ));
            });
            chord(cx, "v", false);
            cx.update(|_, cx| {
                assert!(matches!(
                    app.read(cx).workspace.voice.gesture,
                    Gesture::Tapped(_)
                ))
            });
            assert!(
                runtime.try_recv_signal().is_none(),
                "old default must not abort"
            );
        },
    );
}

#[gpui::test]
fn voice_right_shift_ignores_left_shift_and_modifier_combinations(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::voice_right_shift_ignores_left_shift_and_modifier_combinations"
        ),
        cx,
        |cx, app, _, _| {
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.workspace.surface = AppSurface::Editor;
                    app.workspace.voice.available = true;
                    for (down, other) in [
                        (false, true),
                        (false, false),
                        (true, true),
                        (true, false),
                        (false, false),
                    ] {
                        app.voice_right_shift_changed(down, other, window, cx);
                        assert!(app.workspace.voice.key_down.is_none());
                    }
                    app.voice_right_shift_changed(true, false, window, cx);
                    assert!(matches!(
                        app.workspace.voice.gesture,
                        Gesture::Pressed { .. }
                    ));
                    app.voice_right_shift_changed(true, true, window, cx);
                    assert!(matches!(app.workspace.voice.gesture, Gesture::Used));
                    app.voice_right_shift_changed(false, false, window, cx);
                    assert!(matches!(app.workspace.voice.gesture, Gesture::Idle));
                });
            });
        },
    );
}

#[gpui::test]
fn voice_shortcut_capture_accepts_right_shift_but_not_shift_used_for_typing(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::voice_shortcut_capture_accepts_right_shift_but_not_shift_used_for_typing"
        ),
        cx,
        |cx, app, _, _| {
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.open_settings(window, cx);
                    app.settings.voice_shortcut = Some("ctrl-alt-v".into());
                    app.begin_voice_shortcut_capture(cx);
                    app.voice_right_shift_changed(true, false, window, cx);
                    assert!(app.voice_key_down(
                        &gpui::Keystroke::parse("shift-a").unwrap(),
                        window,
                        cx
                    ));
                    app.voice_right_shift_changed(false, false, window, cx);
                    assert!(app.workspace.voice.capturing_shortcut);
                    assert_eq!(app.settings.voice_shortcut.as_deref(), Some("ctrl-alt-v"));
                });
            });
            right_shift(cx, app, true);
            right_shift(cx, app, false);
            cx.update(|_, cx| {
                assert!(!app.read(cx).workspace.voice.capturing_shortcut);
                assert!(app.read(cx).workspace.voice.settings_error.is_none());
                assert!(app.read(cx).settings.voice_shortcut.is_none());
                assert_eq!(app.read(cx).voice_shortcut_label(), "Right Shift");
            });
            assert!(
                crate::app::persistence::open()
                    .unwrap()
                    .load_voice_shortcut()
                    .unwrap()
                    .is_none()
            );
        },
    );
}
