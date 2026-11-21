#![allow(clippy::print_stderr)]

use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use gpui::{Entity, VisualTestContext};

use crate::{
    agents::{AgentLaunchConfig, Backend},
    app::{
        FarcasterApp,
        project::trust,
        runtime::{RuntimeCommand, RuntimeHandle},
    },
    projects::TrustChoice,
    sessions::SessionTarget,
};

const SWITCH_MESSAGES: usize = 2_000;
const SWITCH_POLL: Duration = Duration::from_millis(2);
const SWITCH_TIMEOUT: Duration = Duration::from_secs(15);
const SUMMARY_WINDOW: Duration = Duration::from_millis(1_400);

#[gpui::test]
fn session_actions_draw_before_runtime_acknowledgment(cx: &mut gpui::TestAppContext) {
    use crate::sessions::SessionSummary;
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::session_actions_draw_before_runtime_acknowledgment"
        ),
        cx,
        |cx, app, _, project| {
            let rows: Vec<_> = (1..=2_716)
                .map(|id| {
                    let title = format!("Chat {id}");
                    SessionSummary::from_cached(
                        id.to_string(),
                        project.join(id.to_string()),
                        project.into(),
                        title,
                        String::new(),
                        String::new(),
                        None,
                        std::time::UNIX_EPOCH,
                        1,
                        Default::default(),
                        id > 120,
                        false,
                        String::new(),
                    )
                    .with_app_session_id(id)
                })
                .collect();
            let target = rows[1].catalog_key();
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    std::sync::Arc::make_mut(&mut app.snapshot).selected_session =
                        Some(rows[0].path.clone());
                    app.sessions.visible = rows.clone().into();
                    app.sessions.all = rows.into();
                    app.sessions.selected_draft = None;
                    app.sessions.archived_expanded = true;
                    app.notify_session_rail(cx);
                })
            });
            cx.simulate_resize(gpui::size(gpui::px(1600.), gpui::px(900.)));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            for action in ["switch", "archive", "new"] {
                let start = Instant::now();
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| match action {
                        "switch" => app.select_rail_session(target.clone(), window, cx),
                        "archive" => app.request_rail_archive(target.clone(), true, window, cx),
                        "new" => app.new_session(project.into(), window, cx),
                        _ => unreachable!(),
                    });
                });
                let handler = start.elapsed();
                cx.update(|window, cx| window.draw(cx).clear(cx));
                let drawn = start.elapsed();
                cx.update(|_, cx| {
                    let app = app.read(cx);
                    match action {
                        "switch" => {
                            assert_eq!(app.selected_rail_root().unwrap().catalog_key(), target)
                        }
                        "archive" => {
                            assert!(app.sessions.visible.by_key(&target).unwrap().archived)
                        }
                        "new" => assert!(app.sessions.selected_draft.is_some()),
                        _ => unreachable!(),
                    }
                });
                eprintln!(
                    "sidebar {action} with 2716 sessions: handler {handler:?}, through test-window draw {drawn:?}"
                );
            }
        },
    );
}

fn write_session_file(directory: &Path, name: &str, messages: usize) -> PathBuf {
    let mut body = format!("{{\"type\":\"session\",\"id\":\"{name}\",\"cwd\":\".\"}}\n");
    let filler = "a transcript line that a switch has to read and project ".repeat(8);
    for index in 0..messages {
        body.push_str(&message_line(index, &filler));
    }
    let path = directory.join(format!("{name}.jsonl"));
    fs::write(&path, body).expect("write session file");
    path
}

fn message_line(index: usize, filler: &str) -> String {
    let role = if index.is_multiple_of(2) {
        "user"
    } else {
        "assistant"
    };
    let message = serde_json::json!({
        "type": "message",
        "message": {
            "role": role,
            "content": [{"type": "text", "text": format!("{index} {filler}")}],
        },
    });
    format!("{message}\n")
}

fn append_messages(path: &Path, count: usize) {
    let mut body = fs::read_to_string(path).expect("read session file");
    let filler = "a transcript line that arrived while the session was open ".repeat(8);
    for index in 0..count {
        body.push_str(&message_line(SWITCH_MESSAGES + index, &filler));
    }
    fs::write(path, body).expect("append session messages");
}

fn projected(path: &Path, project: &Path) -> (Duration, Duration, usize) {
    let reading = Instant::now();
    let history =
        crate::agents::load_session_history(Backend::Pi, path, project).expect("load history");
    let read = reading.elapsed();
    let projecting = Instant::now();
    let mut conversation = crate::conversation::ConversationState::default();
    conversation.replace_history(&history.messages);
    let project_cost = projecting.elapsed();
    (read, project_cost, conversation.items.len())
}

fn drive(cx: &mut VisualTestContext, app: &Entity<FarcasterApp>, duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.drain_runtime(cx));
            window.draw(cx).clear(cx);
        });
        thread::sleep(SWITCH_POLL);
    }
}

fn wait_for_items(
    cx: &mut VisualTestContext,
    app: &Entity<FarcasterApp>,
    path: &Path,
    items: usize,
) {
    let deadline = Instant::now() + SWITCH_TIMEOUT;
    loop {
        cx.run_until_parked();
        let applied = cx.update(|window, cx| {
            app.update(cx, |app, cx| app.drain_runtime(cx));
            window.draw(cx).clear(cx);
            let snapshot = app.read(cx).snapshot.clone();
            snapshot.selected_session.as_deref() == Some(path)
                && snapshot.status == "Ready"
                && snapshot.conversation.items.len() == items
        });
        if applied {
            return;
        }
        let shown = cx.update(|_, cx| {
            let app = app.read(cx);
            format!(
                "selected={:?} status={} items={} error={:?}",
                app.snapshot.selected_session,
                app.snapshot.status,
                app.snapshot.conversation.items.len(),
                app.sessions.error,
            )
        });
        assert!(
            Instant::now() < deadline,
            "{} never showed its {items} transcript items; {shown}",
            path.display(),
        );
        thread::sleep(SWITCH_POLL);
    }
}

fn switch_to(
    cx: &mut VisualTestContext,
    app: &Entity<FarcasterApp>,
    path: &Path,
    project: &Path,
    items: usize,
) -> Duration {
    let started = Instant::now();
    cx.update(|window, cx| {
        app.update(cx, |app, _| {
            app.runtime.session_targets.insert(
                path.to_path_buf(),
                SessionTarget {
                    harness: Backend::Pi,
                    id: path
                        .file_stem()
                        .expect("session file stem")
                        .to_string_lossy()
                        .into_owned(),
                    path: path.to_path_buf(),
                },
            );
        });
        app.update(cx, |app, cx| {
            app.select_session(path.to_path_buf(), project.to_path_buf(), window, cx);
        });
    });
    wait_for_items(cx, app, path, items);
    started.elapsed()
}

fn refresh_document(
    cx: &mut VisualTestContext,
    app: &Entity<FarcasterApp>,
    path: &Path,
    project: &Path,
) {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.send(
                RuntimeCommand::RefreshSessionDocument {
                    path: path.to_path_buf(),
                    project: project.to_path_buf(),
                    harness: Some(Backend::Pi),
                },
                cx,
            );
        });
    });
}

#[gpui::test]
fn switching_between_sessions_runs_every_measured_phase(cx: &mut gpui::TestAppContext) {
    let executor = cx.background_executor.clone();
    let test_name = concat!(
        module_path!(),
        "::switching_between_sessions_runs_every_measured_phase"
    );
    crate::app::test_support::with_runtime_app(
        test_name,
        cx,
        |project| {
            let script = project.join("fake-pi.sh");
            fs::write(&script, include_str!("../../../tests/fixtures/fake-pi.sh"))
                .expect("write fake pi");
            RuntimeHandle::spawn(
                project.to_path_buf(),
                crate::sessions::DraftSession::with_id(
                    Some(Backend::Pi),
                    "switch-perf-draft".into(),
                    project.to_path_buf(),
                ),
                None,
                AgentLaunchConfig::test_script(&script, vec!["quiet".into()]),
                crate::app::runtime_host::host(executor.clone()),
            )
        },
        |cx, app, project| {
            let canonical = project.canonicalize().expect("canonical project path");
            let project = canonical.as_path();
            trust::apply(project, TrustChoice::TrustProject).expect("trust the temp project");
            let large = crate::sessions::normalize_session_path(&write_session_file(
                project,
                "switch-large",
                SWITCH_MESSAGES,
            ));
            let small = crate::sessions::normalize_session_path(&write_session_file(
                project,
                "switch-small",
                4,
            ));
            let large_bytes = fs::metadata(&large).expect("large session").len();
            eprintln!(
                "SWITCH_MEASURE large={} bytes={} messages={}",
                large.display(),
                large_bytes,
                SWITCH_MESSAGES
            );

            let first = switch_to(cx, app, &large, project, SWITCH_MESSAGES);
            eprintln!("SWITCH_MEASURE first_visit_wall_ms={:.2}", millis(first));

            let away = switch_to(cx, app, &small, project, 4);
            eprintln!("SWITCH_MEASURE small_visit_wall_ms={:.2}", millis(away));

            let resident = switch_to(cx, app, &large, project, SWITCH_MESSAGES);
            eprintln!(
                "SWITCH_MEASURE resident_visit_wall_ms={:.2}",
                millis(resident)
            );

            eprintln!("SWITCH_MEASURE warm_refresh_begin");
            refresh_document(cx, app, &large, project);
            drive(cx, app, Duration::from_millis(250));
            eprintln!("SWITCH_MEASURE warm_refresh_end");

            append_messages(&large, 2);
            let grown = SWITCH_MESSAGES + 2;
            eprintln!("SWITCH_MEASURE cold_refresh_begin items={grown}");
            refresh_document(cx, app, &large, project);
            wait_for_items(cx, app, &large, grown);
            eprintln!("SWITCH_MEASURE cold_refresh_end");

            let (read, project_cost, items) = projected(&large, project);
            eprintln!(
                "SWITCH_MEASURE read_ms={:.2} project_ms={:.2} items={items}",
                millis(read),
                millis(project_cost)
            );

            eprintln!("SWITCH_MEASURE summary_window_begin");
            drive(cx, app, SUMMARY_WINDOW);
            eprintln!("SWITCH_MEASURE summary_window_end");
        },
    );
}

#[gpui::test]
fn hover_prefetch_and_cold_selection_load_the_same_transcript(cx: &mut gpui::TestAppContext) {
    let executor = cx.background_executor.clone();
    crate::app::test_support::with_runtime_app(
        concat!(
            module_path!(),
            "::hover_prefetch_and_cold_selection_load_the_same_transcript"
        ),
        cx,
        |project| {
            let script = project.join("fake-pi.sh");
            fs::write(&script, include_str!("../../../tests/fixtures/fake-pi.sh")).unwrap();
            RuntimeHandle::spawn(
                project.into(),
                crate::sessions::DraftSession::with_id(
                    Some(Backend::Pi),
                    "prefetch-perf-draft".into(),
                    project.into(),
                ),
                None,
                AgentLaunchConfig::test_script(&script, vec!["quiet".into()]),
                crate::app::runtime_host::host(executor.clone()),
            )
        },
        |cx, app, project| {
            let project = project.canonicalize().unwrap();
            trust::apply(&project, TrustChoice::TrustProject).unwrap();
            let warmup = write_session_file(&project, "prefetch-warmup", 4);
            switch_to(cx, app, &warmup, &project, 4);
            cx.run_until_parked();
            for index in 0..3 {
                let cold = write_session_file(&project, &format!("cold-{index}"), SWITCH_MESSAGES);
                let hovered =
                    write_session_file(&project, &format!("hovered-{index}"), SWITCH_MESSAGES);
                let bytes = fs::metadata(&hovered).unwrap().len();
                let mut cold_wall = Duration::ZERO;
                let mut hover_work = Duration::ZERO;
                let mut prefetched_wall = Duration::ZERO;
                for prefetch in [index % 2 == 0, index % 2 != 0] {
                    if prefetch {
                        let started = Instant::now();
                        cx.update(|_, cx| {
                            app.update(cx, |app, cx| {
                                app.prefetch_session(
                                    Backend::Pi,
                                    hovered.clone(),
                                    project.clone(),
                                    cx,
                                );
                            })
                        });
                        cx.run_until_parked();
                        hover_work = started.elapsed();
                        prefetched_wall = switch_to(cx, app, &hovered, &project, SWITCH_MESSAGES);
                    } else {
                        cold_wall = switch_to(cx, app, &cold, &project, SWITCH_MESSAGES);
                    }
                }
                let cold_history =
                    crate::agents::load_session_history(Backend::Pi, &cold, &project).unwrap();
                let hovered_history =
                    crate::agents::load_session_history(Backend::Pi, &hovered, &project).unwrap();
                assert_eq!(cold_history.messages, hovered_history.messages);
                eprintln!(
                    "PREFETCH_MEASURE sample={index} bytes={bytes} items={SWITCH_MESSAGES} cold_selection_ms={:.2} hover_executor_ms={:.2} prefetched_selection_ms={:.2}",
                    millis(cold_wall),
                    millis(hover_work),
                    millis(prefetched_wall)
                );
            }
        },
    );
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[gpui::test]
fn hovering_a_cursor_session_does_not_start_its_profile(cx: &mut gpui::TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::hovering_a_cursor_session_does_not_start_its_profile"
        ),
        cx,
        |cx, app, _, project| {
            let target = project.join("other-project");
            fs::create_dir(&target).unwrap();
            crate::agents::set_test_project_environment(
                &target,
                vec![("CURSOR_API_KEY".into(), "fixture".into())],
            );
            let executable = project.join("cursor-history-fixture");
            fs::write(
                &executable,
                "#!/bin/sh\n: > history-bridge-started\nexit 1\n",
            )
            .unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
            let profile = crate::agents::HarnessProfile {
                id: "11111111-1111-4111-8111-111111111111".into(),
                name: "Cursor fixture".into(),
                backend: Backend::Cursor,
                executable,
                data_directory: Some(project.to_owned()),
            };
            let path = project
                .join("profiles")
                .join(&profile.id)
                .join(Backend::Cursor.as_str())
                .join("thread");
            let config = cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    let config = AgentLaunchConfig {
                        profiles: app.settings.harness_profiles.clone(),
                        profile_id: Some(profile.id.clone()),
                        ..Default::default()
                    };
                    config.profiles.replace(vec![profile]).unwrap();
                    app.prefetch_session(Backend::Cursor, path.clone(), target.clone(), cx);
                    config
                })
            });
            cx.run_until_parked();
            let marker = target.join("history-bridge-started");
            assert!(!marker.exists());
            // The same locator and profile start the fixture when explicitly loading history.
            let _ = crate::agents::load_session_history_for_profile(
                &config,
                Backend::Cursor,
                &path,
                &target,
            );
            assert!(marker.is_file());
        },
    );
}

#[cfg(unix)]
#[gpui::test]
fn cold_codex_selection_paints_rows_at_their_final_positions(cx: &mut gpui::TestAppContext) {
    cold_codex_frame_check(
        cx,
        concat!(
            module_path!(),
            "::cold_codex_selection_paints_rows_at_their_final_positions"
        ),
        30,
    );
}

#[cfg(unix)]
#[gpui::test]
fn cold_codex_short_chat_paints_rows_at_their_final_positions(cx: &mut gpui::TestAppContext) {
    cold_codex_frame_check(
        cx,
        concat!(
            module_path!(),
            "::cold_codex_short_chat_paints_rows_at_their_final_positions"
        ),
        1,
    );
}

#[cfg(unix)]
fn cold_codex_frame_check(cx: &mut gpui::TestAppContext, name: &str, turn_count: usize) {
    use std::os::unix::fs::PermissionsExt as _;

    const PROFILE: &str = "11111111-1111-4111-8111-111111111111";
    let executor = cx.background_executor.clone();
    let complete = turn_count <= 8;
    crate::app::test_support::with_runtime_app(
        name,
        cx,
        |project| {
            // The fixture uses a fixed executable; keep the user's login shell out of the measurement.
            crate::agents::set_test_project_environment(
                project,
                vec![("PATH".into(), "/usr/bin:/bin".into())],
            );
            let executable = project.join("codex-fixture");
            fs::write(
                &executable,
                include_str!("../../../tests/fixtures/cold-codex-history.sh"),
            )
            .unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
            let turns: Vec<_> = (0..turn_count).map(|n| serde_json::json!({"id":format!("turn-{n}"),"itemsView":"full","status":"completed","items":[
                {"type":"userMessage","id":format!("q-{n}"),"content":[{"type":"text","text":format!("Question {n}")}]},
                {"type":"agentMessage","id":format!("a-{n}"),"text":format!("**Answer {n}**\n\n{}", "Wrapped words in this answer. ".repeat(20))}
            ]})).collect();
            let page: Vec<_> = turns.iter().rev().take(8).collect();
            fs::write(
                project.join("recent-turns.json"),
                serde_json::json!({"data":page,"nextCursor":(!complete).then_some("older")})
                    .to_string(),
            )
            .unwrap();
            fs::write(
                project.join("full-thread.json"),
                serde_json::json!({"thread":{"turns":turns}}).to_string(),
            )
            .unwrap();
            let rollout = project.join("rollout.jsonl");
            fs::write(&rollout, "").unwrap();
            let history_db =
                rusqlite::Connection::open(project.join("thread_history_1.sqlite")).unwrap();
            history_db.execute_batch("CREATE TABLE thread_turns(thread_id TEXT);
                INSERT INTO thread_turns VALUES ('thread');
                CREATE TABLE thread_history_projection_state(thread_id TEXT PRIMARY KEY, next_rollout_byte_offset INTEGER, next_rollout_ordinal INTEGER);
                INSERT INTO thread_history_projection_state VALUES ('thread',0,30);").unwrap();
            let db = rusqlite::Connection::open(project.join("state_5.sqlite")).unwrap();
            db.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, rollout_path TEXT, history_mode TEXT, model_provider TEXT, model TEXT, reasoning_effort TEXT)").unwrap();
            db.execute(
                "INSERT INTO threads VALUES ('thread', ?1, 'paginated', 'openai', 'fixture', NULL)",
                [rollout.to_str().unwrap()],
            )
            .unwrap();
            let config = AgentLaunchConfig::default();
            config
                .profiles
                .replace(vec![crate::agents::HarnessProfile {
                    id: PROFILE.into(),
                    name: "Fixture".into(),
                    backend: Backend::Codex,
                    executable,
                    data_directory: Some(project.to_owned()),
                }])
                .unwrap();
            RuntimeHandle::spawn(
                project.to_owned(),
                crate::sessions::DraftSession::with_id(
                    None,
                    "cold-history-test".into(),
                    project.into(),
                ),
                None,
                config,
                crate::app::runtime_host::host(executor.clone()),
            )
        },
        |cx, app, project| {
            let project = project.canonicalize().unwrap();
            trust::apply(&project, TrustChoice::TrustProject).unwrap();
            let path = project
                .join("profiles")
                .join(PROFILE)
                .join(Backend::Codex.as_str())
                .join("thread");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let row = crate::sessions::SessionSummary::from_cached_for_harness(
                "thread".into(),
                Backend::Codex,
                path.clone(),
                project.clone(),
                "Saved chat".into(),
                "Original question".into(),
                String::new(),
                None,
                std::time::UNIX_EPOCH,
                2,
                Default::default(),
                false,
                false,
                String::new(),
            )
            .with_app_session_id(17);
            let key = row.catalog_key();
            let mut source = row.clone().with_app_session_id(16);
            source.id = "source".into();
            source.path.set_file_name("source");
            let source_path = source.path.clone();
            let source_key = source.catalog_key();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.all = vec![source.clone(), row.clone()].into();
                    app.sessions.visible = vec![source, row.clone()].into();
                    app.select_rail_session(source_key, window, cx);
                })
            });
            wait_for_items(cx, app, &source_path, 1);
            assert!(cx.debug_bounds("transcript-row-0").is_some());
            let started = Instant::now();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    // The fixture uses a real supervisor, whose startup catalog refresh
                    // can run while we wait for the source document.
                    app.sessions.all.upsert_by_recency(row.clone());
                    app.sessions.visible.upsert_by_recency(row);
                    app.select_rail_session(key, window, cx);
                    assert!(app.lifecycle.pending_session_switch.is_some());
                });
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(cx.debug_bounds("transcript-loading").is_some());
            assert!(cx.debug_bounds("transcript-row-0").is_none());
            let deadline = started + SWITCH_TIMEOUT;
            let short = ["transcript-row-0", "transcript-row-1"];
            let partial = [
                "transcript-row-10",
                "transcript-row-11",
                "transcript-row-12",
                "transcript-row-13",
                "transcript-row-14",
                "transcript-row-15",
            ];
            let full = [
                "transcript-row-54",
                "transcript-row-55",
                "transcript-row-56",
                "transcript-row-57",
                "transcript-row-58",
                "transcript-row-59",
            ];
            let partial_rows = if complete {
                short.as_slice()
            } else {
                partial.as_slice()
            };
            let full_rows = if complete {
                short.as_slice()
            } else {
                full.as_slice()
            };
            let mut frames = Vec::new();
            loop {
                cx.run_until_parked();
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                    window.draw(cx).clear(cx);
                });
                if !complete {
                    assert!(cx.debug_bounds("transcript-row-0").is_none());
                }
                let rows: Vec<_> = partial_rows
                    .iter()
                    .map(|key| cx.debug_bounds(key))
                    .collect();
                if rows.iter().any(Option::is_some) {
                    cx.update(|_, cx| {
                        let snapshot = &app.read(cx).snapshot;
                        assert!(!snapshot.document_ready);
                        let items = &snapshot.history_tail.as_ref().unwrap().content.items;
                        assert_eq!(items.len(), turn_count.min(8) * 2);
                        assert_eq!(
                            items[items.len() - 2].text,
                            format!("Question {}", turn_count - 1)
                        );
                        assert!(
                            items[items.len() - 1]
                                .text
                                .contains(&format!("Answer {}", turn_count - 1))
                        );
                    });
                    frames.push(rows);
                }
                if !frames.is_empty() && project.join("reading-history").exists() {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "no readable messages while full history was blocked"
                );
                thread::sleep(SWITCH_POLL);
            }
            let first_readable = started.elapsed();
            assert!(project.join("requested-preview").exists());
            assert!(cx.debug_bounds("transcript-loading").is_none());
            fs::write(project.join("release-full-read"), "").unwrap();
            let mut ready_frames = 0;
            while ready_frames < 8 {
                assert!(Instant::now() < deadline, "cold history did not settle");
                cx.run_until_parked();
                let ready = cx.update(|window, cx| {
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                    window.draw(cx).clear(cx);
                    let snapshot = &app.read(cx).snapshot;
                    snapshot.selected_session.as_ref() == Some(&path) && snapshot.document_ready
                });
                assert!(
                    complete || cx.debug_bounds("transcript-row-0").is_none(),
                    "painted the top of a long history"
                );
                let selectors = if ready { full_rows } else { partial_rows };
                frames.push(selectors.iter().map(|key| cx.debug_bounds(key)).collect());
                ready_frames += usize::from(ready);
                thread::sleep(SWITCH_POLL);
            }
            let settled = frames.last().expect("visible history");
            assert!(
                settled.iter().flatten().count() >= 2 && settled.last().unwrap().is_some(),
                "expected visible tail rows, got {settled:?}"
            );
            assert!(cx.debug_bounds("transcript-loading").is_none());
            for frame in &frames {
                assert_eq!(frame, settled, "a transcript row shifted during cold open");
            }
            eprintln!(
                "Cold open: first readable rows in {first_readable:?} with full history blocked; {} frames, {} visible tail rows, zero pixel displacement",
                frames.len(),
                settled.iter().flatten().count()
            );
        },
    );
}

/// Run against an existing paginated chat without resuming it or sending a prompt.
#[gpui::test]
#[ignore = "requires FARCASTER_TEST_CODEX_THREAD, FARCASTER_TEST_CODEX_HOME and FARCASTER_TEST_CODEX_PROGRAM"]
fn cold_saved_codex_chat_draws_recent_rows_before_full_read(cx: &mut gpui::TestAppContext) {
    const PROFILE: &str = "11111111-1111-4111-8111-111111111111";
    let thread = std::env::var("FARCASTER_TEST_CODEX_THREAD").unwrap();
    let full_only = std::env::var_os("FARCASTER_TEST_FULL_HISTORY_ONLY").is_some();
    let executor = cx.background_executor.clone();
    crate::app::test_support::with_runtime_app(
        concat!(
            module_path!(),
            "::cold_saved_codex_chat_draws_recent_rows_before_full_read"
        ),
        cx,
        |project| {
            let mut environment = vec![("PATH".into(), "/usr/bin:/bin".into())];
            if full_only {
                // A SQLite-home override uses the full-read-only path on the same data.
                environment.push((
                    "CODEX_SQLITE_HOME".into(),
                    std::env::var_os("FARCASTER_TEST_CODEX_HOME").unwrap(),
                ));
            }
            crate::agents::set_test_project_environment(project, environment);
            let config = AgentLaunchConfig::default();
            config
                .profiles
                .replace(vec![crate::agents::HarnessProfile {
                    id: PROFILE.into(),
                    name: "Local history".into(),
                    backend: Backend::Codex,
                    executable: std::env::var_os("FARCASTER_TEST_CODEX_PROGRAM")
                        .unwrap()
                        .into(),
                    data_directory: Some(
                        std::env::var_os("FARCASTER_TEST_CODEX_HOME")
                            .unwrap()
                            .into(),
                    ),
                }])
                .unwrap();
            RuntimeHandle::spawn(
                project.into(),
                crate::sessions::DraftSession::with_id(None, "start".into(), project.into()),
                None,
                config,
                crate::app::runtime_host::host(executor.clone()),
            )
        },
        |cx, app, project| {
            let project = project.canonicalize().unwrap();
            trust::apply(&project, TrustChoice::TrustProject).unwrap();
            let path = project
                .join("profiles")
                .join(PROFILE)
                .join(Backend::Codex.as_str())
                .join(&thread);
            let row = crate::sessions::SessionSummary::from_cached_for_harness(
                thread.clone(),
                Backend::Codex,
                path.clone(),
                project,
                "Saved chat".into(),
                String::new(),
                String::new(),
                None,
                std::time::UNIX_EPOCH,
                1,
                Default::default(),
                false,
                false,
                String::new(),
            )
            .with_app_session_id(17);
            let started = Instant::now();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let key = row.catalog_key();
                    app.sessions.all = vec![row.clone()].into();
                    app.sessions.visible = vec![row].into();
                    app.select_rail_session(key, window, cx);
                })
            });
            let mut first = None;
            let mut page_rows = 0;
            let mut selectors = std::collections::HashMap::new();
            loop {
                assert!(
                    started.elapsed() < SWITCH_TIMEOUT,
                    "live history did not arrive"
                );
                cx.run_until_parked();
                let (ready, count) = cx.update(|window, cx| {
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                    window.draw(cx).clear(cx);
                    let app = app.read(cx);
                    let snapshot = &app.snapshot;
                    if snapshot.selected_session.as_ref() != Some(&path) {
                        return (false, 0);
                    }
                    if snapshot.document_ready {
                        return (true, app.views.transcript.read(cx).rows.len());
                    }
                    (
                        false,
                        app.views.transcript.read(cx).history_tail_row_count(),
                    )
                });
                if !ready {
                    page_rows = page_rows.max(count);
                }
                if count > 0 {
                    let bounds: Vec<_> = (0..count.min(24))
                        .map(|offset| {
                            let index = count - 1 - offset;
                            let key = selectors.entry(index).or_insert_with(|| {
                                Box::leak(format!("transcript-row-{index}").into_boxed_str())
                                    as &'static str
                            });
                            cx.debug_bounds(key)
                        })
                        .take_while(Option::is_some)
                        .flatten()
                        .collect();
                    if !bounds.is_empty() {
                        let (when, initial) = first.get_or_insert_with(|| {
                            assert_eq!(ready, full_only, "unexpected first paint ({page_rows} provisional rows, {count} final rows)");
                            (started.elapsed(), bounds.clone())
                        });
                        assert_eq!(initial, &bounds, "visible tail shifted on actual history");
                        if ready {
                            eprintln!(
                                "Real cold open: first readable {when:?}, full {:?}, {} visible rows at identical bounds",
                                started.elapsed(),
                                bounds.len()
                            );
                            break;
                        }
                    } else {
                        assert!(!ready, "full transcript has no visible tail");
                    }
                }
                thread::sleep(SWITCH_POLL);
            }
        },
    );
}

#[gpui::test]
fn resuming_a_draft_keeps_late_editor_text_out_of_its_input(cx: &mut gpui::TestAppContext) {
    use crate::{protocol::ExtensionUiRequest, runtime::RuntimeEvent, sessions::DraftSession};

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::resuming_a_draft_keeps_late_editor_text_out_of_its_input"
        ),
        cx,
        |cx, app, runtime, project| {
            trust::apply(project, TrustChoice::TrustProject).unwrap();
            let draft =
                DraftSession::new(Some(Backend::Codex), "resume".into(), 42, project.into(), 0);
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let id = draft.id.clone();
                    app.sessions.drafts.push(draft);
                    app.resume_draft(id, project.into(), window, cx);
                    app.composer
                        .input
                        .update(cx, |input, cx| input.set_value("draft text", window, cx));
                    runtime.send_event(RuntimeEvent::ExtensionUi {
                        generation: app.runtime_generation,
                        request: ExtensionUiRequest::SetEditorText {
                            id: "previous".into(),
                            text: "previous actor's text".into(),
                        },
                        system_notification_target: None,
                    });
                    app.drain_runtime(cx);
                    assert!(app.lifecycle.pending_session_switch.is_none());
                });
                window.draw(cx).clear(cx);
            });
            cx.run_until_parked();
            let request = std::iter::from_fn(|| runtime.try_recv_command())
                .find_map(|command| match command {
                    RuntimeCommand::ResumeDraft { request, .. } => request,
                    _ => None,
                })
                .expect("resume dispatched");
            runtime.send_event(RuntimeEvent::SelectionRouted(request));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
            });
            cx.run_until_parked();
            cx.update(|_, cx| {
                let app = app.read(cx);
                assert_eq!(app.composer.input.read(cx).value().as_ref(), "draft text");
                assert!(app.awaiting_selection_snapshot());
            });
            let path = project.join("materialized");
            let row = crate::sessions::SessionSummary::from_cached_for_harness(
                "native".into(),
                Backend::Codex,
                path.clone(),
                project.into(),
                "Resumed draft".into(),
                String::new(),
                String::new(),
                None,
                std::time::UNIX_EPOCH,
                1,
                Default::default(),
                false,
                false,
                String::new(),
            )
            .with_app_session_id(42);
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.sessions
                        .submitted_drafts
                        .insert("resume".into(), Some(path.clone()));
                    runtime.send_event(RuntimeEvent::SessionUpdated(row));
                    app.drain_runtime(cx);
                    assert!(app.sessions.selected_draft.is_none());
                    assert!(app.awaiting_selection_snapshot());
                });
                window.draw(cx).clear(cx);
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    assert_eq!(app.composer.input.read(cx).value().as_ref(), "draft text");
                    runtime.send_event(RuntimeEvent::Snapshot {
                        generation: app.runtime_generation + 1,
                        snapshot: std::sync::Arc::new(crate::runtime::RuntimeSnapshot {
                            harness: Some(Backend::Codex),
                            selected_session: Some(path),
                            project: project.into(),
                            document_ready: true,
                            ..Default::default()
                        }),
                    });
                    app.drain_runtime(cx);
                    assert!(!app.awaiting_selection_snapshot());
                });
                window.draw(cx).clear(cx);
            });
            cx.run_until_parked();
            cx.update(|_, cx| {
                assert_eq!(
                    app.read(cx).composer.input.read(cx).value().as_ref(),
                    "draft text"
                );
            });
        },
    );
}

#[gpui::test]
fn late_editor_text_cannot_replace_input_for_the_session_being_opened(
    cx: &mut gpui::TestAppContext,
) {
    use crate::{
        protocol::ExtensionUiRequest,
        runtime::RuntimeEvent,
        sessions::{ComposerSnapshot, session_target},
    };

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::late_editor_text_cannot_replace_input_for_the_session_being_opened"
        ),
        cx,
        |cx, app, runtime, project| {
            let requested = project.join("requested");
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.switch_composer_target(session_target(&requested), window, cx);
                    app.apply_composer_snapshot(
                        ComposerSnapshot::new("new input".into(), 9, 9..9),
                        window,
                        cx,
                    );
                    app.lifecycle.pending_session_switch = Some((
                        requested.clone(),
                        crate::app::infrastructure::performance::Timing::new("switch.test"),
                    ));
                    runtime.send_event(RuntimeEvent::ExtensionUi {
                        generation: app.runtime_generation,
                        request: ExtensionUiRequest::SetEditorText {
                            id: "old".into(),
                            text: "old extension".into(),
                        },
                        system_notification_target: None,
                    });
                    app.drain_runtime(cx);
                })
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.run_until_parked();
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    assert_eq!(app.composer.input.read(cx).value().as_ref(), "new input");
                    assert!(app.extensions.pending_editor_text.is_some());
                    runtime.send_event(RuntimeEvent::Snapshot {
                        generation: app.runtime_generation + 1,
                        snapshot: std::sync::Arc::new(crate::runtime::RuntimeSnapshot {
                            selected_session: Some(requested.clone()),
                            project: project.into(),
                            document_ready: true,
                            status: "Ready".into(),
                            ..Default::default()
                        }),
                    });
                    app.drain_runtime(cx);
                })
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.run_until_parked();
            cx.update(|_, cx| {
                let app = app.read(cx);
                assert!(app.extensions.pending_editor_text.is_none());
                assert_eq!(app.composer.input.read(cx).value().as_ref(), "new input");
            });
        },
    );
}
