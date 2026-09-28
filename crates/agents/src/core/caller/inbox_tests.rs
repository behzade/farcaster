use super::*;

#[test]
fn retained_inbox_preserves_peeked_and_late_reports_once_in_order() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let parent = identity(&registry, Path::new("/project"), Backend::Codex);
    parent.bind("parent");
    let child = child(&registry, &context(&registry, &parent), "review")?;
    child.bind("child");
    let old_token = parent.token().to_owned();
    let lease = parent.retain_inbox()?.unwrap();
    registry.send(child.token(), "parent", "during close".into())?;
    assert!(parent.has_pending_messages());
    drop(parent);
    assert!(
        registry.resolve(&old_token).is_err(),
        "retirement revokes authentication"
    );
    registry.send(child.token(), "parent", "after close".into())?;
    assert!(lease.has_pending_messages());
    let resumed = identity(&registry, Path::new("/project"), Backend::Codex);
    resumed.bind("parent");
    assert!(lease.transferred());
    drop(lease);
    registry.send(child.token(), "parent", "after resume".into())?;
    for expected in ["during close", "after close", "after resume"] {
        assert_eq!(resumed.try_recv().unwrap().message, expected);
    }
    assert!(resumed.try_recv().is_none());
    Ok(())
}

#[test]
fn retained_inbox_requires_exact_profile_and_survives_failed_resume() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let parent = identity(&registry, Path::new("/project"), Backend::Codex);
    parent.set_harness_profile_id(Some("custom".into()));
    parent.bind("parent");
    let child = child(&registry, &context(&registry, &parent), "review")?;
    let lease = parent.retain_inbox()?.unwrap();
    drop(parent);
    registry.send(child.token(), "parent", "report".into())?;
    let wrong = identity(&registry, Path::new("/project"), Backend::Codex);
    wrong.bind("parent");
    assert!(!lease.transferred());
    assert!(wrong.try_recv().is_none());
    drop(wrong);
    let failed_start = identity(&registry, Path::new("/project"), Backend::Codex);
    failed_start.set_harness_profile_id(Some("custom".into()));
    failed_start.bind("parent");
    assert!(lease.transferred());
    drop(failed_start);
    assert!(!lease.transferred());
    assert!(lease.has_pending_messages());
    let resumed = identity(&registry, Path::new("/project"), Backend::Codex);
    resumed.set_harness_profile_id(Some("custom".into()));
    resumed.bind("parent");
    assert!(lease.transferred());
    assert_eq!(resumed.try_recv().unwrap().message, "report");
    Ok(())
}

#[test]
fn abandoned_retirement_keeps_the_original_consumer() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let parent = identity(&registry, Path::new("/project"), Backend::Pi);
    parent.bind("parent");
    let child = child(&registry, &context(&registry, &parent), "review")?;
    let lease = parent.retain_inbox()?.unwrap();
    registry.send(child.token(), "parent", "close failed".into())?;
    drop(lease);
    assert_eq!(parent.try_recv().unwrap().message, "close failed");
    assert!(registry.resolve(parent.token()).is_ok());
    let token = parent.token().to_owned();
    drop(parent);
    assert!(registry.retired_inboxes.lock().unwrap().is_empty());
    assert!(registry.resolve(&token).is_err());
    Ok(())
}
