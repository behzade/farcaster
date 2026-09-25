//! Measures what moving the folder and registry writes off the click path
//! bought.
//!
//! `save_session_state` hands the write to a background task instead of
//! performing it while the user waits. This case times both ways of adding a
//! project: the write as the click used to perform it, and the handoff the
//! click performs now, then confirms every handed-off write still lands.
//!
//! Both halves repeat, because the first write into a fresh store also creates
//! the schema and would otherwise make the inline write look slower than it is.
#![allow(clippy::print_stderr)]

use super::*;

use gpui::{Entity, VisualTestContext};
use std::time::{Duration, Instant};

const WRITES: usize = 5;
const SETTLE_TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(2);

#[gpui::test]
fn folder_and_registry_writes_leave_the_click_path(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::folder_and_registry_writes_leave_the_click_path"
        ),
        cx,
        |cx, app, _, project| {
            // The store canonicalizes on the way out, so compare like for like.
            let project = project.canonicalize().expect("canonical project path");
            let project = project.as_path();
            let added = (0..WRITES * 2)
                .map(|index| {
                    let path = project.join(format!("added-{index}"));
                    std::fs::create_dir_all(&path).expect("create an added project");
                    path
                })
                .collect::<Vec<_>>();

            // The state the background writer leaves behind once it is idle.
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.project.registered.push(added[0].clone());
                    app.save_session_state(cx);
                });
            });
            settle(cx, app);

            let inline = added[..WRITES]
                .iter()
                .map(|path| {
                    let started = Instant::now();
                    cx.update(|_, cx| {
                        app.update(cx, |app, _| {
                            app.project.registered.push(path.clone());
                            let registry = projects::ProjectList {
                                projects: app.project.registered.clone(),
                                excluded_projects: app.project.excluded.clone(),
                            };
                            let folders = app.sessions.folders.clone();
                            project_registry::save(&registry).expect("save registry");
                            crate::app::persistence::open()
                                .and_then(|store| store.save_session_folders(&folders))
                                .expect("save session folders");
                        });
                    });
                    millis(started.elapsed())
                })
                .collect::<Vec<_>>();

            let handoff = added[WRITES..]
                .iter()
                .map(|path| {
                    let started = Instant::now();
                    cx.update(|_, cx| {
                        app.update(cx, |app, cx| {
                            app.project.registered.push(path.clone());
                            app.save_session_state(cx);
                        });
                    });
                    let waited = millis(started.elapsed());
                    settle(cx, app);
                    waited
                })
                .collect::<Vec<_>>();

            let (persisted, error) = cx.update(|_, cx| {
                let app = app.read(cx);
                (
                    project_registry::load().expect("load registry"),
                    app.sessions.error.clone(),
                )
            });
            assert_eq!(error, None, "the handed-off writes must not fail");
            for path in &added {
                assert!(
                    persisted.projects.contains(path),
                    "the handed-off registry write must reach the store: {}",
                    path.display(),
                );
            }

            eprintln!("WRITER_MEASURE writes={}", WRITES);
            eprintln!(
                "WRITER_MEASURE inline_ms={} median={:.2}",
                format(&inline),
                median(&inline),
            );
            eprintln!(
                "WRITER_MEASURE handoff_ms={} median={:.2}",
                format(&handoff),
                median(&handoff),
            );
            eprintln!(
                "WRITER_MEASURE moved_off_click_ms={:.2}",
                median(&inline) - median(&handoff),
            );
        },
    );
}

/// Waits until the handed-off write has landed, so the next one is measured on
/// its own instead of coalescing into it.
fn settle(cx: &mut VisualTestContext, app: &Entity<FarcasterApp>) {
    let deadline = Instant::now() + SETTLE_TIMEOUT;
    while cx.update(|_, cx| app.read(cx).sessions.save_in_flight) {
        // The handed-off write runs on the app's background executor.
        cx.run_until_parked();
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.drain_runtime(cx));
            window.draw(cx).clear(cx);
        });
        assert!(
            Instant::now() < deadline,
            "the handed-off state write should settle"
        );
        std::thread::sleep(POLL);
    }
}

fn format(samples: &[f64]) -> String {
    samples
        .iter()
        .map(|sample| format!("{sample:.2}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn median(samples: &[f64]) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));
    sorted[sorted.len() / 2]
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}
