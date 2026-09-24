//! Measures what keeping off-screen projects fresh on a slow background pass
//! bought.
//!
//! Before the pass, a project that was not on screen was never read again: its
//! change count stayed whatever it was when it was last warmed, until the user
//! selected it. These cases put a real repository off screen, change it, and
//! time how long the app's remembered count stays wrong, with the pass and
//! without it.
//!
//! The pass reads one project per tick, so the bound on staleness is one tick
//! per off-screen project. A third case pins the round-robin that sets it.
#![allow(clippy::print_stderr)]

use super::*;

use gpui::{Entity, VisualTestContext};
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

const PASS_TICK: Duration = Duration::from_secs(5);
const WARM_TICK: Duration = Duration::from_millis(600);
const TICK_LIMIT: u32 = 3;

fn git(project: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(project)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A real repository with one committed file and one uncommitted change, so the
/// app has a working copy to remember and a later write shows up as another
/// change.
fn new_repository() -> tempfile::TempDir {
    let repository = tempfile::tempdir_in(
        std::env::temp_dir()
            .canonicalize()
            .expect("canonical temp dir"),
    )
    .expect("test repository");
    git(repository.path(), &["init", "-q"]);
    fs::write(repository.path().join("tracked.txt"), "one\n").expect("write tracked file");
    git(repository.path(), &["add", "."]);
    git(
        repository.path(),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "base",
        ],
    );
    fs::write(repository.path().join("changed.txt"), "changed\n").expect("write a change");
    repository
}

/// What the app would see if it read the working copy right now, which is the
/// number the remembered one has to keep up with.
fn current_changes(project: &Path) -> usize {
    let (_, scanned) = observe_project(project, BackendPreference::default())
        .expect("scan the test repository")
        .expect("the test repository is discoverable");
    let (snapshot, _, _) = scanned.expect("snapshot the test repository");
    snapshot.changes.len()
}

fn scan_ms(project: &Path) -> f64 {
    let started = Instant::now();
    let changes = current_changes(project);
    assert!(changes > 0, "the test repository has a change to count");
    millis(started.elapsed())
}

/// The change count the app is holding for a project it does not have on
/// screen.
fn remembered_changes(app: &FarcasterApp, project: &Path) -> Option<usize> {
    app.project
        .repository
        .observations
        .projects
        .get(project)
        .and_then(|observation| observation.snapshot.as_ref())
        .map(|snapshot| snapshot.changes.len())
}

/// Moves the app's own clock forward by whole pass ticks, so the count of ticks
/// a project stays stale is what the pass's timer decides, not the test's.
fn ticks(cx: &mut VisualTestContext, app: &Entity<FarcasterApp>, count: u32) {
    for _ in 0..count {
        cx.executor().advance_clock(PASS_TICK);
        pump(cx, app);
    }
}

/// Runs everything the app has queued, which is all the warm-up pass needs: it
/// waits on timers shorter than one tick.
fn pump(cx: &mut VisualTestContext, app: &Entity<FarcasterApp>) {
    cx.executor().advance_clock(WARM_TICK);
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| app.drain_runtime(cx));
        window.draw(cx).clear(cx);
    });
}

/// Warms the app's remembered working copies with the pass either running or
/// suppressed, and waits for the off-screen project to be remembered at all.
fn warm_off_screen(
    cx: &mut VisualTestContext,
    app: &Entity<FarcasterApp>,
    repository: &Path,
    with_pass: bool,
) -> usize {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            if !with_pass {
                // The state the pass leaves behind, without the pass's work.
                app.project.repository.pass_started = true;
            }
            app.project.registered.push(repository.to_path_buf());
            app.warm_repository_observations(cx);
        });
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        pump(cx, app);
        if let Some(changes) = cx.update(|_, cx| remembered_changes(app.read(cx), repository)) {
            return changes;
        }
        assert!(
            Instant::now() < deadline,
            "warming should remember the off-screen project"
        );
    }
}

#[gpui::test]
fn an_off_screen_project_stays_stale_without_the_pass(cx: &mut gpui::TestAppContext) {
    let test_name = concat!(
        module_path!(),
        "::an_off_screen_project_stays_stale_without_the_pass"
    );
    crate::app::test_support::with_offline_app(test_name, cx, |cx, app, _, _| {
        let repository = new_repository();
        let repository = repository
            .path()
            .canonicalize()
            .expect("canonical repository path");
        let repository = repository.as_path();
        let warmed = warm_off_screen(cx, app, repository, false);
        assert_eq!(warmed, current_changes(repository));

        // A change while the project is off screen, which nothing reads back.
        fs::write(repository.join("arrived.txt"), "two\n").expect("change the off-screen project");
        const TICKS: u32 = 3;
        ticks(cx, app, TICKS);
        let held = cx.update(|_, cx| remembered_changes(app.read(cx), repository));
        let actual = current_changes(repository);

        assert_eq!(
            held,
            Some(warmed),
            "without the pass the remembered count never catches up"
        );
        assert!(actual > warmed);
        eprintln!("OFFSCREEN_MEASURE no_pass_ticks={TICKS} remembered={warmed} actual={actual}");
    });
}

#[gpui::test]
fn the_pass_refreshes_an_off_screen_project_without_selecting_it(cx: &mut gpui::TestAppContext) {
    let test_name = concat!(
        module_path!(),
        "::the_pass_refreshes_an_off_screen_project_without_selecting_it"
    );
    crate::app::test_support::with_offline_app(test_name, cx, |cx, app, _, _| {
        let repository = new_repository();
        let repository = repository
            .path()
            .canonicalize()
            .expect("canonical repository path");
        let repository = repository.as_path();
        let scan = scan_ms(repository);
        let warmed = warm_off_screen(cx, app, repository, true);
        assert_eq!(warmed, current_changes(repository));

        fs::write(repository.join("arrived.txt"), "two\n").expect("change the off-screen project");
        let expected = current_changes(repository);
        assert!(expected > warmed);

        let mut waited = 0;
        while cx.update(|_, cx| remembered_changes(app.read(cx), repository)) != Some(expected) {
            assert!(
                waited < TICK_LIMIT,
                "the pass should refresh an off-screen project on its own"
            );
            ticks(cx, app, 1);
            waited += 1;
        }

        assert_eq!(
            waited, 1,
            "an off-screen project should be current one tick after it changes"
        );
        eprintln!("OFFSCREEN_MEASURE scan_ms={scan:.2}");
        eprintln!(
            "OFFSCREEN_MEASURE off_screen_projects=1 staleness_ticks=1 tick_ms={} staleness_ms={}",
            PASS_TICK.as_millis(),
            PASS_TICK.as_millis()
        );
    });
}

#[gpui::test]
fn the_pass_reads_each_off_screen_project_once_per_cycle(cx: &mut gpui::TestAppContext) {
    let test_name = concat!(
        module_path!(),
        "::the_pass_reads_each_off_screen_project_once_per_cycle"
    );
    crate::app::test_support::with_offline_app(test_name, cx, |cx, app, _, project| {
        let current = project
            .canonicalize()
            .expect("canonical project path")
            .to_path_buf();
        let repositories = (0..3)
            .map(|_| {
                tempfile::tempdir()
                    .expect("off-screen project")
                    .keep()
                    .canonicalize()
                    .expect("canonical off-screen project")
            })
            .collect::<Vec<_>>();
        cx.update(|_, cx| {
            app.update(cx, |app, _| {
                app.project.registered = repositories.clone();
            });
        });

        let order = cx.update(|_, cx| {
            app.update(cx, |app, _| {
                (0..4)
                    .filter_map(|_| app.next_offscreen_project())
                    .collect::<Vec<_>>()
            })
        });

        assert_eq!(
            order.len(),
            4,
            "every off-screen project should be read on every tick"
        );
        assert!(
            !order.contains(&current),
            "the project on screen is refreshed by its own path"
        );
        let mut cycle = order[..3].to_vec();
        cycle.sort();
        assert_eq!(
            cycle,
            {
                let mut expected = repositories.clone();
                expected.sort();
                expected
            },
            "one tick should visit each off-screen project once"
        );
        assert_eq!(
            order[3], order[0],
            "the cycle should start again after the last project"
        );
        eprintln!(
            "OFFSCREEN_MEASURE off_screen_projects=3 cycle_tick_ms={} worst_case_staleness_ms={}",
            PASS_TICK.as_millis(),
            PASS_TICK.as_millis() * 3
        );
    });
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}
