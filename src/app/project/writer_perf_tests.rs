//! Compares synchronous registry/folder saves with the UI's queued saves.
//! Report enqueue and flush time separately; timing is diagnostic, not a pass
//! threshold. Every sample verifies the resulting state in SQLite.
#![allow(clippy::print_stderr)]

use super::*;
use gpui::{Entity, VisualTestContext};
use std::time::{Duration, Instant};

const WRITES: usize = 10;

fn settle(cx: &mut VisualTestContext, app: &Entity<FarcasterApp>) {
    let flush = cx.update(|_, cx| app.read(cx).sessions.writer.flush());
    futures::executor::block_on(flush).expect("persist queued changes");
}

fn save(cx: &mut VisualTestContext, app: &Entity<FarcasterApp>, queued: bool) -> Duration {
    let started = Instant::now();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            if queued {
                app.save_project_registry();
                assert!(app.save_session_folders(app.sessions.folders.clone(), cx));
            } else {
                project_registry::save(&projects::ProjectList {
                    projects: app.project.registered.clone(),
                    excluded_projects: app.project.excluded.clone(),
                })
                .expect("save registry inline");
                crate::app::persistence::open()
                    .unwrap()
                    .save_session_folders(&app.sessions.folders)
                    .expect("save folders inline");
                // Include the same view update as save_session_folders above.
                app.notify_session_rail(cx);
            }
        });
    });
    started.elapsed()
}

fn assert_saved(cx: &mut VisualTestContext, app: &Entity<FarcasterApp>) {
    cx.update(|_, cx| {
        let app = app.read(cx);
        let registry = project_registry::load().expect("read saved registry");
        assert_eq!(registry.projects, app.project.registered);
        assert_eq!(registry.excluded_projects, app.project.excluded);
        let folders = crate::app::persistence::open()
            .unwrap()
            .load_session_folders()
            .expect("read saved folders");
        assert_eq!(
            serde_json::to_value(folders).unwrap(),
            serde_json::to_value(&app.sessions.folders).unwrap()
        );
    });
}

#[gpui::test]
fn folder_and_registry_writes_leave_the_click_path(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::folder_and_registry_writes_leave_the_click_path"
        ),
        cx,
        |cx, app, _, project| {
            let project = project.canonicalize().expect("canonical project");
            settle(cx, app);
            let paths = [project.join("first"), project.join("second")];
            for path in &paths {
                std::fs::create_dir(path).expect("project fixture");
            }
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    app.project.registered = vec![project.clone(), paths[0].clone()];
                    app.project.excluded = vec![paths[1].clone()];
                })
            });
            // Warm both paths before measuring, including schema and journal setup.
            save(cx, app, false);
            save(cx, app, true);
            settle(cx, app);
            let mut inline = Vec::new();
            let mut handoff = Vec::new();
            let mut flush = Vec::new();
            for index in 0..WRITES {
                // Alternate the order; both paths save the same number of rows.
                for queued in [index % 2 == 0, index % 2 != 0] {
                    let selected = usize::from(queued);
                    cx.update(|_, cx| {
                        app.update(cx, |app, _| {
                            app.project.registered = vec![project.clone(), paths[selected].clone()];
                            app.project.excluded = vec![paths[1 - selected].clone()];
                            app.sessions.folders = Default::default();
                            app.sessions
                                .folders
                                .create(format!("Folder {index}-{queued}"), None);
                        })
                    });
                    let elapsed = millis(save(cx, app, queued));
                    if queued {
                        handoff.push(elapsed);
                        let started = Instant::now();
                        settle(cx, app);
                        flush.push(millis(started.elapsed()));
                    } else {
                        inline.push(elapsed);
                    }
                    assert_saved(cx, app);
                }
            }
            // A burst may coalesce. Verify the final state after its barrier.
            let started = Instant::now();
            for index in 0..WRITES {
                cx.update(|_, cx| {
                    app.update(cx, |app, cx| {
                        app.sessions.folders.create(format!("Burst {index}"), None);
                        app.save_project_registry();
                        assert!(app.save_session_folders(app.sessions.folders.clone(), cx));
                    })
                });
            }
            let burst_enqueue = started.elapsed();
            let started = Instant::now();
            settle(cx, app);
            let burst_flush = started.elapsed();
            assert_saved(cx, app);
            eprintln!("WRITER_MEASURE writes={WRITES}");
            eprintln!(
                "WRITER_MEASURE inline_ms={} median={:.3}",
                format(&inline),
                median(&inline)
            );
            eprintln!(
                "WRITER_MEASURE handoff_ms={} median={:.3}",
                format(&handoff),
                median(&handoff)
            );
            eprintln!(
                "WRITER_MEASURE flush_wait_ms={} median={:.3}",
                format(&flush),
                median(&flush)
            );
            eprintln!(
                "WRITER_MEASURE burst_updates={WRITES} enqueue_ms={:.3} flush_wait_ms={:.3}",
                millis(burst_enqueue),
                millis(burst_flush)
            );
        },
    );
}

fn format(samples: &[f64]) -> String {
    samples
        .iter()
        .map(|sample| format!("{sample:.3}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn median(samples: &[f64]) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    (sorted[(sorted.len() - 1) / 2] + sorted[middle]) / 2.0
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}
