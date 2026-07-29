#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use pi_sandbox_broker::linux::{prepare, self_test};
use pi_sandbox_broker::protocol::{Access, DeniedAccess, DenyScope, PathScope};
use pi_sandbox_broker::seatbelt::{NormalizedDeny, NormalizedRight};
use pi_sandbox_broker::validation::ValidatedExec;

#[test]
#[ignore = "release gate: requires a Linux host with unprivileged namespaces and fixed Bubblewrap"]
fn linux_bubblewrap_release_gate() {
    self_test().expect("Bubblewrap readiness self-test");

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("pi-linux-release-{}-{nonce}", std::process::id()));
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let secret = workspace.join("secret.txt");
    fs::write(&secret, "hidden").expect("secret fixture");
    let allowed = workspace.join("allowed.txt");
    let outside = root.join("outside.txt");

    let request = ValidatedExec {
        id: "linux-release".to_owned(),
        program: PathBuf::from("/bin/sh"),
        args: Vec::new(),
        cwd: workspace.clone(),
        env: BTreeMap::new(),
        timeout_ms: Some(5_000),
        output_limit_bytes: 1024,
        rights: vec![
            NormalizedRight {
                access: Access::Read,
                path: PathBuf::from("/"),
                scope: PathScope::Tree,
                approved: false,
            },
            NormalizedRight {
                access: Access::Write,
                path: workspace.clone(),
                scope: PathScope::Tree,
                approved: false,
            },
        ],
        denies: vec![NormalizedDeny {
            access: DeniedAccess::ReadWrite,
            pattern: secret.to_string_lossy().into_owned(),
            scope: DenyScope::File,
            path: Some(secret.clone()),
        }],
        unix_socket_roots: vec![],
    };
    let script = format!(
        "grep -Eq '^NoNewPrivs:[[:space:]]+1$' /proc/self/status || exit 41; cat '{}' >/dev/null 2>&1 && exit 42; touch '{}'; touch '{}'",
        secret.display(),
        allowed.display(),
        outside.display()
    );
    let command = vec!["/bin/sh".to_owned(), "-c".to_owned(), script];
    let launch = prepare(&request, &command).expect("prepare Bubblewrap");
    let status = Command::new(launch.program)
        .args(&launch.args)
        .status()
        .expect("run Bubblewrap");

    assert!(!status.success(), "outside write unexpectedly succeeded");
    assert!(allowed.exists(), "workspace write was blocked");
    assert!(!outside.exists(), "read-only host root was writable");
    assert_eq!(fs::read_to_string(secret).expect("host secret"), "hidden");
    let _ = fs::remove_dir_all(root);
}
