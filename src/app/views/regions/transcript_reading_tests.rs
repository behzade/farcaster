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

#[test]
fn locally_stored_suffix_retains_its_loading_boundary() {
    use crate::{
        agents::HistoryCoverage,
        storage::{StoredViewport, TranscriptStamp},
    };
    let project = Path::new(".");
    let snapshot = reading_snapshot(project, "stored", 4);
    for (coverage, expected_boundary) in [
        (HistoryCoverage::Suffix, Some(0)),
        (HistoryCoverage::Complete, None),
    ] {
        let page = super::super::PreparedPage::from_store(
            StoredViewport {
                full_document: true,
                coverage,
                content: (*snapshot.conversation).clone(),
                markdown: Default::default(),
                revision: 1,
                stamp: TranscriptStamp {
                    session_id: 1,
                    revision: 1,
                    prompt_event_seq: 0,
                },
            },
            project,
        )
        .unwrap();
        assert_eq!(page.loading_boundary, expected_boundary);
    }
}

#[gpui::test]
fn transcript_reading_paints_mid_turn_reply_while_earlier_messages_load(
    cx: &mut gpui::TestAppContext,
) {
    use crate::{
        agents::{HistoryCoverage, HistoryPage},
        conversation::ConversationState,
    };
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::transcript_reading_paints_mid_turn_reply_while_earlier_messages_load"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.simulate_resize(size(px(1600.), px(1600.)));
            cx.update(|_, cx| app.update(cx, |app, _| app.sessions.selected_draft = None));
            let full = reading_snapshot(project, "mid-turn", 20);
            let messages = &full.conversation.items;
            let mut tail = ConversationState::default();
            tail.items = messages.iter_range(19..20).cloned().collect();
            let before = RuntimeSnapshot {
                status: "Loading history".into(),
                conversation: Default::default(),
                history_tail: Some(Arc::new(HistoryPage {
                    content: tail.clone(),
                    coverage: HistoryCoverage::Suffix,
                })),
                ..(*full).clone()
            };
            send_snapshot(runtime, 1, Arc::new(before));
            cx.update(|_, cx| app.update(cx, |app, cx| app.drain_runtime(cx)));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let reply = cx
                .debug_bounds("transcript-row-0")
                .expect("final reply in a mid-turn suffix");
            assert!(
                cx.debug_bounds("transcript-older-loading")
                    .unwrap()
                    .bottom()
                    <= reply.top()
            );
            send_snapshot(
                runtime,
                1,
                Arc::new(RuntimeSnapshot {
                    conversation: Arc::new(tail),
                    saved_history: Some(crate::runtime::SavedHistory::RefreshingPartial),
                    status: "Partial saved history".into(),
                    document_ready: true,
                    ..(*full).clone()
                }),
            );
            cx.update(|_, cx| app.update(cx, |app, cx| app.drain_runtime(cx)));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert_eq!(cx.debug_bounds("transcript-row-0"), Some(reply));
            send_snapshot(
                runtime,
                1,
                Arc::new(RuntimeSnapshot {
                    document_ready: true,
                    ..(*full).clone()
                }),
            );
            cx.update(|_, cx| app.update(cx, |app, cx| app.drain_runtime(cx)));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert_eq!(cx.debug_bounds("transcript-row-19"), Some(reply));
            assert!(cx.debug_bounds("transcript-older-loading").is_none());
        },
    );
}

#[test]
fn transcript_reading_matches_only_a_unique_page_before_new_items() {
    fn items(messages: &[&str]) -> super::Items {
        let mut conversation = crate::conversation::ConversationState::default();
        let messages: Vec<_> = messages
            .iter()
            .map(|text| {
                serde_json::json!({
                    "role": "user", "content": text
                })
            })
            .collect();
        conversation.replace_history(&messages);
        conversation.items
    }
    let page = items(&["a", "a"]);
    assert_eq!(
        super::unique_page_start(&items(&["x", "a", "a"]), &page),
        Some(1)
    );
    assert_eq!(
        super::unique_page_start(&items(&["a", "a", "a"]), &page),
        None
    );
    for (full, expected) in [
        (vec!["a", "x", "a", "a", "b"], Some(2)),
        (vec!["a", "a", "a", "b"], None),
        (vec!["x", "a", "a"], None),
        (vec!["a", "x", "a", "b"], None),
    ] {
        assert_eq!(
            super::unique_page_before_append(&items(&full), &page),
            expected
        );
    }
}

#[gpui::test]
fn transcript_reading_keeps_painted_rows_when_full_history_has_new_turns(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::transcript_reading_keeps_painted_rows_when_full_history_has_new_turns"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.simulate_resize(size(px(1200.), px(650.)));
            let transcript = cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.selected_draft = None;
                    cx.notify();
                    app.views.transcript.clone()
                })
            });
            for (generation, name, paint, saved_between) in [
                (1, "painted", true, false),
                (2, "unpainted", false, false),
                (3, "saved", true, true),
                (4, "unpainted-saved", false, true),
            ] {
                let first = reading_snapshot(project, name, 44);
                let mut tail = crate::conversation::ConversationState::default();
                tail.items = first
                    .conversation
                    .items
                    .iter_range(28..44)
                    .cloned()
                    .collect();
                let page = Arc::new(RuntimeSnapshot {
                    status: "Loading history".into(),
                    conversation: Default::default(),
                    history_tail: Some(Arc::new(crate::agents::HistoryPage {
                        content: tail,
                        coverage: crate::agents::HistoryCoverage::Suffix,
                    })),
                    ..(*first).clone()
                });
                send_snapshot(runtime, generation, page);
                let original_y = paint.then(|| {
                    cx.update(|_, cx| app.update(cx, |app, cx| app.drain_runtime(cx)));
                    cx.update(|window, cx| window.draw(cx).clear(cx));
                    cx.debug_bounds("transcript-row-15")
                        .expect("painted page end")
                        .origin
                        .y
                });
                cx.update(|_, cx| {
                    if saved_between {
                        send_snapshot(
                            runtime,
                            generation,
                            Arc::new(RuntimeSnapshot {
                                document_ready: true,
                                saved_history: Some(crate::runtime::SavedHistory::Complete),
                                ..(*first).clone()
                            }),
                        );
                        app.update(cx, |app, cx| app.drain_runtime(cx));
                    }
                    let full = reading_snapshot(project, name, 66);
                    send_snapshot(
                        runtime,
                        generation,
                        Arc::new(RuntimeSnapshot {
                            document_ready: true,
                            ..(*full).clone()
                        }),
                    );
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                });
                for _ in 0..3 {
                    cx.update(|window, cx| window.draw(cx).clear(cx));
                    if let Some(y) = original_y {
                        assert_eq!(
                            cx.debug_bounds("transcript-row-43")
                                .expect("original page end")
                                .origin
                                .y,
                            y
                        );
                        assert!(cx.debug_bounds("transcript-row-65").is_none());
                        cx.update(|_, cx| {
                            assert!(!transcript.read(cx).following);
                            assert_eq!(transcript.read(cx).unseen, 22);
                        });
                    } else {
                        assert!(cx.debug_bounds("transcript-row-65").is_some());
                        cx.update(|_, cx| assert!(transcript.read(cx).following));
                    }
                }
                if paint {
                    cx.update(|window, cx| {
                        app.update(cx, |app, cx| app.jump_to_latest(cx));
                        window.draw(cx).clear(cx);
                    });
                    assert!(cx.debug_bounds("transcript-row-65").is_some());
                    cx.update(|_, cx| assert_eq!(transcript.read(cx).unseen, 0));
                }
            }
        },
    );
}

#[test]
fn transcript_reading_preserves_the_visible_group_when_earlier_tools_join_it() {
    use crate::app::views::transcript::project_rows;
    use crate::conversation::ConversationState;
    let mut full = ConversationState::default();
    full.replace_history(&(0..3).map(|i| serde_json::json!({
        "role":"assistant", "content":[{"type":"toolCall", "id":format!("tool-{i}"), "name":"bash","arguments":{}}]
    })).collect::<Vec<_>>());
    let suffix: super::Items = full.items.iter_range(1..3).cloned().collect();
    let old_rows = project_rows(&suffix);
    let rows = project_rows(&full.items);
    let source = super::SourceItem::new(0, &suffix).unwrap();
    let mut saved = super::ReadingState {
        suffix: Some(suffix.clone()),
        anchor: super::ReadingAnchor::at(old_rows[0], px(9.), &suffix),
        disclosures: [(old_rows[0].disclosure_key(), (source, true))].into(),
        file_trees: Default::default(),
        last_count: old_rows.len(),
        unseen: 0,
    };
    let start = saved
        .source_offset(crate::agents::HistoryCoverage::Complete, &full.items)
        .unwrap();
    saved.rebase(start, &rows);
    assert_eq!(rows.len(), 1);
    assert!(matches!(
        rows[0],
        TranscriptRow::ActivityGroup {
            start: 0,
            len: 3,
            ..
        }
    ));
    assert_eq!(
        saved.anchor.unwrap().resolve(&rows, &full.items),
        Some(ListOffset {
            item_ix: 0,
            offset_in_item: px(9.)
        })
    );
    assert!(saved.disclosures[&rows[0].disclosure_key()].1);
}

#[gpui::test]
fn transcript_reading_maps_a_scrolled_suffix_when_older_messages_arrive(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::transcript_reading_maps_a_scrolled_suffix_when_older_messages_arrive"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.simulate_resize(size(px(1200.), px(650.)));
            let transcript = cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.selected_draft = None;
                    cx.notify();
                    app.views.transcript.clone()
                })
            });
            for (generation, name, switch_away) in [(1, "selected", false), (2, "parked", true)] {
                let mut full = reading_snapshot(project, name, 100);
                let snapshot = Arc::make_mut(&mut full);
                snapshot.document_ready = true;
                let items = &mut Arc::make_mut(&mut snapshot.conversation).items;
                items.set(84, items[4].clone());
                let mut suffix = crate::conversation::ConversationState::default();
                suffix.items = items.iter_range(80..100).cloned().collect();
                let tail_top = if switch_away {
                    None
                } else {
                    let mut tail = crate::conversation::ConversationState::default();
                    tail.items = items.iter_range(90..100).cloned().collect();
                    send_snapshot(
                        runtime,
                        generation,
                        Arc::new(RuntimeSnapshot {
                            status: "Loading history".into(),
                            document_ready: false,
                            conversation: Default::default(),
                            history_tail: Some(Arc::new(crate::agents::HistoryPage {
                                content: tail,
                                coverage: crate::agents::HistoryCoverage::Suffix,
                            })),
                            ..snapshot.clone()
                        }),
                    );
                    Some(cx.update(|window, cx| {
                        app.update(cx, |app, cx| app.drain_runtime(cx));
                        window.draw(cx).clear(cx);
                        let page = transcript
                            .read(cx)
                            .history_tail
                            .as_ref()
                            .unwrap()
                            .list
                            .clone();
                        page.scroll_by(px(-80.), window, transcript.entity_id());
                        transcript.update(cx, |_, cx| cx.notify());
                        window.draw(cx).clear(cx);
                        assert!(!page.is_following_tail());
                        page.painted_history_top().unwrap()
                    }))
                };
                send_snapshot(
                    runtime,
                    generation,
                    Arc::new(RuntimeSnapshot {
                        status: "Partial saved history".into(),
                        saved_history: Some(crate::runtime::SavedHistory::RefreshingPartial),
                        conversation: Arc::new(suffix),
                        ..snapshot.clone()
                    }),
                );
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                    window.draw(cx).clear(cx);
                    transcript.update(cx, |view, cx| {
                        if let Some(top) = tail_top {
                            assert!(!view.following);
                            assert_eq!(
                                view.list.logical_scroll_top(),
                                ListOffset {
                                    item_ix: 10 + top.item_ix,
                                    ..top
                                }
                            );
                        }
                        view.list.scroll_to(ListOffset {
                            item_ix: 4,
                            offset_in_item: px(9.),
                        });
                        view.following = false;
                        view.disclosure_states.insert(4, true);
                        view.unseen = 2;
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                let before = cx
                    .debug_bounds("transcript-row-4")
                    .expect("visible suffix row");
                if switch_away {
                    send_snapshot(
                        runtime,
                        generation + 1,
                        reading_snapshot(project, "other", 20),
                    );
                    cx.update(|_, cx| app.update(cx, |app, cx| app.drain_runtime(cx)));
                }
                send_snapshot(
                    runtime,
                    if switch_away {
                        generation + 2
                    } else {
                        generation
                    },
                    full,
                );
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                    window.draw(cx).clear(cx);
                    let view = transcript.read(cx);
                    let offset = view.list.logical_scroll_top();
                    assert_eq!(offset.item_ix, 84);
                    assert_eq!(offset.offset_in_item, px(9.));
                    assert_eq!(view.unseen, 2);
                    assert_eq!(view.disclosure_states.get(&84), Some(&true));
                    assert!(!view.disclosure_states.contains_key(&4));
                });
                assert_eq!(
                    cx.debug_bounds("transcript-row-84").unwrap().origin.y,
                    before.origin.y
                );
            }
        },
    );
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
            let transcript = cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.selected_draft = None;
                    cx.notify();
                    app.views.transcript.clone()
                })
            });
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
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let saved_y = cx
                .debug_bounds("transcript-row-12")
                .expect("saved row")
                .origin
                .y;
            let updated = reading_snapshot(project, "a", 64);
            for (generation, snapshot) in
                [(2, b.clone()), (3, updated.clone()), (4, b), (5, updated)]
            {
                let items = &snapshot.conversation.items;
                let mut tail = crate::conversation::ConversationState::default();
                tail.items = items
                    .iter_range(items.len() - 16..items.len())
                    .cloned()
                    .collect();
                runtime.send_event(RuntimeEvent::SessionReset {
                    generation,
                    preserve_submission: false,
                });
                send_snapshot(
                    runtime,
                    generation,
                    Arc::new(RuntimeSnapshot {
                        conversation: Default::default(),
                        history_tail: Some(Arc::new(crate::agents::HistoryPage {
                            content: tail,
                            coverage: crate::agents::HistoryCoverage::Suffix,
                        })),
                        status: "Loading history".into(),
                        ..(*snapshot).clone()
                    }),
                );
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                    window.draw(cx).clear(cx);
                });
                let returning = generation == 3 || generation == 5;
                if returning {
                    assert!(cx.debug_bounds("transcript-loading").is_some());
                    assert!(cx.debug_bounds("transcript-row-15").is_none());
                    let mut partial = crate::conversation::ConversationState::default();
                    partial.items = items
                        .iter_range(items.len() - 8..items.len())
                        .cloned()
                        .collect();
                    let partial = Arc::new(RuntimeSnapshot {
                        status: "Partial saved history".into(),
                        saved_history: Some(crate::runtime::SavedHistory::RefreshingPartial),
                        document_ready: true,
                        conversation: Arc::new(partial),
                        ..(*snapshot).clone()
                    });
                    send_snapshot(runtime, generation, partial.clone());
                    for _ in 0..2 {
                        cx.update(|window, cx| {
                            app.update(cx, |app, cx| app.drain_runtime(cx));
                            window.draw(cx).clear(cx);
                        });
                        assert!(cx.debug_bounds("transcript-loading").is_some());
                        assert!(cx.debug_bounds("transcript-row-7").is_none());
                        assert!(cx.debug_bounds("transcript-position-unavailable").is_none());
                    }
                    if generation == 5 {
                        let mut failed = partial;
                        Arc::make_mut(&mut failed).saved_history =
                            Some(crate::runtime::SavedHistory::Partial);
                        send_snapshot(runtime, generation, failed);
                        cx.update(|window, cx| {
                            app.update(cx, |app, cx| app.drain_runtime(cx));
                            window.draw(cx).clear(cx);
                        });
                        assert!(cx.debug_bounds("transcript-loading").is_none());
                        assert!(cx.debug_bounds("transcript-row-7").is_some());
                        assert!(cx.debug_bounds("transcript-position-unavailable").is_some());
                    }
                }
                runtime.send_event(RuntimeEvent::HistoryReset { generation });
                send_snapshot(runtime, generation, snapshot);
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                    window.draw(cx).clear(cx);
                    let view = transcript.read(cx);
                    if !returning {
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
                if returning {
                    for _ in 0..3 {
                        assert!(cx.debug_bounds("transcript-position-unavailable").is_none());
                        let row = cx.debug_bounds("transcript-row-12").expect("restored row");
                        assert_eq!(row.origin.y, saved_y);
                        cx.update(|window, cx| window.draw(cx).clear(cx));
                    }
                }
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
            let last_row = cx
                .debug_bounds("transcript-row-59")
                .expect("last row after resize");
            cx.update(|_, cx| {
                let view = transcript.read(cx);
                assert!(view.list.is_following_tail());
                assert!(
                    view.following,
                    "restoration reached the bottom but left the view paused"
                );
                assert_eq!(view.unseen, 0);
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert_eq!(cx.debug_bounds("transcript-row-59"), Some(last_row));
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

#[gpui::test]
fn reopened_page_transfers_only_reader_input_over_a_saved_anchor(cx: &mut gpui::TestAppContext) {
    use super::super::{DisplayQuery, DisplayRead, PreparedPage};
    use crate::{
        agents::HistoryCoverage,
        storage::{StoredViewport, TranscriptStamp},
    };
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::reopened_page_transfers_only_reader_input_over_a_saved_anchor"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.simulate_resize(size(px(1200.), px(650.)));
            let transcript = cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.selected_draft = None;
                    cx.notify();
                    app.views.transcript.clone()
                })
            });
            let old = ListOffset {
                item_ix: 12,
                offset_in_item: px(9.),
            };
            for (i, delta, draw) in [
                (0, 0., true),
                (1, 20., false),
                (2, 20., true),
                (3, 100_000., false),
            ] {
                let name = format!("reopen-{i}");
                let generation = 3 * i + 1;
                let mut ready = reading_snapshot(project, &name, 60);
                Arc::make_mut(&mut ready).document_ready = true;
                let path = ready.selected_session.clone().unwrap();
                send_snapshot(runtime, generation, ready.clone());
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                    transcript.update(cx, |view, cx| {
                        view.list.restore_scroll(old);
                        view.following = false;
                        view.unseen = 4;
                        view.disclosure_states.insert(12, true);
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                send_snapshot(
                    runtime,
                    generation + 1,
                    reading_snapshot(project, "away", 2),
                );
                cx.update(|_, cx| {
                    app.update(cx, |app, cx| {
                        app.drain_runtime(cx);
                        let page = PreparedPage::from_store(
                            StoredViewport {
                                full_document: true,
                                coverage: HistoryCoverage::Complete,
                                content: (*ready.conversation).clone(),
                                markdown: Default::default(),
                                revision: 1,
                                stamp: TranscriptStamp {
                                    session_id: 1,
                                    revision: 1,
                                    prompt_event_seq: 0,
                                },
                            },
                            project,
                        )
                        .unwrap();
                        app.present_or_read_transcript(
                            DisplayQuery {
                                trace: None,
                                path,
                                project: project.into(),
                                harness: crate::agents::Backend::Pi,
                                profile: None,
                            },
                            None,
                            Some(DisplayRead::Page(page)),
                            cx,
                        );
                    })
                });
                send_snapshot(
                    runtime,
                    generation + 2,
                    Arc::new(RuntimeSnapshot {
                        status: "Loading history".into(),
                        document_ready: false,
                        conversation: Default::default(),
                        ..(*ready).clone()
                    }),
                );
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                    window.draw(cx).clear(cx);
                    let page = transcript
                        .read(cx)
                        .selected_local_page(app.read(cx))
                        .unwrap()
                        .list
                        .clone();
                    assert_eq!(page.painted_history_top(), Some(old));
                    if delta == 0. {
                        page.scroll_to(ListOffset { item_ix: 16, ..old });
                    } else {
                        page.scroll_by(px(delta), window, transcript.entity_id());
                        assert!(page.has_pending_scroll());
                    }
                    if draw {
                        transcript.update(cx, |_, cx| cx.notify());
                        window.draw(cx).clear(cx);
                    }
                });
                let appended = if draw && delta > 0. { 2 } else { 0 };
                if appended > 0 {
                    ready = reading_snapshot(project, &name, 60 + appended);
                    Arc::make_mut(&mut ready).document_ready = true;
                }
                send_snapshot(runtime, generation + 2, ready);
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                    window.draw(cx).clear(cx);
                    let view = transcript.read(cx);
                    assert_eq!(view.disclosure_states.get(&12), Some(&true));
                    if delta == 100_000. {
                        assert!(view.following && view.list.is_following_tail());
                        assert_eq!(view.unseen, 0);
                    } else {
                        assert!(!view.following);
                        assert_eq!(
                            view.list.logical_scroll_top(),
                            ListOffset {
                                offset_in_item: old.offset_in_item + px(delta),
                                ..old
                            }
                        );
                        assert_eq!(view.unseen, 4 + appended);
                    }
                });
            }
        },
    );
}

#[track_caller]
fn wait_for_resident_selection(
    cx: &mut gpui::VisualTestContext,
    app: &gpui::Entity<crate::app::FarcasterApp>,
    row: &crate::sessions::SessionSummary,
) -> u64 {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        let request = cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            let app = app.read(cx);
            (app.lifecycle.requested_session_key.as_ref() == Some(&row.catalog_key())
                && app.lifecycle.pending_session_switch.is_some())
            .then_some(app.lifecycle.switch_sequence)
        });
        if let Some(request) = request {
            return request;
        }
        if std::time::Instant::now() >= deadline {
            cx.update(|_,cx| {
                let app = app.read(cx);
                panic!("open {:?} did not route; selected {:?}, requested {:?}, pending {}, trust {}, lookup {}", row.path, app.snapshot.selected_session, app.lifecycle.requested_session_key, app.lifecycle.pending_session_switch.is_some(),app.project.pending_trust_command.is_some(),app.lifecycle.opening_local.is_some());
            });
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[gpui::test]
fn resident_transcript_draws_saved_anchor_before_routing_and_reuses_layout(
    cx: &mut gpui::TestAppContext,
) {
    use crate::{agents::Backend, sessions::SessionSummary};
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::resident_transcript_draws_saved_anchor_before_routing_and_reuses_layout"
        ),
        cx,
        |cx, app, runtime, project| {
            let canonical_project = project.canonicalize().unwrap();
            let project = canonical_project.as_path();
            crate::app::project::trust::apply(project, crate::projects::TrustChoice::TrustProject)
                .unwrap();
            let row = |name: &str, id| {
                SessionSummary::from_cached(
                    name.into(),
                    project.join(name),
                    project.into(),
                    name.into(),
                    String::new(),
                    String::new(),
                    None,
                    std::time::UNIX_EPOCH,
                    80,
                    Default::default(),
                    false,
                    false,
                    String::new(),
                )
                .with_app_session_id(id)
            };
            let a = row("a", 1);
            let b = row("b", 2);
            let snapshot = |name| {
                let mut snap = reading_snapshot(project, name, 80);
                let value = Arc::make_mut(&mut snap);
                value.document_ready = true;
                value.harness = Some(Backend::Pi);
                snap
            };
            use crate::storage::{SessionRepository as _, TranscriptSession};
            let session = TranscriptSession {
                path: &a.path,
                project,
                harness: a.harness,
                profile_id: None,
            };
            let repo = crate::app::persistence::shared().unwrap();
            let mut messages: Vec<_> = (0..80).map(|index| serde_json::json!({
                "role": if index % 2 == 0 { "user" } else { "assistant" },
                "content": [{"type": "text", "text": format!("a message {index}: {}", "reading context ".repeat(30))}]
            })).collect();
            let stamp = repo
                .replace_transcript_with_stamp(session, 0, &messages, &serde_json::Value::Null)
                .unwrap()
                .unwrap();
            let mut ready_a = snapshot("a");
            Arc::make_mut(&mut ready_a).transcript_stamp = Some(stamp);
            let transcript = cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.all = vec![a.clone(), b.clone()].into();
                    app.sessions.visible = vec![a.clone(), b.clone()].into();
                    app.sessions.selected_draft = None;
                    cx.notify();
                    app.views.transcript.clone()
                })
            });
            cx.simulate_resize(size(px(1200.0), px(650.0)));
            send_snapshot(runtime, 1, ready_a.clone());
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
                transcript.update(cx, |view, cx| {
                    view.list.scroll_to(ListOffset {
                        item_ix: 12,
                        offset_in_item: px(9.0),
                    });
                    view.following = false;
                    view.unseen = 4;
                    view.disclosure_states.insert(12, true);
                    view.file_trees
                        .entry(12)
                        .or_default()
                        .toggle(project, Path::new("src"));
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            let y = cx
                .debug_bounds("transcript-row-12")
                .expect("reading anchor")
                .origin
                .y;
            let original = cx.update(|_, cx| transcript.read(cx).rows.clone());
            send_snapshot(runtime, 2, snapshot("b"));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
                let view = transcript.read(cx);
                assert!(view.has_resident_for(&a));
                let local = repo
                    .load_transcript_display(session, true)
                    .unwrap()
                    .unwrap();
                let saved = super::super::PreparedPage::from_store(local, project).unwrap();
                let page = view
                    .local_layout(saved, &a.path, Some(a.catalog_key()))
                    .unwrap();
                assert!(page.list.has_pending_restore());
                assert_eq!(page.disclosures.get(&12), Some(&true));
                assert!(!page.trees[&12].is_open(project, Path::new("src")));
                let mut changed = repo
                    .load_transcript_display(session, true)
                    .unwrap()
                    .unwrap();
                changed.coverage = crate::agents::HistoryCoverage::Suffix;
                let repeated = changed.content.items[12].clone();
                changed.content.items = changed.content.items.iter().skip(2).cloned().collect();
                changed.content.items.set(12, repeated);
                let saved = super::super::PreparedPage::from_store(changed, project).unwrap();
                assert!(
                    view.local_layout(saved, &a.path, Some(a.catalog_key()))
                        .is_none()
                );
                for sibling in [
                    SessionSummary {
                        harness: Backend::Codex,
                        ..a.clone()
                    },
                    SessionSummary {
                        profile_id: Some("other".into()),
                        ..a.clone()
                    },
                    SessionSummary {
                        message_count: a.message_count + 1,
                        ..a.clone()
                    },
                    SessionSummary {
                        app_session_id: 55,
                        ..a.clone()
                    },
                ] {
                    assert!(!view.has_resident_for(&sibling));
                }
            });
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.select_rail_session(a.catalog_key(), window, cx)
                })
            });
            let request = wait_for_resident_selection(cx, app, &a);
            assert_eq!(
                cx.debug_bounds("transcript-row-12")
                    .expect("before route")
                    .origin
                    .y,
                y
            );
            runtime.send_event(RuntimeEvent::SelectionRouted(request));
            send_snapshot(runtime, 3, ready_a.clone());
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
                let view = transcript.read(cx);
                assert!(
                    Arc::ptr_eq(&original, &view.rows),
                    "reused exact projected rows"
                );
                assert_eq!(view.disclosure_states.get(&12), Some(&true));
                assert!(!view.file_trees[&12].is_open(project, Path::new("src")));
                assert!(!view.following);
                assert_eq!(view.unseen, 4);
            });
            assert_eq!(
                cx.debug_bounds("transcript-row-12")
                    .expect("after route")
                    .origin
                    .y,
                y
            );

            send_snapshot(runtime, 4, snapshot("b"));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.drain_runtime(cx);
                    app.select_session_at_latest(a.path.clone(), a.project.clone(), window, cx);
                });
            });
            let request = wait_for_resident_selection(cx, app, &a);
            assert!(cx.debug_bounds("transcript-row-79").is_some());
            runtime.send_event(RuntimeEvent::SelectionRouted(request));
            send_snapshot(runtime, 5, ready_a);
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
                let view = transcript.read(cx);
                assert!(view.list.is_following_tail());
                assert_eq!(view.unseen, 0);
            });
            assert!(cx.debug_bounds("transcript-row-79").is_some());

            send_snapshot(runtime, 6, snapshot("b"));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
            });
            messages[79]["content"][0]["text"] = "edited before returning".into();
            let updated = repo
                .replace_transcript_with_stamp(
                    session,
                    stamp.revision,
                    &messages,
                    &serde_json::Value::Null,
                )
                .unwrap()
                .unwrap();
            assert_ne!(updated, stamp);
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.select_rail_session(a.catalog_key(), window, cx)
                });
            });
            wait_for_resident_selection(cx, app, &a);
            cx.update(|_, cx| {
                let app = app.read(cx);
                let items = transcript
                    .read(cx)
                    .local_display_items(app)
                    .expect("painted latest saved page");
                assert_eq!(items.last().unwrap().text, "edited before returning");
            });
            assert!(cx.debug_bounds("transcript-loading").is_none());
        },
    );
}
