use std::{path::Path, sync::Arc};

use gpui::{ListOffset, px, size};

use crate::{
    app::{runtime::TestRuntime, views::transcript::TranscriptRow},
    runtime::{RuntimeEvent, RuntimeSnapshot},
};

fn reading_snapshot(project: &Path, name: &str, messages: usize) -> Arc<RuntimeSnapshot> {
    let mut conversation = crate::conversation::ConversationState::default();
    conversation.replace_history(&(0..messages).map(|index| serde_json::json!({
        "role": if index % 2 == 0 { "user" } else { "assistant" },
        "content": [{"type": "text", "text": format!("{name} message {index}: {}", "reading context ".repeat(30))}]
    })).collect::<Vec<_>>());
    Arc::new(RuntimeSnapshot {
        project: project.into(),
        selected_session: Some(project.join(name)),
        conversation: Arc::new(conversation),
        status: "Ready".into(),
        ..Default::default()
    })
}

fn send_snapshot(runtime: &TestRuntime, generation: u64, snapshot: Arc<RuntimeSnapshot>) {
    runtime.send_event(RuntimeEvent::Snapshot {
        generation,
        snapshot,
    });
}

#[gpui::test]
fn transcript_reading_survives_session_switch_and_loading(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::transcript_reading_survives_session_switch_and_loading"
        ),
        cx,
        |cx, app, runtime, project| {
            let transcript = cx.update(|_, cx| app.read(cx).views.transcript.clone());
            let a = reading_snapshot(project, "a", 60);
            let b = reading_snapshot(project, "b", 60);
            send_snapshot(runtime, 1, a.clone());
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
                transcript.update(cx, |view, _| {
                    view.list.scroll_to(ListOffset {
                        item_ix: 12,
                        offset_in_item: px(9.0),
                    });
                    view.following = false;
                    view.disclosure_states.insert(12, true);
                    let mut tree = crate::app::ui::change_tree::ChangeTreeState::default();
                    tree.toggle(project, Path::new("src"));
                    view.file_trees.insert(12, tree);
                });
            });
            // A reset can precede a loading snapshot, then a second history
            // reset before the destination is ready. Neither may overwrite A.
            for (generation, snapshot) in [(2, b), (3, reading_snapshot(project, "a", 64))] {
                runtime.send_event(RuntimeEvent::SessionReset {
                    generation,
                    preserve_submission: false,
                });
                send_snapshot(
                    runtime,
                    generation,
                    Arc::new(RuntimeSnapshot {
                        conversation: Default::default(),
                        status: "Loading history".into(),
                        ..(*snapshot).clone()
                    }),
                );
                cx.update(|_, cx| app.update(cx, |app, cx| app.drain_runtime(cx)));
                runtime.send_event(RuntimeEvent::HistoryReset { generation });
                send_snapshot(runtime, generation, snapshot);
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                    window.draw(cx).clear(cx);
                    let view = transcript.read(cx);
                    if generation == 2 {
                        assert!(view.list.is_following_tail());
                        assert!(view.disclosure_states.is_empty());
                    } else {
                        let offset = view.list.logical_scroll_top();
                        assert_eq!(offset.item_ix, 12);
                        assert_eq!(offset.offset_in_item, px(9.0));
                        assert!(!view.following);
                        assert_eq!(view.disclosure_states.get(&12), Some(&true));
                        assert!(!view.file_trees[&12].is_open(project, Path::new("src")));
                        assert_eq!(view.unseen, 4);
                    }
                });
            }
        },
    );
}

#[gpui::test]
fn transcript_reading_return_after_resize_syncs_follow_state(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::transcript_reading_return_after_resize_syncs_follow_state"
        ),
        cx,
        |cx, app, runtime, project| {
            let transcript = cx.update(|_, cx| app.read(cx).views.transcript.clone());
            cx.simulate_resize(size(px(1200.0), px(400.0)));
            send_snapshot(runtime, 1, reading_snapshot(project, "a", 60));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
                transcript.update(cx, |view, _| {
                    let bottom = view.list.logical_scroll_top();
                    view.list.scroll_to(ListOffset {
                        item_ix: bottom.item_ix.saturating_sub(1),
                        offset_in_item: px(0.0),
                    });
                    view.following = false;
                    view.unseen = 3;
                });
                window.draw(cx).clear(cx);
                assert!(!transcript.read(cx).list.is_following_tail());
            });
            send_snapshot(runtime, 2, reading_snapshot(project, "b", 60));
            cx.update(|_, cx| app.update(cx, |app, cx| app.drain_runtime(cx)));
            cx.simulate_resize(size(px(1200.0), px(1200.0)));
            send_snapshot(runtime, 3, reading_snapshot(project, "a", 60));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                let view = transcript.read(cx);
                assert!(!view.following);
                assert_eq!(view.unseen, 3);
                window.draw(cx).clear(cx);
            });
            // Let the scroll handler's deferred view update run after paint.
            cx.update(|_, cx| {
                let view = transcript.read(cx);
                assert!(view.list.is_following_tail());
                assert!(
                    view.following,
                    "restoration reached the bottom but left the view paused"
                );
                assert_eq!(view.unseen, 0);
            });
            send_snapshot(runtime, 3, reading_snapshot(project, "a", 62));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
                let view = transcript.read(cx);
                assert!(view.list.is_following_tail());
                assert!(view.following);
                assert_eq!(view.unseen, 0, "followed output must not count as unseen");
            });
        },
    );
}

#[gpui::test]
fn transcript_reading_follows_tail_and_notification_intent(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::transcript_reading_follows_tail_and_notification_intent"
        ),
        cx,
        |cx, app, runtime, project| {
            let transcript = cx.update(|_, cx| app.read(cx).views.transcript.clone());
            for (generation, name) in [(1, "a"), (2, "b"), (3, "a")] {
                send_snapshot(runtime, generation, reading_snapshot(project, name, 60));
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                    window.draw(cx).clear(cx);
                    assert!(transcript.read(cx).list.is_following_tail());
                });
            }
            cx.update(|_, cx| {
                transcript.update(cx, |view, _| {
                    view.list.scroll_to_start();
                    view.following = false;
                });
            });
            send_snapshot(runtime, 4, reading_snapshot(project, "b", 60));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.drain_runtime(cx);
                    app.activity
                        .system_notification_targets
                        .insert("completed-a".into(), (project.join("a"), project.into()));
                    app.activate_system_notification("completed-a", window, cx);
                });
            });
            send_snapshot(runtime, 5, reading_snapshot(project, "a", 64));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
                assert!(transcript.read(cx).list.is_following_tail());
                transcript.update(cx, |view, _| view.list.scroll_to_start());
                app.update(cx, |app, cx| {
                    app.activate_system_notification("completed-a", window, cx)
                });
                assert!(transcript.read(cx).list.is_following_tail());
            });
        },
    );
}

#[gpui::test]
fn transcript_reading_rejects_replaced_messages(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::transcript_reading_rejects_replaced_messages"
        ),
        cx,
        |cx, app, runtime, project| {
            let transcript = cx.update(|_, cx| app.read(cx).views.transcript.clone());
            send_snapshot(runtime, 1, reading_snapshot(project, "a", 60));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
                transcript.update(cx, |view, _| {
                    view.list.scroll_to_start();
                    view.disclosure_states.insert(0, true);
                });
            });
            send_snapshot(runtime, 2, reading_snapshot(project, "b", 60));
            cx.update(|_, cx| app.update(cx, |app, cx| app.drain_runtime(cx)));
            let replaced = Arc::new(RuntimeSnapshot {
                selected_session: Some(project.join("a")),
                ..(*reading_snapshot(project, "rewritten", 60)).clone()
            });
            send_snapshot(runtime, 3, replaced);
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
                let view = transcript.read(cx);
                assert!(view.list.is_following_tail());
                assert!(view.disclosure_states.is_empty());
            });
        },
    );
}

#[gpui::test]
fn transcript_reading_keeps_position_when_visible_stream_finishes(cx: &mut gpui::TestAppContext) {
    check_stream_reading_restore(
        cx,
        concat!(
            module_path!(),
            "::transcript_reading_keeps_position_when_visible_stream_finishes"
        ),
        StreamFinish::WhileReading,
    );
}

#[gpui::test]
fn transcript_reading_stream_completion_keeps_queued_scroll(cx: &mut gpui::TestAppContext) {
    check_stream_reading_restore(
        cx,
        concat!(
            module_path!(),
            "::transcript_reading_stream_completion_keeps_queued_scroll"
        ),
        StreamFinish::WhileScrolling,
    );
}

#[gpui::test]
fn transcript_reading_remaps_stream_finished_before_paint(cx: &mut gpui::TestAppContext) {
    check_stream_reading_restore(
        cx,
        concat!(
            module_path!(),
            "::transcript_reading_remaps_stream_finished_before_paint"
        ),
        StreamFinish::BeforePaint,
    );
}

#[gpui::test]
fn transcript_reading_jump_cancels_pending_restore(cx: &mut gpui::TestAppContext) {
    check_stream_reading_restore(
        cx,
        concat!(
            module_path!(),
            "::transcript_reading_jump_cancels_pending_restore"
        ),
        StreamFinish::AfterJump,
    );
}

#[gpui::test]
fn transcript_reading_does_not_restore_again_after_paint(cx: &mut gpui::TestAppContext) {
    check_stream_reading_restore(
        cx,
        concat!(
            module_path!(),
            "::transcript_reading_does_not_restore_again_after_paint"
        ),
        StreamFinish::AfterPaint,
    );
}

#[derive(Clone, Copy)]
enum StreamFinish {
    WhileReading,
    WhileScrolling,
    BeforePaint,
    AfterJump,
    AfterPaint,
}

fn check_stream_reading_restore(
    cx: &mut gpui::TestAppContext,
    test_name: &str,
    finish: StreamFinish,
) {
    crate::app::test_support::with_offline_app(test_name, cx, |cx, app, runtime, project| {
        let transcript = cx.update(|_, cx| app.read(cx).views.transcript.clone());
        let paragraphs = (0..12)
            .map(|index| format!("Paragraph {index}: {}\n\n", "explanation ".repeat(210)))
            .collect::<Vec<_>>();
        let mut streaming = (*reading_snapshot(project, "a", 60)).clone();
        let conversation = Arc::make_mut(&mut streaming.conversation);
        let mut answer = (*conversation.items[1]).clone();
        answer.streaming = true;
        answer.stream_chunks = Arc::new(
            paragraphs[..7]
                .concat()
                .as_bytes()
                .chunks(2048)
                .map(|bytes| Arc::from(std::str::from_utf8(bytes).unwrap()))
                .collect(),
        );
        answer.text.clear();
        conversation.items.set(1, Arc::new(answer.clone()));
        send_snapshot(runtime, 1, Arc::new(streaming.clone()));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.drain_runtime(cx));
            window.draw(cx).clear(cx);
            transcript.update(cx, |view, _| {
                let index = view
                    .rows
                    .position(|row| {
                        matches!(
                            row,
                            TranscriptRow::StreamChunk {
                                index: 1,
                                chunk: 3,
                                ..
                            }
                        )
                    })
                    .expect("streamed paragraph");
                view.list.scroll_to(ListOffset {
                    item_ix: index,
                    offset_in_item: px(0.0),
                });
                view.following = false;
            });
        });
        let generation = if matches!(
            finish,
            StreamFinish::WhileReading | StreamFinish::WhileScrolling
        ) {
            cx.update(|window, cx| window.draw(cx).clear(cx));
            1
        } else {
            send_snapshot(runtime, 2, reading_snapshot(project, "b", 60));
            cx.update(|_, cx| app.update(cx, |app, cx| app.drain_runtime(cx)));
            send_snapshot(runtime, 3, Arc::new(streaming.clone()));
            3
        };
        answer.streaming = false;
        answer.stream_chunks = Arc::default();
        answer.text = paragraphs.concat();
        Arc::make_mut(&mut streaming.conversation)
            .items
            .set(1, Arc::new(answer));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.drain_runtime(cx));
            let view = transcript.read(cx);
            assert_eq!(view.list.logical_scroll_top().item_ix, 4);
            match finish {
                StreamFinish::WhileScrolling => {
                    view.list
                        .scroll_by(px(1_000_000.0), window, transcript.entity_id());
                }
                StreamFinish::AfterJump => {
                    transcript.update(cx, |view, _| view.jump_to_latest());
                }
                StreamFinish::AfterPaint => {
                    window.draw(cx).clear(cx);
                    let view = transcript.read(cx);
                    view.list.scroll_to(ListOffset {
                        item_ix: 0,
                        offset_in_item: px(0.0),
                    });
                }
                _ => {}
            }
            // Without an intervening paint, finalization must remap the pending anchor.
            send_snapshot(runtime, generation, Arc::new(streaming));
            app.update(cx, |app, cx| app.drain_runtime(cx));

            window.draw(cx).clear(cx);
            let view = transcript.read(cx);
            match finish {
                StreamFinish::WhileScrolling => {
                    assert!(
                        view.list.is_following_tail(),
                        "queued scroll must reach the end"
                    );
                    return;
                }
                StreamFinish::AfterJump => {
                    assert!(view.list.is_following_tail());
                    assert!(view.following);
                    return;
                }
                StreamFinish::AfterPaint => {
                    assert_eq!(view.list.logical_scroll_top().item_ix, 0);
                    assert!(!view.following);
                    return;
                }
                _ => {}
            }
            let row = view.rows[view.list.logical_scroll_top().item_ix];
            let expected = 3 * 2048;
            assert!(
                matches!(row, TranscriptRow::MessageChunk {
                    index: 1, start, end, ..
                } if start <= expected && expected < end),
                "expected byte {expected}, got {row:?}"
            );
            assert!(!view.following);
        });
    });
}
