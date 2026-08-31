use super::*;

fn setup() -> Result<
    (
        tempfile::TempDir,
        ApprovalService,
        ApprovalUi,
        PathBuf,
        PathBuf,
    ),
    String,
> {
    setup_with_nono(crate::access::test_nono_bypass())
}

fn setup_with_nono(
    nono: crate::access::NonoExecutable,
) -> Result<
    (
        tempfile::TempDir,
        ApprovalService,
        ApprovalUi,
        PathBuf,
        PathBuf,
    ),
    String,
> {
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let project = root.path().join("project");
    let home = root.path().join("home");
    fs::create_dir(&project).map_err(|error| error.to_string())?;
    fs::create_dir_all(home.join(".pi/agent")).map_err(|error| error.to_string())?;
    let temporary = root.path().join("tmp");
    fs::create_dir(&temporary).map_err(|error| error.to_string())?;
    let (service, ui) = channel(
        &project,
        &home,
        root.path(),
        &home.join(".pi/agent"),
        &temporary,
        nono,
    )?;
    Ok((root, service, ui, project, home))
}

#[tokio::test]
async fn session_approval_activates_exact_external_right() -> Result<(), String> {
    let (root, service, ui, _project, _home) = setup()?;
    ui.set_project_trusted(true);
    let external = root.path().join("external");
    let input = root.path().join("input.txt");
    fs::create_dir(&external).map_err(|error| error.to_string())?;
    fs::write(&input, "fixture").map_err(|error| error.to_string())?;
    let request = tokio::spawn({
        let service = service.clone();
        let path = external
            .canonicalize()
            .expect("canonical external path")
            .display()
            .to_string();
        let input = input
            .canonicalize()
            .expect("canonical input path")
            .display()
            .to_string();
        async move {
            service
                .request_access(RequestAccessParams {
                    rights: vec![
                        AccessRight::Filesystem {
                            access: FilesystemRightAccess::Write,
                            path,
                            scope: FilesystemScope::Tree,
                        },
                        AccessRight::Filesystem {
                            access: FilesystemRightAccess::Read,
                            path: input,
                            scope: FilesystemScope::File,
                        },
                    ],
                    reason: "update the sibling checkout".into(),
                })
                .await
        }
    });
    let prompt = ui
        .receiver()
        .recv()
        .await
        .map_err(|error| error.to_string())?;
    assert_eq!(prompt.options, [SESSION_LABEL, DENY_LABEL]);
    assert!(prompt.title.contains("update the sibling checkout"));
    assert!(ui.respond(&prompt.id, SESSION_LABEL)?);
    tokio::time::timeout(std::time::Duration::from_secs(2), request)
        .await
        .map_err(|_| "approval response timed out".to_owned())?
        .map_err(|error| error.to_string())??;
    let grants = ui.grants().resolve();
    assert_eq!(
        grants.writable,
        [external.canonicalize().map_err(|error| error.to_string())?]
    );
    assert_eq!(
        grants.readable_files,
        [input.canonicalize().map_err(|error| error.to_string())?]
    );
    Ok(())
}

#[tokio::test]
async fn project_approval_is_bound_and_persisted_outside_the_workspace() -> Result<(), String> {
    let (root, service, ui, project, home) = setup()?;
    ui.set_project_trusted(true);
    let host = "downloads.example.com";
    let request = tokio::spawn({
        let service = service.clone();
        async move {
            service
                .request_access(RequestAccessParams {
                    rights: vec![AccessRight::NetworkHost { host: host.into() }],
                    reason: "download a fixture".into(),
                })
                .await
        }
    });
    let prompt = ui
        .receiver()
        .recv()
        .await
        .map_err(|error| error.to_string())?;
    assert_eq!(prompt.options, [SESSION_LABEL, PROJECT_LABEL, DENY_LABEL]);
    assert!(ui.respond(&prompt.id, PROJECT_LABEL)?);
    request.await.map_err(|error| error.to_string())??;

    let canonical_root = root
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let policy = policy_path(
        &canonical_root,
        &project.canonicalize().map_err(|error| error.to_string())?,
    );
    assert!(policy.starts_with(canonical_root.join("sandbox/projects")));
    assert!(policy.exists(), "missing policy at {}", policy.display());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            policy
                .metadata()
                .map_err(|error| error.to_string())?
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let (_, reloaded) = channel(
        &project,
        &home,
        root.path(),
        &home.join(".pi/agent"),
        &root.path().join("tmp"),
        crate::access::test_nono_bypass(),
    )?;
    assert_eq!(reloaded.grants().resolve().network_hosts, [host.to_owned()]);
    Ok(())
}

#[tokio::test]
async fn cancelling_ui_state_unblocks_pending_tool_calls() -> Result<(), String> {
    let (_root, service, ui, _project, _home) = setup()?;
    ui.set_project_trusted(true);
    let request = tokio::spawn(async move {
        service
            .request_access(RequestAccessParams {
                rights: vec![AccessRight::NetworkHost {
                    host: "cancel.example.com".into(),
                }],
                reason: "test cancellation".into(),
            })
            .await
    });
    ui.receiver()
        .recv()
        .await
        .map_err(|error| error.to_string())?;
    ui.cancel_all();
    let error = request
        .await
        .map_err(|error| error.to_string())?
        .expect_err("cancelled approval must fail");
    assert!(error.contains("cancelled"));
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn rejected_nono_profile_is_not_activated_or_persisted() -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;

    let nono_root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let nono = nono_root.path().join("nono");
    fs::write(&nono, "#!/bin/sh\necho invalid-profile >&2\nexit 1\n")
        .map_err(|error| error.to_string())?;
    fs::set_permissions(&nono, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let (root, service, ui, project, _home) =
        setup_with_nono(crate::access::NonoExecutable::Fixed(nono))?;
    ui.set_project_trusted(true);
    let request = tokio::spawn(async move {
        service
            .request_access(RequestAccessParams {
                rights: vec![AccessRight::NetworkHost {
                    host: "validation.example.com".into(),
                }],
                reason: "validate rollback".into(),
            })
            .await
    });
    let prompt = ui
        .receiver()
        .recv()
        .await
        .map_err(|error| error.to_string())?;
    assert!(!ui.respond(&prompt.id, PROJECT_LABEL)?);
    let error = request
        .await
        .map_err(|error| error.to_string())?
        .expect_err("invalid profile must fail");
    assert!(error.contains("invalid-profile"));
    let canonical_root = root
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?;
    assert!(
        !policy_path(
            &canonical_root,
            &project.canonicalize().map_err(|error| error.to_string())?
        )
        .exists()
    );
    Ok(())
}

#[tokio::test]
async fn untrusted_projects_and_protected_paths_fail_closed() -> Result<(), String> {
    let (root, service, ui, project, _home) = setup()?;
    let untrusted = service
        .request_access(RequestAccessParams {
            rights: vec![AccessRight::NetworkHost {
                host: "example.com".into(),
            }],
            reason: "test".into(),
        })
        .await
        .expect_err("untrusted request must fail");
    assert!(untrusted.contains("trusted project"));

    ui.set_project_trusted(true);
    let invalid_port = service
        .request_access(RequestAccessParams {
            rights: vec![AccessRight::NetworkEndpoint {
                host: "localhost".into(),
                port: 0,
            }],
            reason: "test".into(),
        })
        .await
        .expect_err("port zero must fail");
    assert!(invalid_port.contains("1 to 65535"));

    let git = project.join(".git");
    fs::create_dir(&git).map_err(|error| error.to_string())?;
    let protected = service
        .request_access(RequestAccessParams {
            rights: vec![AccessRight::Filesystem {
                access: FilesystemRightAccess::Write,
                path: git
                    .canonicalize()
                    .map_err(|error| error.to_string())?
                    .display()
                    .to_string(),
                scope: FilesystemScope::Tree,
            }],
            reason: "test".into(),
        })
        .await
        .expect_err("protected writes must fail");
    assert!(protected.contains("protected write"));
    drop(root);
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlinked_rights_are_checked_at_the_resolved_target() -> Result<(), String> {
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let project = root.path().join("project");
    let home = root.path().join("home");
    let target = root.path().join("target");
    fs::create_dir(&project).map_err(|error| error.to_string())?;
    fs::create_dir(&target).map_err(|error| error.to_string())?;
    fs::create_dir_all(home.join(".ssh")).map_err(|error| error.to_string())?;

    let link = project.join("link");
    std::os::unix::fs::symlink(&target, &link).map_err(|error| error.to_string())?;
    let normalized = normalize_right(
        &AccessRight::Filesystem {
            access: FilesystemRightAccess::Read,
            path: link.display().to_string(),
            scope: FilesystemScope::Tree,
        },
        &project,
        &home,
    )?;
    let AccessRight::Filesystem { path, .. } = normalized else {
        return Err("filesystem right changed kind".into());
    };
    assert_eq!(
        path,
        target
            .canonicalize()
            .map_err(|error| error.to_string())?
            .display()
            .to_string()
    );

    let protected_link = project.join("protected-link");
    std::os::unix::fs::symlink(home.join(".ssh"), &protected_link)
        .map_err(|error| error.to_string())?;
    let error = normalize_right(
        &AccessRight::Filesystem {
            access: FilesystemRightAccess::Read,
            path: protected_link.display().to_string(),
            scope: FilesystemScope::Tree,
        },
        &project,
        &home,
    )
    .expect_err("protected symlink target must fail");
    assert!(error.contains("protected read"));
    Ok(())
}
