use std::{path::Path, process::Command};

use gpui::{AnyWindowHandle, Entity, TestAppContext, VisualTestContext};

use super::{FarcasterApp, runtime::TestRuntime, ui::theme::install_component_theme};

const ISOLATED_APP_TEST: &str = "FARCASTER_OFFLINE_APP_TEST_CHILD";

pub(crate) fn with_offline_app(
    test_name: &str,
    cx: &mut TestAppContext,
    test: impl FnOnce(&mut VisualTestContext, &Entity<FarcasterApp>, &TestRuntime, &Path),
) {
    let test_name = test_name
        .split_once("::")
        .map_or(test_name, |(_, test_name)| test_name);
    if std::env::var(ISOLATED_APP_TEST).as_deref() != Ok(test_name) {
        let sandbox = tempfile::tempdir().expect("isolated app test directory");
        let output = Command::new(std::env::current_exe().expect("current test executable"))
            .args(["--exact", test_name, "--nocapture"])
            .env_clear()
            .env(ISOLATED_APP_TEST, test_name)
            .env("FARCASTER_DATA_DIR", sandbox.path().join("data"))
            .env("HOME", sandbox.path())
            .env("PATH", "/usr/bin:/bin")
            .env("SHELL", "/bin/sh")
            .current_dir(sandbox.path())
            .output()
            .expect("run isolated app test");
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
        return;
    }

    let project = tempfile::tempdir().expect("offline app project");
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_component::init(cx);
        install_component_theme(cx);
        cx.bind_keys(super::ui::keybindings::bindings());
    });
    let (workgraph_updates, workgraph_rx) = async_channel::unbounded();
    let (worker_updates, worker_rx) = async_channel::unbounded();
    let (runtime, test_runtime) = super::runtime::RuntimeHandle::offline_for_test();
    let window = cx.add_window(|window, cx| {
        FarcasterApp::new_offline_for_test(
            project.path().to_path_buf(),
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
    test(cx, &app, &test_runtime, project.path());
}
