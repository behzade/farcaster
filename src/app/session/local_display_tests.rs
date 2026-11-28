use super::switch_perf_tests::{SWITCH_POLL, SWITCH_TIMEOUT};
use crate::{
    agents::Backend,
    app::{FarcasterApp, project::trust},
    conversation::ConversationState,
    projects::TrustChoice,
    runtime::{RuntimeCommand, RuntimeEvent, RuntimeSnapshot, TestRuntime},
    sessions::SessionSummary,
    storage::{SessionRepository as _, TranscriptSession},
};
use gpui::{Bounds, Entity, Pixels, VisualTestContext};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn messages(name: &str, count: usize) -> Vec<Value> {
    (0..count)
        .map(|i| json!({"role": if i % 2 == 0 {"user"} else {"assistant"}, "content":[{"type":"text", "text":format!("{name} message {i}")}]}))
        .collect()
}

fn store_chat(project: &Path, path: &Path, harness: Backend, name: &str, count: usize) {
    let repository = crate::app::persistence::shared().unwrap();
    assert!(
        repository
            .replace_transcript(
                TranscriptSession {
                    harness,
                    path,
                    project,
                    profile_id: None
                },
                0,
                &messages(name, count),
                &json!({}),
            )
            .unwrap()
    );
}

fn snapshot(row: &SessionSummary, messages: &[Value]) -> Arc<RuntimeSnapshot> {
    let mut conversation = ConversationState::default();
    conversation.replace_history(messages);
    Arc::new(RuntimeSnapshot {
        selected_session: Some(row.path.clone()),
        project: row.project.clone(),
        harness: Some(row.harness),
        conversation: Arc::new(conversation),
        document_ready: true,
        status: "Ready".into(),
        ..Default::default()
    })
}

fn click(cx: &mut VisualTestContext, app: &Entity<FarcasterApp>, row: &SessionSummary) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.select_rail_session(row.catalog_key(), window, cx)
        });
        window.draw(cx).clear(cx);
    });
}

fn assert_selected(app: &FarcasterApp, row: &SessionSummary) {
    assert_eq!(
        app.selected_rail_root().unwrap().catalog_key(),
        row.catalog_key()
    );
    assert_eq!(
        app.composer.sessions.current_buffer(),
        crate::sessions::session_buffer(row.harness, &row.path)
    );
}

fn wait_for_local_paint(
    cx: &mut VisualTestContext,
    app: &Entity<FarcasterApp>,
    text: &str,
    key: &'static str,
) -> Bounds<Pixels> {
    let deadline = Instant::now() + SWITCH_TIMEOUT;
    loop {
        cx.run_until_parked();
        let loaded = cx.update(|window, cx| {
            app.update(cx, |app, cx| app.drain_runtime(cx));
            window.draw(cx).clear(cx);
            let app = app.read(cx);
            let view = app.views.transcript.read(cx);
            view.local_display_items(app).is_some_and(|items| {
                assert!(app.awaiting_selection_snapshot());
                assert!(
                    items.iter().any(|item| item.text == text),
                    "wrong chat supplied local rows while awaiting {text}"
                );
                true
            })
        });
        if loaded && let Some(bounds) = cx.debug_bounds(key) {
            assert!(cx.debug_bounds("transcript-loading").is_none());
            return bounds;
        }
        assert!(
            Instant::now() < deadline,
            "no SQLite paint for {text} before route"
        );
        thread::sleep(SWITCH_POLL);
    }
}

fn with_saved_chat(
    name: &str,
    cx: &mut gpui::TestAppContext,
    check: impl FnOnce(&mut VisualTestContext, &Entity<FarcasterApp>, &TestRuntime, SessionSummary),
) {
    crate::app::test_support::with_prepared_offline_app(
        name,
        cx,
        |project| {
            let project = project.canonicalize().unwrap();
            store_chat(&project, &project.join("saved"), Backend::Pi, "saved", 2);
        },
        |cx, app, runtime, _| {
            let row = crate::app::persistence::open()
                .unwrap()
                .cached_sessions("")
                .unwrap()
                .into_iter()
                .find(|row| row.path.ends_with("saved"))
                .unwrap();
            trust::apply(&row.project, TrustChoice::TrustProject).unwrap();
            crate::agents::apply_project_trust(
                row.harness,
                &row.project,
                TrustChoice::TrustProject,
            )
            .unwrap();
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    app.sessions.all = vec![row.clone()].into();
                    app.sessions.visible = vec![row.clone()].into();
                })
            });
            check(cx, app, runtime, row);
        },
    );
}

#[gpui::test]
fn saved_selection_supersedes_unresolved_catalog_retry(cx: &mut gpui::TestAppContext) {
    with_saved_chat(
        concat!(
            module_path!(),
            "::saved_selection_supersedes_unresolved_catalog_retry"
        ),
        cx,
        |cx, app, runtime, saved| {
            let older = saved.project.join("older");
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.select_session(older.clone(), saved.project.clone(), window, cx);
                });
            });
            cx.run_until_parked();
            assert!(
                std::iter::from_fn(|| runtime.try_recv_command())
                    .any(|command| matches!(command, RuntimeCommand::RefreshSessions))
            );

            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.select_rail_session(saved.catalog_key(), window, cx);
                    assert!(app.lifecycle.opening_local.is_some());
                    app.sessions.all = Default::default();
                    app.sessions.visible = Default::default();
                    let mut target = saved.target();
                    target.path = older.clone();
                    app.runtime.session_targets.insert(older.clone(), target);
                });
            });
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_millis(101));
            cx.run_until_parked();
            cx.update(|_, cx| {
                let app = app.read(cx);
                assert!(app.lifecycle.opening_local.is_none());
                assert!(app.lifecycle.pending_session_switch.is_none());
                assert!(app.sessions.error.is_none());
            });
            assert!(!std::iter::from_fn(|| runtime.try_recv_command())
                .any(|command| matches!(command, RuntimeCommand::SelectSession { path, .. } if path == older)));
        },
    );
}

#[gpui::test]
fn forking_cancels_a_staged_saved_selection(cx: &mut gpui::TestAppContext) {
    with_saved_chat(
        concat!(module_path!(), "::forking_cancels_a_staged_saved_selection"),
        cx,
        |cx, app, runtime, saved| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.select_rail_session(saved.catalog_key(), window, cx);
                    assert!(app.lifecycle.opening_local.is_some());
                    app.fork_session(saved.path.clone(), saved.project.clone(), window, cx);
                });
            });
            cx.run_until_parked();
            let mut forked = false;
            for command in std::iter::from_fn(|| runtime.try_recv_command()) {
                match command {
                    RuntimeCommand::ForkSession { path, .. } => {
                        assert_eq!(path, saved.path);
                        forked = true;
                    }
                    RuntimeCommand::SelectSession { .. } => {
                        panic!("older selection routed after fork")
                    }
                    _ => {}
                }
            }
            assert!(forked);
            cx.update(|_, cx| {
                let app = app.read(cx);
                assert!(app.lifecycle.opening_local.is_none());
                assert!(app.lifecycle.pending_session_switch.is_none());
            });
        },
    );
}

#[gpui::test]
fn saved_chat_paints_before_routing_and_ignores_superseded_reads(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_prepared_offline_app(
        concat!(
            module_path!(),
            "::saved_chat_paints_before_routing_and_ignores_superseded_reads"
        ),
        cx,
        |project| {
            let project = project.canonicalize().unwrap();
            let shared = project.join("shared-locator");
            store_chat(&project, &shared, Backend::Pi, "pi", 2);
            store_chat(&project, &shared, Backend::Codex, "codex", 4);
            store_chat(&project, &project.join("third"), Backend::Pi, "third", 6);
        },
        |cx, app, runtime, project| {
            cx.simulate_resize(gpui::size(gpui::px(1600.), gpui::px(900.)));
            let project = project.canonicalize().unwrap();
            trust::apply(&project, TrustChoice::TrustProject).unwrap();
            let rows = crate::app::persistence::open()
                .unwrap()
                .cached_sessions("")
                .unwrap();
            let pi = rows
                .iter()
                .find(|r| r.harness == Backend::Pi && r.path.ends_with("shared-locator"))
                .unwrap();
            let codex = rows.iter().find(|r| r.harness == Backend::Codex).unwrap();
            let third = rows.iter().find(|r| r.path.ends_with("third")).unwrap();
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    app.sessions.all = rows.clone().into();
                    app.sessions.visible = rows.clone().into();
                })
            });
            assert!(rows.iter().all(|row| row.has_local_transcript));
            let draft_id = cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.new_session(project.clone(), window, cx);
                    let draft_id = app.sessions.selected_draft.clone().unwrap();
                    app.select_rail_session(pi.catalog_key(), window, cx);
                    assert!(app.lifecycle.opening_local.is_some());
                    app.resume_draft(draft_id.clone(), project.clone(), window, cx);
                    assert!(app.lifecycle.opening_local.is_none());
                    draft_id
                })
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                let app = app.read(cx);
                assert_eq!(app.sessions.selected_draft.as_ref(), Some(&draft_id));
                assert!(
                    app.views
                        .transcript
                        .read(cx)
                        .local_display_items(app)
                        .is_none()
                );
            });
            click(cx, app, pi);
            let pi_bounds = wait_for_local_paint(cx, app, "pi message 1", "transcript-row-1");
            let old_route =
                cx.update(|_, cx| app.read(cx).lifecycle.awaiting_switch_route.unwrap());
            runtime.send_event(RuntimeEvent::SelectionRouted(old_route));
            runtime.send_event(RuntimeEvent::Snapshot {
                generation: 1,
                snapshot: snapshot(pi, &messages("pi", 2)),
            });
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
                let app = app.read(cx);
                assert!(!app.awaiting_selection_snapshot());
                assert_selected(app, pi);
            });
            assert_eq!(cx.debug_bounds("transcript-row-1"), Some(pi_bounds));

            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.select_rail_session(codex.catalog_key(), window, cx);
                    app.select_rail_session(third.catalog_key(), window, cx);
                });
                window.draw(cx).clear(cx);
                let app = app.read(cx);
                assert_selected(app, pi);
                assert_eq!(app.snapshot.conversation.items[1].text, "pi message 1");
            });
            assert_eq!(cx.debug_bounds("transcript-row-1"), Some(pi_bounds));
            assert!(cx.debug_bounds("transcript-loading").is_none());

            let bounds = wait_for_local_paint(cx, app, "third message 5", "transcript-row-5");
            let current_route = cx.update(|_, cx| {
                let app = app.read(cx);
                assert_selected(app, third);
                app.lifecycle.awaiting_switch_route.unwrap()
            });
            runtime.send_event(RuntimeEvent::SelectionRouted(old_route));
            for (row, name, count) in [(pi, "pi", 2), (codex, "codex", 4)] {
                runtime.send_event(RuntimeEvent::Snapshot {
                    generation: 1,
                    snapshot: snapshot(row, &messages(name, count)),
                });
                assert_eq!(
                    wait_for_local_paint(cx, app, "third message 5", "transcript-row-5"),
                    bounds
                );
            }
            runtime.send_event(RuntimeEvent::SelectionRouted(current_route));
            assert_eq!(
                wait_for_local_paint(cx, app, "third message 5", "transcript-row-5"),
                bounds
            );
            std::fs::write(&third.path, "{}").unwrap();
            super::remembered_transcript::remember(&snapshot(third, &messages("old", 1)));
            let loading = Arc::new(RuntimeSnapshot {
                selected_session: Some(third.path.clone()),
                project: project.clone(),
                harness: Some(third.harness),
                status: "Loading history".into(),
                ..Default::default()
            });
            assert_eq!(
                super::remembered_transcript::stand_in(loading.clone())
                    .conversation
                    .items[0]
                    .text,
                "old message 0"
            );
            runtime.send_event(RuntimeEvent::Snapshot {
                generation: 2,
                snapshot: loading,
            });
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
                let app = app.read(cx);
                assert_eq!(
                    app.views
                        .transcript
                        .read(cx)
                        .local_display_items(app)
                        .unwrap()[5]
                        .text,
                    "third message 5"
                );
            });
            assert_eq!(cx.debug_bounds("transcript-row-5"), Some(bounds));
            assert!(cx.debug_bounds("transcript-loading").is_none());
            runtime.send_event(RuntimeEvent::Snapshot {
                generation: 2,
                snapshot: snapshot(third, &messages("third", 6)),
            });
            for _ in 0..4 {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                    window.draw(cx).clear(cx);
                    let app = app.read(cx);
                    assert!(!app.awaiting_selection_snapshot());
                    assert_eq!(app.snapshot.conversation.items[5].text, "third message 5");
                });
                assert_eq!(cx.debug_bounds("transcript-row-5"), Some(bounds));
            }

            click(cx, app, codex);
            wait_for_local_paint(cx, app, "codex message 3", "transcript-row-3");
            let rejected = cx.update(|_, cx| app.read(cx).lifecycle.awaiting_switch_route.unwrap());
            runtime.send_event(RuntimeEvent::SelectionRejected(rejected));
            cx.update(|_, cx| app.update(cx, |app, cx| app.drain_runtime(cx)));
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                let app = app.read(cx);
                assert!(!app.awaiting_selection_snapshot());
                assert_eq!(app.snapshot.conversation.items[5].text, "third message 5");
                assert!(
                    app.views
                        .transcript
                        .read(cx)
                        .local_display_items(app)
                        .is_none()
                );
            });
            assert_eq!(cx.debug_bounds("transcript-row-5"), Some(bounds));
        },
    );
}

#[gpui::test]
fn short_saved_suffix_paints_before_route_and_stays_at_its_final_position(
    cx: &mut gpui::TestAppContext,
) {
    check_saved_suffix_fit(
        cx,
        concat!(
            module_path!(),
            "::short_saved_suffix_paints_before_route_and_stays_at_its_final_position"
        ),
        6000.,
        false,
    );
}

#[gpui::test]
fn wrapped_saved_suffix_paints_without_reading_the_full_document(cx: &mut gpui::TestAppContext) {
    check_saved_suffix_fit(
        cx,
        concat!(
            module_path!(),
            "::wrapped_saved_suffix_paints_without_reading_the_full_document"
        ),
        2000.,
        true,
    );
}

fn check_saved_suffix_fit(
    cx: &mut gpui::TestAppContext,
    test_name: &'static str,
    height: f32,
    wrapped: bool,
) {
    crate::app::test_support::with_prepared_offline_app(
        test_name,
        cx,
        |project| {
            let project = project.canonicalize().unwrap();
            let path = project.join("tall");
            let repo = crate::app::persistence::shared().unwrap();
            let session = TranscriptSession {
                path: &path,
                project: &project,
                harness: Backend::Pi,
                profile_id: None,
            };
            let mut document = messages("tall", 84);
            if wrapped {
                document[83]["content"][0]["text"] = "Wrapped reply text ".repeat(750).into();
            }
            assert!(
                repo.replace_transcript(session, 0, &document, &json!({}))
                    .unwrap()
            );
            let deadline = Instant::now() + SWITCH_TIMEOUT;
            loop {
                let candidate = repo
                    .load_transcript_display(session, false)
                    .unwrap()
                    .unwrap();
                if !candidate.full_document {
                    assert_eq!(candidate.coverage, crate::agents::HistoryCoverage::Suffix);
                    assert!(candidate.content.items.len() < 84);
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "compact page preparation timed out"
                );
                thread::sleep(SWITCH_POLL);
            }
        },
        |cx, app, runtime, project| {
            let project = project.canonicalize().unwrap();
            trust::apply(&project, TrustChoice::TrustProject).unwrap();
            let rows = crate::app::persistence::open()
                .unwrap()
                .cached_sessions("")
                .unwrap();
            let row = rows.iter().find(|row| row.path.ends_with("tall")).unwrap();
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    app.sessions.all = rows.clone().into();
                    app.sessions.visible = rows.clone().into();
                })
            });
            cx.simulate_resize(gpui::size(gpui::px(1600.), gpui::px(height)));
            click(cx, app, row);
            let deadline = Instant::now() + SWITCH_TIMEOUT;
            loop {
                cx.run_until_parked();
                let selected = cx.update(|window, cx| {
                    window.draw(cx).clear(cx);
                    let app = app.read(cx);
                    let selected =
                        app.lifecycle.requested_session_key.as_ref() == Some(&row.catalog_key());
                    if selected {
                        assert!(app.awaiting_selection_snapshot());
                        let view = app.views.transcript.read(cx);
                        let items = view.local_display_items(app).unwrap();
                        if wrapped {
                            assert!(
                                items.len() < 84,
                                "the visible suffix fits without older rows"
                            );
                            assert!(items.last().unwrap().text.starts_with("Wrapped reply text"));
                        } else {
                            assert!(items.len() < 84);
                            assert_eq!(items.last().unwrap().text, "tall message 83");
                        }
                    }
                    selected
                });
                assert!(cx.debug_bounds("transcript-loading").is_none());
                if selected {
                    if !wrapped {
                        let (route, count) = cx.update(|_, cx| {
                            let app = app.read(cx);
                            (
                                app.lifecycle.awaiting_switch_route.unwrap(),
                                app.views.transcript.read(cx).local_display_row_count(app),
                            )
                        });
                        assert_eq!(count, 64);
                        let last = cx.debug_bounds("transcript-row-63").unwrap();
                        let gap = cx.debug_bounds("transcript-older-loading").unwrap();
                        assert!(gap.bottom() <= last.top());
                        runtime.send_event(RuntimeEvent::SelectionRouted(route));
                        let all = messages("tall", 84);
                        let mut partial = snapshot(row, &all[20..]);
                        Arc::make_mut(&mut partial).saved_history =
                            Some(crate::runtime::SavedHistory::RefreshingPartial);
                        runtime.send_event(RuntimeEvent::Snapshot {
                            generation: 2,
                            snapshot: partial,
                        });
                        cx.update(|window, cx| {
                            app.update(cx, |app, cx| app.drain_runtime(cx));
                            window.draw(cx).clear(cx);
                        });
                        assert_eq!(cx.debug_bounds("transcript-row-63"), Some(last));
                        assert_eq!(cx.debug_bounds("transcript-older-loading"), Some(gap));
                        runtime.send_event(RuntimeEvent::Snapshot {
                            generation: 2,
                            snapshot: snapshot(row, &all),
                        });
                        for _ in 0..4 {
                            cx.update(|window, cx| {
                                app.update(cx, |app, cx| app.drain_runtime(cx));
                                window.draw(cx).clear(cx);
                            });
                            assert_eq!(cx.debug_bounds("transcript-row-83"), Some(last));
                            assert!(cx.debug_bounds("transcript-older-loading").is_none());
                        }
                    }
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "no tall-window paint before routing"
                );
                thread::sleep(SWITCH_POLL);
            }
        },
    );
}
