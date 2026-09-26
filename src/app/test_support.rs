use std::{path::Path, process::Command};

use gpui::{AnyWindowHandle, Entity, TestAppContext, VisualTestContext};

use super::{FarcasterApp, runtime::TestRuntime, ui::theme::install_component_theme};

const ISOLATED_APP_TEST: &str = "FARCASTER_OFFLINE_APP_TEST_CHILD";
const INHERITED_ENV: &[&str] = &["DEBUG", "FARCASTER_PERF_TRACE"];

pub(crate) fn with_offline_app(
    test_name: &str,
    cx: &mut TestAppContext,
    test: impl FnOnce(&mut VisualTestContext, &Entity<FarcasterApp>, &TestRuntime, &Path),
) {
    with_prepared_offline_app(test_name, cx, |_| {}, test);
}

pub(crate) fn with_prepared_offline_app(
    test_name: &str,
    cx: &mut TestAppContext,
    prepare: impl FnOnce(&Path),
    test: impl FnOnce(&mut VisualTestContext, &Entity<FarcasterApp>, &TestRuntime, &Path),
) {
    let (runtime, test_runtime) = super::runtime::RuntimeHandle::offline_for_test();
    with_app(
        test_name,
        cx,
        prepare,
        move |_| runtime,
        move |cx, app, project| test(cx, app, &test_runtime, project),
    );
}

/// An app over a runtime the test builds for the project it is given, so a case
/// can drive the app against a real runtime instead of the offline one.
pub(crate) fn with_runtime_app(
    test_name: &str,
    cx: &mut TestAppContext,
    runtime: impl FnOnce(&Path) -> super::runtime::RuntimeHandle,
    test: impl FnOnce(&mut VisualTestContext, &Entity<FarcasterApp>, &Path),
) {
    with_app(test_name, cx, |_| {}, runtime, test);
}

fn with_app(
    test_name: &str,
    cx: &mut TestAppContext,
    prepare: impl FnOnce(&Path),
    runtime: impl FnOnce(&Path) -> super::runtime::RuntimeHandle,
    test: impl FnOnce(&mut VisualTestContext, &Entity<FarcasterApp>, &Path),
) {
    let test_name = test_name
        .split_once("::")
        .map_or(test_name, |(_, test_name)| test_name);
    if !in_isolated_child(test_name) {
        return;
    }

    let project = tempfile::tempdir().expect("isolated app project");
    let project_path = project.path().to_path_buf();
    prepare(&project_path);
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_component::init(cx);
        install_component_theme(cx);
        cx.bind_keys(super::ui::keybindings::bindings());
    });
    let (workgraph_updates, workgraph_rx) = async_channel::unbounded();
    let (worker_updates, worker_rx) = async_channel::unbounded();
    let runtime = runtime(&project_path);
    let window = cx.add_window(|window, cx| {
        FarcasterApp::new_offline_for_test(
            project_path.clone(),
            runtime,
            workgraph_rx,
            worker_rx,
            window,
            cx,
        )
    });
    let app = window.root(cx).expect("offline app root");
    let window: AnyWindowHandle = window.into();
    let cx = VisualTestContext::from_window(window, cx).into_mut();
    let _updates = (workgraph_updates, worker_updates);
    test(cx, &app, project.path());
}

#[allow(clippy::print_stderr)]
fn in_isolated_child(test_name: &str) -> bool {
    if std::env::var(ISOLATED_APP_TEST).as_deref() == Ok(test_name) {
        return true;
    }
    let sandbox = tempfile::tempdir().expect("isolated app test directory");
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command
        .args(["--exact", test_name, "--nocapture"])
        .env_clear()
        .env(ISOLATED_APP_TEST, test_name)
        .env("FARCASTER_DATA_DIR", sandbox.path().join("data"))
        .env("HOME", sandbox.path())
        .env("PATH", "/usr/bin:/bin")
        .env("SHELL", "/bin/sh")
        .current_dir(sandbox.path());
    for name in INHERITED_ENV {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let output = command.output().expect("run isolated app test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "isolated app test failed:\n{}\n{}",
        stdout,
        stderr,
    );
    assert!(
        stdout.contains("1 passed") || stderr.contains("1 passed"),
        "isolated app filter did not run exactly one test:\n{stdout}\n{stderr}",
    );
    if tracing_every_operation() {
        eprint!("{stdout}{stderr}");
    }
    false
}

fn tracing_every_operation() -> bool {
    matches!(
        std::env::var("FARCASTER_PERF_TRACE").as_deref(),
        Ok("1" | "true" | "yes")
    )
}
