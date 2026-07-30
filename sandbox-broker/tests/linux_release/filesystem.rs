use std::fs;
use std::os::unix::fs::symlink;

use pi_sandbox_broker::protocol::DeniedAccess;

use super::support::{
    Broker, TempRoot, file_deny, file_grant, request, shell_quote, tree_grant,
    wait_for_path_absence,
};

const OUTPUT_LIMIT: u64 = 4 * 1024;

#[test]
#[ignore = "release gate: requires an unsandboxed Linux host with fixed Bubblewrap"]
fn linux_filesystem_rights_release_gate() {
    let root = TempRoot::new("filesystem");
    let workspace = root.0.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    let mut broker = Broker::start();

    let workspace_file = workspace.join("allowed.txt");
    let allowed = broker.exec(request(
        "workspace-write",
        &workspace,
        format!(
            "printf allowed > {}",
            shell_quote(&workspace_file.to_string_lossy())
        ),
        vec![],
        vec![],
        None,
        OUTPUT_LIMIT,
    ));
    assert_eq!(allowed.code, Some(0));
    assert_eq!(
        fs::read_to_string(&workspace_file).expect("workspace write"),
        "allowed"
    );

    let outside = root.0.join("outside.txt");
    let denied = broker.exec(request(
        "outside-write-denied",
        &workspace,
        format!(
            "printf denied > {}",
            shell_quote(&outside.to_string_lossy())
        ),
        vec![],
        vec![],
        None,
        OUTPUT_LIMIT,
    ));
    assert_ne!(denied.code, Some(0));
    assert!(!outside.exists());

    let granted = broker.exec(request(
        "outside-file-granted",
        &workspace,
        format!(
            "printf granted > {}",
            shell_quote(&outside.to_string_lossy())
        ),
        vec![file_grant(&outside)],
        vec![],
        None,
        OUTPUT_LIMIT,
    ));
    assert_eq!(granted.code, Some(0));
    assert_eq!(
        fs::read_to_string(&outside).expect("granted file"),
        "granted"
    );

    let grant_is_fresh = broker.exec(request(
        "outside-file-fresh-rights",
        &workspace,
        format!("printf stale > {}", shell_quote(&outside.to_string_lossy())),
        vec![],
        vec![],
        None,
        OUTPUT_LIMIT,
    ));
    assert_ne!(grant_is_fresh.code, Some(0));
    assert_eq!(
        fs::read_to_string(&outside).expect("unchanged granted file"),
        "granted"
    );

    let missing_tree = root.0.join("state").join("created-tree");
    let tree_file = missing_tree.join("value.txt");
    let created_tree = broker.exec(request(
        "missing-tree-granted",
        &workspace,
        format!(
            "printf tree > {}",
            shell_quote(&tree_file.to_string_lossy())
        ),
        vec![tree_grant(&missing_tree)],
        vec![],
        None,
        OUTPUT_LIMIT,
    ));
    assert_eq!(created_tree.code, Some(0));
    assert_eq!(
        fs::read_to_string(tree_file).expect("created tree file"),
        "tree"
    );
}

#[test]
#[ignore = "release gate: requires an unsandboxed Linux host with fixed Bubblewrap"]
fn linux_read_denies_release_gate() {
    let root = TempRoot::new("read-denies");
    let workspace = root.0.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    let mut broker = Broker::start();

    let secret = workspace.join("secret.txt");
    fs::write(&secret, "hidden\n").expect("secret fixture");
    let hidden = broker.exec(request(
        "explicit-read-deny",
        &workspace,
        format!(
            "if IFS= read -r value 2>/dev/null < {}; then exit 41; else printf hidden-ok; fi",
            shell_quote(&secret.to_string_lossy())
        ),
        vec![],
        vec![file_deny(DeniedAccess::ReadWrite, &secret)],
        None,
        OUTPUT_LIMIT,
    ));
    assert_eq!(hidden.code, Some(0));
    assert_eq!(hidden.output, b"hidden-ok");
    assert_eq!(
        fs::read_to_string(&secret).expect("host secret"),
        "hidden\n"
    );

    let env_file = workspace.join(".env");
    fs::write(&env_file, "TOKEN=hidden\n").expect("environment secret fixture");
    let hard_hidden = broker.exec(request(
        "hard-glob-read-deny",
        &workspace,
        format!(
            "if IFS= read -r value 2>/dev/null < {}; then exit 42; else printf hard-hidden-ok; fi",
            shell_quote(&env_file.to_string_lossy())
        ),
        vec![],
        vec![],
        None,
        OUTPUT_LIMIT,
    ));
    assert_eq!(hard_hidden.code, Some(0));
    assert_eq!(hard_hidden.output, b"hard-hidden-ok");
}

#[test]
#[ignore = "release gate: requires an unsandboxed Linux host with fixed Bubblewrap"]
fn linux_control_and_symlink_release_gate() {
    let root = TempRoot::new("controls");
    let workspace = root.0.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    let mut broker = Broker::start();

    let git = workspace.join(".git");
    fs::create_dir(&git).expect("git control fixture");
    let git_config = git.join("config");
    let protected_git = broker.exec(request(
        "git-protected",
        &workspace,
        format!(
            "printf bad > {}",
            shell_quote(&git_config.to_string_lossy())
        ),
        vec![],
        vec![],
        None,
        OUTPUT_LIMIT,
    ));
    assert_ne!(protected_git.code, Some(0));
    assert!(!git_config.exists());

    let approved_git = broker.exec(request(
        "git-approved",
        &workspace,
        format!("printf ok > {}", shell_quote(&git_config.to_string_lossy())),
        vec![tree_grant(&git)],
        vec![],
        None,
        OUTPUT_LIMIT,
    ));
    assert_eq!(approved_git.code, Some(0));
    assert_eq!(
        fs::read_to_string(git_config).expect("approved git write"),
        "ok"
    );

    let pi_control = workspace.join(".pi");
    let protected_pi = broker.exec(request(
        "missing-pi-protected",
        &workspace,
        format!(
            "mkdir -p {0} && printf bad > {0}/config.json",
            shell_quote(&pi_control.to_string_lossy())
        ),
        vec![],
        vec![],
        None,
        OUTPUT_LIMIT,
    ));
    assert_ne!(protected_pi.code, Some(0));
    wait_for_path_absence(&pi_control);

    let outside_secret = root.0.join("outside-secret.txt");
    fs::write(&outside_secret, "alias-hidden\n").expect("outside secret fixture");
    let alias = workspace.join("secret-link");
    symlink(&outside_secret, &alias).expect("secret symlink fixture");
    let hidden_alias = broker.exec(request(
        "symlink-deny",
        &workspace,
        format!(
            "if IFS= read -r value 2>/dev/null < {}; then exit 43; else printf alias-hidden-ok; fi",
            shell_quote(&alias.to_string_lossy())
        ),
        vec![],
        vec![file_deny(DeniedAccess::Read, &alias)],
        None,
        OUTPUT_LIMIT,
    ));
    assert_eq!(hidden_alias.code, Some(0));
    assert_eq!(hidden_alias.output, b"alias-hidden-ok");
}
