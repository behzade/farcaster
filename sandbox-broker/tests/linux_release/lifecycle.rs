use std::fs;
use std::path::Path;

use super::support::{Broker, TempRoot, assert_no_survivor, find_program, request, shell_quote};

const OUTPUT_LIMIT: u64 = 4 * 1024;

#[test]
#[ignore = "release gate: requires an unsandboxed Linux host with fixed Bubblewrap"]
fn linux_descendant_completion_and_timeout_release_gate() {
    let root = TempRoot::new("lifecycle-completion");
    let workspace = root.0.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    let setsid = find_program("setsid");
    let shell = find_program("sh");
    let mut broker = Broker::start();

    let detached_ready = workspace.join("detached-ready");
    let detached_trigger = workspace.join("detached-release");
    let detached_marker = workspace.join("detached-survived");
    let detached = broker.exec(request(
        "detached-cleanup",
        &workspace,
        detached_script(
            &setsid,
            &shell,
            &detached_ready,
            &detached_trigger,
            &detached_marker,
            "true",
        ),
        vec![],
        vec![],
        Some(5_000),
        OUTPUT_LIMIT,
    ));
    assert_eq!(detached.code, Some(0));
    assert_no_survivor(&detached_trigger, &detached_marker);

    let double_ready = workspace.join("double-fork-ready");
    let double_trigger = workspace.join("double-fork-release");
    let double_marker = workspace.join("double-fork-survived");
    let double_fork = broker.exec(request(
        "double-fork-cleanup",
        &workspace,
        double_fork_script(&shell, &double_ready, &double_trigger, &double_marker),
        vec![],
        vec![],
        Some(5_000),
        OUTPUT_LIMIT,
    ));
    assert_eq!(double_fork.code, Some(0));
    assert_no_survivor(&double_trigger, &double_marker);

    let timeout_ready = workspace.join("timeout-ready");
    let timeout_trigger = workspace.join("timeout-release");
    let timeout_marker = workspace.join("timeout-survived");
    let timed_out = broker.exec(request(
        "timeout-detached-cleanup",
        &workspace,
        detached_script(
            &setsid,
            &shell,
            &timeout_ready,
            &timeout_trigger,
            &timeout_marker,
            "sleep 30",
        ),
        vec![],
        vec![],
        Some(500),
        OUTPUT_LIMIT,
    ));
    assert!(timed_out.timed_out);
    assert!(!timed_out.cancelled);
    assert_no_survivor(&timeout_trigger, &timeout_marker);
}

#[test]
#[ignore = "release gate: requires an unsandboxed Linux host with fixed Bubblewrap"]
fn linux_descendant_cancel_and_shutdown_release_gate() {
    let root = TempRoot::new("lifecycle-termination");
    let workspace = root.0.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    let setsid = find_program("setsid");
    let shell = find_program("sh");
    let mut broker = Broker::start();

    let cancel_ready = workspace.join("cancel-ready");
    let cancel_trigger = workspace.join("cancel-release");
    let cancel_marker = workspace.join("cancel-survived");
    let cancelled = broker.exec_and_cancel(request(
        "cancel-detached-cleanup",
        &workspace,
        detached_script(
            &setsid,
            &shell,
            &cancel_ready,
            &cancel_trigger,
            &cancel_marker,
            "printf 'PI_RELEASE_READY\\n'; sleep 30",
        ),
        vec![],
        vec![],
        None,
        OUTPUT_LIMIT,
    ));
    assert!(cancelled.cancelled);
    assert!(!cancelled.timed_out);
    assert_no_survivor(&cancel_trigger, &cancel_marker);

    let shutdown_ready = workspace.join("shutdown-ready");
    let shutdown_trigger = workspace.join("shutdown-release");
    let shutdown_marker = workspace.join("shutdown-survived");
    let mut shutdown_broker = Broker::start();
    let shutdown = shutdown_broker.exec_and_shutdown(request(
        "shutdown-detached-cleanup",
        &workspace,
        detached_script(
            &setsid,
            &shell,
            &shutdown_ready,
            &shutdown_trigger,
            &shutdown_marker,
            "printf 'PI_RELEASE_READY\\n'; sleep 30",
        ),
        vec![],
        vec![],
        None,
        OUTPUT_LIMIT,
    ));
    assert!(shutdown.cancelled);
    shutdown_broker.wait_for_exit();
    assert_no_survivor(&shutdown_trigger, &shutdown_marker);
}

fn double_fork_script(shell: &Path, ready: &Path, trigger: &Path, survivor: &Path) -> String {
    let grandchild = survivor_script(ready, trigger, survivor);
    let child = format!(
        "{} -c {} &",
        shell_quote(&shell.to_string_lossy()),
        shell_quote(&grandchild)
    );
    format!(
        "{} -c {} & while [ ! -e {} ]; do :; done",
        shell_quote(&shell.to_string_lossy()),
        shell_quote(&child),
        shell_quote(&ready.to_string_lossy())
    )
}

fn detached_script(
    setsid: &Path,
    shell: &Path,
    ready: &Path,
    trigger: &Path,
    survivor: &Path,
    tail: &str,
) -> String {
    let child = survivor_script(ready, trigger, survivor);
    format!(
        "{} -f {} -c {}; while [ ! -e {} ]; do :; done; {tail}",
        shell_quote(&setsid.to_string_lossy()),
        shell_quote(&shell.to_string_lossy()),
        shell_quote(&child),
        shell_quote(&ready.to_string_lossy()),
    )
}

fn survivor_script(ready: &Path, trigger: &Path, survivor: &Path) -> String {
    format!(
        "printf ready > {}; while [ ! -e {} ]; do :; done; printf survived > {}",
        shell_quote(&ready.to_string_lossy()),
        shell_quote(&trigger.to_string_lossy()),
        shell_quote(&survivor.to_string_lossy())
    )
}
