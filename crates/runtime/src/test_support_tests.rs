use super::*;

#[test]
fn default_hosts_do_not_share_drafts_or_queues() -> Result<(), String> {
    let first = host();
    let second = host();
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let draft = farcaster_sessions::DraftSession::with_id(
        Some(farcaster_agents::Backend::Pi),
        "same-draft".into(),
        project.path().into(),
    );
    first.state_store()?.with(|store| {
        store.allocate_app_session_id(&draft)?;
        store.enqueue_prompt(
            "draft:same-draft",
            farcaster_agents::Backend::Pi,
            project.path(),
            None,
            farcaster_agent_protocol::extensions::PromptMode::Normal,
            "first host only",
            &[],
        )?;
        Ok(())
    })?;
    second.state_store()?.with(|store| {
        assert!(store.load_drafts()?.is_empty());
        assert!(store.queued_prompts()?.is_empty());
        Ok(())
    })
}

#[test]
fn explicit_fixture_hosts_share_state_across_restart() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let path = project.path().join("state.sqlite3");
    let draft = farcaster_sessions::DraftSession::with_id(
        Some(farcaster_agents::Backend::Pi),
        "saved-draft".into(),
        project.path().into(),
    );
    let first = host_at(&path);
    let id = first
        .state_store()?
        .with(|store| store.allocate_app_session_id(&draft))?;
    drop(first);
    let restarted = host_at(&path);
    let drafts = restarted.state_store()?.with(|store| store.load_drafts())?;
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].app_session_id, id);
    Ok(())
}
