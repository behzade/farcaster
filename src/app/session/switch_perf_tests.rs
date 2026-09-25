//! Measures the phases a session switch runs through.
//!
//! The switch path reports timings only through `Timing`, which is silent
//! unless detailed monitoring is on and only logs phases that reach
//! `SLOW_OPERATION`. A measurement run therefore needs `DEBUG=true` (or
//! `FARCASTER_PERF_TRACE=1`) to see `switch.session_request`,
//! `switch.runtime_route`, `switch.select_document`, `switch.load_history`, and
//! `switch.session_total`.
//!
//! The test switches between real session files through a real runtime, so the
//! numbers come from the same code a click uses. It asserts only that every
//! switch applied its projected transcript. The durations belong in the
//! `PERF operation=` lines; a returning visit can be served from the
//! supervisor's resident document and then reports fewer phases.
//!
//! A document refresh repeats the load that a resident document skips, so the
//! same file is refreshed while cached and after it grows, which is the only
//! place the history cache changes what a user waits for.
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
        runtime::{RuntimeCommand, RuntimeHandle, history_cache},
    },
    projects::TrustChoice,
    sessions::SessionTarget,
};

const SWITCH_MESSAGES: usize = 2_000;
const SWITCH_POLL: Duration = Duration::from_millis(2);
const SWITCH_TIMEOUT: Duration = Duration::from_secs(15);
const SUMMARY_WINDOW: Duration = Duration::from_millis(1_400);

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
    format!(
        "{{\"type\":\"message\",\"message\":{{\"role\":\"{role}\",\"content\":\"{index} {filler}\"}}}}\n"
    )
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

fn projected_items(path: &Path, project: &Path) -> usize {
    projected(path, project).2
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
        let applied = cx.update(|window, cx| {
            app.update(cx, |app, cx| app.drain_runtime(cx));
            window.draw(cx).clear(cx);
            let snapshot = app.read(cx).snapshot.clone();
            snapshot.selected_session.as_deref() == Some(path)
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
) -> Duration {
    let items = projected_items(path, project);
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
                crate::app::runtime_host::host(),
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

            let first = switch_to(cx, app, &large, project);
            eprintln!("SWITCH_MEASURE first_visit_wall_ms={:.2}", millis(first));

            let away = switch_to(cx, app, &small, project);
            eprintln!("SWITCH_MEASURE small_visit_wall_ms={:.2}", millis(away));

            let resident = switch_to(cx, app, &large, project);
            eprintln!(
                "SWITCH_MEASURE resident_visit_wall_ms={:.2}",
                millis(resident)
            );

            eprintln!(
                "SWITCH_MEASURE warm_refresh_begin cached={}",
                history_cache::history_is_fresh(&large)
            );
            refresh_document(cx, app, &large, project);
            drive(cx, app, Duration::from_millis(250));
            eprintln!(
                "SWITCH_MEASURE warm_refresh_end cached={}",
                history_cache::history_is_fresh(&large)
            );

            let before = projected_items(&large, project);
            append_messages(&large, 2);
            assert!(
                !history_cache::history_is_fresh(&large),
                "a grown session file must invalidate the cached history"
            );
            let grown = projected_items(&large, project);
            assert!(grown > before, "the appended messages must project");
            eprintln!(
                "SWITCH_MEASURE cold_refresh_begin cached={} items={grown}",
                history_cache::history_is_fresh(&large)
            );
            refresh_document(cx, app, &large, project);
            wait_for_items(cx, app, &large, grown);
            eprintln!(
                "SWITCH_MEASURE cold_refresh_end cached={}",
                history_cache::history_is_fresh(&large)
            );

            let (read, project_cost, items) = projected(&large, project);
            eprintln!(
                "SWITCH_MEASURE uncached_read_ms={:.2} project_ms={:.2} items={items}",
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
fn hovering_a_chat_loads_the_history_a_click_would_wait_for(cx: &mut gpui::TestAppContext) {
    let test_name = concat!(
        module_path!(),
        "::hovering_a_chat_loads_the_history_a_click_would_wait_for"
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
                    "prefetch-perf-draft".into(),
                    project.to_path_buf(),
                ),
                None,
                AgentLaunchConfig::test_script(&script, vec!["quiet".into()]),
                crate::app::runtime_host::host(),
            )
        },
        |cx, app, project| {
            let canonical = project.canonicalize().expect("canonical project path");
            let project = canonical.as_path();
            trust::apply(project, TrustChoice::TrustProject).expect("trust the temp project");
            let chat = crate::sessions::normalize_session_path(&write_session_file(
                project,
                "prefetch-chat",
                SWITCH_MESSAGES,
            ));
            let bytes = fs::metadata(&chat).expect("measured session").len();

            // What a click pays with no hover: the read itself.
            let cold_started = Instant::now();
            let cold = crate::agents::load_session_history(Backend::Pi, &chat, project)
                .expect("cold read");
            let cold_read = cold_started.elapsed();
            assert!(
                !history_cache::history_is_fresh(&chat),
                "a plain read must not warm what the click reads"
            );

            // The hover does that read ahead of the click.
            let hover_started = Instant::now();
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.prefetch_session(Backend::Pi, chat.clone(), project.to_path_buf(), cx);
                });
            });
            let deadline = Instant::now() + SWITCH_TIMEOUT;
            while !history_cache::history_is_fresh(&chat) {
                assert!(
                    Instant::now() < deadline,
                    "a hover should load the history its click would wait for"
                );
                // The hover's read runs on the app's background executor.
                cx.run_until_parked();
                cx.update(|window, cx| {
                    window.draw(cx).clear(cx);
                });
                thread::sleep(SWITCH_POLL);
            }
            let hover = hover_started.elapsed();

            // What the click pays after the hover: the same load, in memory.
            let warm_started = Instant::now();
            let warm =
                history_cache::load_cached_history(Backend::Pi, &chat, project).expect("warm read");
            let hit = warm_started.elapsed();
            assert_eq!(warm.messages.len(), cold.messages.len());

            let items = projected_items(&chat, project);
            let wall = switch_to(cx, app, &chat, project);
            assert!(wall > Duration::ZERO);
            eprintln!("PREFETCH_MEASURE bytes={bytes} items={items}");
            eprintln!(
                "PREFETCH_MEASURE cold_read_ms={:.2} hover_to_fresh_ms={:.2} warm_hit_ms={:.2} selection_wall_ms={:.2}",
                millis(cold_read),
                millis(hover),
                millis(hit),
                millis(wall),
            );
        },
    );
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}
