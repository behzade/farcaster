use super::*;

#[test]
fn the_flag_is_read_from_any_position_and_leaves_one_project() {
    for arguments in [
        vec!["--isolated", "/projects/app"],
        vec!["/projects/app", "--isolated", "/projects/other"],
    ] {
        let (project, isolated) = split(arguments.into_iter().map(OsString::from));
        assert!(isolated);
        assert_eq!(project, Some(PathBuf::from("/projects/app")));
    }
    let (project, isolated) = split([OsString::from("/projects/app")].into_iter());
    assert!(!isolated);
    assert_eq!(project, Some(PathBuf::from("/projects/app")));
    assert_eq!(
        split([OsString::from("--isolated")].into_iter()),
        (None, true)
    );
}

#[test]
fn copies_durable_app_state_and_resolves_attachments_inside_the_copy() {
    let source = tempfile::tempdir().unwrap();
    let database = source.path().join("state.sqlite3");
    let original = crate::storage::StateStore::open_at(&database).unwrap();
    original
        .enqueue_prompt(
            "draft:isolated-images",
            crate::agents::Backend::Pi,
            source.path(),
            None,
            crate::protocol::PromptMode::Normal,
            "look",
            &[crate::protocol::PromptImage::new(
                "aGVsbG8=".into(),
                "image/png".into(),
            )],
        )
        .unwrap();
    fs::write(source.path().join("projects.json"), b"{\"projects\":[]}").unwrap();
    fs::write(source.path().join("project-trust.json"), b"{}").unwrap();
    fs::create_dir(source.path().join("logs")).unwrap();
    fs::write(source.path().join("logs/private.log"), b"not app state").unwrap();
    fs::write(source.path().join("export.css"), b"not app state").unwrap();
    fs::write(source.path().join("images/.tmp-in-flight"), b"partial").unwrap();

    let isolated = prepare(source.path()).unwrap();
    let root = isolated.directory.path();
    let copied = crate::storage::StateStore::open_at(&root.join("state.sqlite3")).unwrap();
    let prompts = copied.queued_prompts().unwrap();
    assert_eq!(prompts.len(), 1);
    let image = &prompts[0].images[0];
    assert!(image.path.as_ref().unwrap().starts_with(root));
    assert_eq!(image.bytes().unwrap(), b"hello");
    let names = fs::read_dir(root.join("images"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(names.len(), 1);
    assert_eq!(
        fs::read(root.join("images").join(&names[0])).unwrap(),
        b"hello"
    );
    assert_eq!(fs::read(root.join("project-trust.json")).unwrap(), b"{}");
    assert_eq!(
        fs::read(root.join("projects.json")).unwrap(),
        b"{\"projects\":[]}"
    );
    assert!(!root.join("logs").exists());
    assert!(!root.join("export.css").exists());
    fs::write(root.join("project-trust.json"), b"changed").unwrap();
    assert_eq!(
        fs::read(source.path().join("project-trust.json")).unwrap(),
        b"{}"
    );
}

#[test]
fn private_unique_directories_are_removed_when_their_guards_drop() {
    use std::os::unix::fs::PermissionsExt as _;
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("projects.json"), b"original").unwrap();
    let first = prepare(source.path()).unwrap();
    let second = prepare(source.path()).unwrap();
    let first_path = first.directory.path().to_path_buf();
    let second_path = second.directory.path().to_path_buf();
    assert_ne!(first_path, second_path);
    assert_eq!(
        fs::metadata(&first_path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    drop(first);
    assert!(!first_path.exists());
    assert!(second_path.is_dir());
    assert_eq!(
        fs::read(source.path().join("projects.json")).unwrap(),
        b"original"
    );
    drop(second);
    assert!(!second_path.exists());
}

#[test]
fn missing_app_data_starts_empty_but_invalid_state_fails() {
    let source = tempfile::tempdir().unwrap();
    let isolated = prepare(&source.path().join("missing")).unwrap();
    assert_eq!(fs::read_dir(isolated.directory.path()).unwrap().count(), 0);
    assert!(!source.path().join("missing").exists());
    fs::write(source.path().join("state.sqlite3"), b"corrupt").unwrap();
    assert!(prepare(source.path()).is_err());
    assert_eq!(
        fs::read(source.path().join("state.sqlite3")).unwrap(),
        b"corrupt"
    );
}

#[test]
fn symlinked_state_is_rejected_without_traversing_external_data() {
    use std::os::unix::fs::symlink;
    let source = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    fs::write(external.path().join("trust"), b"external").unwrap();
    symlink(
        external.path().join("trust"),
        source.path().join("project-trust.json"),
    )
    .unwrap();
    assert!(prepare(source.path()).is_err());
    assert_eq!(
        fs::read(external.path().join("trust")).unwrap(),
        b"external"
    );
    assert!(prepare(&source.path().join("project-trust.json")).is_err());
}

#[test]
fn installed_copy_routes_app_state_and_mcp_to_the_owned_instance() {
    const CHILD: &str = "FARCASTER_ISOLATION_INSTALL_TEST";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "app::infrastructure::isolation::tests::installed_copy_routes_app_state_and_mcp_to_the_owned_instance", "--nocapture"])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let source = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let project_path = project.path().canonicalize().unwrap();
    let mut original =
        crate::storage::StateStore::open_at(&source.path().join("state.sqlite3")).unwrap();
    crate::projects::save_projects(
        &mut original,
        &crate::projects::ProjectList {
            projects: vec![project_path.clone()],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(window_title("Chat"), "Chat");
    let isolated = install(source.path()).unwrap();
    let root = isolated.directory.path().to_path_buf();
    assert_eq!(super::super::paths::data_dir().unwrap(), root);
    assert_eq!(window_title("Chat"), "Chat — Isolated app state");
    let url = crate::builtin_mcp::url();
    assert_eq!(url, format!("http://{}/mcp", isolated.mcp_addr));
    assert!(TcpListener::bind(&isolated.mcp_addr).is_err());
    assert!(install(source.path()).is_err());
    let shared = super::super::persistence::initialize().unwrap();
    assert_eq!(
        super::super::launch::resolve_project(None).unwrap(),
        project_path
    );
    crate::projects::save_projects(
        &mut *shared.lock().unwrap(),
        &crate::projects::ProjectList::default(),
    )
    .unwrap();
    assert_eq!(
        crate::projects::load_projects(&original).unwrap().projects,
        vec![project_path]
    );
    assert_eq!(crate::builtin_mcp::url(), url);
    drop(shared);
    drop(isolated);
    assert!(!root.exists());
    assert!(source.path().join("state.sqlite3").exists());
}
