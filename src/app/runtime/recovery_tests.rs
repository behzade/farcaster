use super::*;
use crate::app::{extensions::ExtensionUiState, persistence::ComposerRecord};

fn interrupted_prompt(
    database: &Path,
    project: &Path,
    session: &Path,
) -> Result<(StateStore, i64, String), String> {
    let store = StateStore::open_at(database)?;
    let target = format!("session:{}", session.display());
    let id = store.enqueue_prompt(
        &target,
        "codex-cli",
        project,
        Some(session),
        PromptMode::Normal,
        "check the exact interrupted payload",
        &[],
    )?;
    store.save_composer_session(&ComposerRecord {
        target: target.clone(),
        text: "draft text must stay here".into(),
        cursor: 8,
        selection_start: 2,
        selection_end: 8,
        history: vec!["older draft".into()],
        attachments: Vec::new(),
    })?;
    store.begin_prompt(id)?;
    Ok((store, id, target))
}

#[test]
fn mark_delivered_from_ui_clears_the_blocker_without_touching_the_draft() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session.jsonl");
    let (store, _, target) = interrupted_prompt(&database, temp.path(), &session)?;
    drop(store);

    let mut reopened = StateStore::open_at(&database)?;
    let mut recovery = InterruptedPromptRecovery::recover(&reopened)?;
    let request = recovery
        .requests_for(&target, temp.path(), Some(&session))
        .into_iter()
        .next()
        .ok_or_else(|| "missing recovery request".to_owned())?;
    let id = request.dialog_id().unwrap().to_owned();
    let mut ui = ExtensionUiState::default();
    assert_eq!(
        ui.apply(request),
        crate::app::extensions::ExtensionEffect::DialogOpened
    );
    let response = ui
        .respond_value(&id, MARK_INTERRUPTED_PROMPT_DELIVERED.into())
        .ok_or_else(|| "recovery dialog did not accept the action".to_owned())?;
    assert!(matches!(
        recovery.resolve(
            &mut reopened,
            &target,
            temp.path(),
            Some(&session),
            &response,
        )?,
        RecoveryResolution::Resolved { .. }
    ));
    drop(recovery);
    drop(reopened);
    let reopened = StateStore::open_at(&database)?;
    assert!(reopened.unknown_prompts()?.is_empty());
    assert!(!reopened.has_queued_prompts_for(std::slice::from_ref(&session))?);
    assert_eq!(reopened.accepted_prompt_history(&session)?.len(), 1);
    let composers = reopened.load_composer_sessions()?;
    assert_eq!(composers.len(), 1);
    assert_eq!(composers[0].text, "draft text must stay here");
    assert_eq!(
        (
            composers[0].cursor,
            composers[0].selection_start,
            composers[0].selection_end
        ),
        (8, 2, 8)
    );
    Ok(())
}

#[test]
fn disposition_rejects_a_different_project_and_keeps_the_blocker() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session.jsonl");
    let (store, id, target) = interrupted_prompt(&database, temp.path(), &session)?;
    drop(store);
    let mut reopened = StateStore::open_at(&database)?;
    let mut recovery = InterruptedPromptRecovery::recover(&reopened)?;
    let response = ExtensionUiResponse::Value {
        id: format!("farcaster-recovery-{id}"),
        value: DISCARD_INTERRUPTED_PROMPT.into(),
    };
    assert!(
        recovery
            .resolve(
                &mut reopened,
                &target,
                &temp.path().join("other-project"),
                Some(&session),
                &response,
            )
            .is_err()
    );
    assert_eq!(reopened.unknown_prompts()?.len(), 1);
    assert!(reopened.has_queued_prompts_for(&[session])?);
    Ok(())
}

#[test]
fn cancel_reselect_and_reopen_keep_unknown_until_explicit_discard() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session.jsonl");
    let (store, id, target) = interrupted_prompt(&database, temp.path(), &session)?;
    drop(store);
    let mut reopened = StateStore::open_at(&database)?;
    let mut recovery = InterruptedPromptRecovery::recover(&reopened)?;
    assert!(matches!(
        recovery.resolve(
            &mut reopened,
            &target,
            temp.path(),
            Some(&session),
            &ExtensionUiResponse::Cancelled {
                id: format!("farcaster-recovery-{id}"),
                cancelled: true,
            },
        )?,
        RecoveryResolution::Pending
    ));
    assert_eq!(
        recovery
            .requests_for(&target, temp.path(), Some(&session))
            .len(),
        1,
        "reselecting must show the unresolved prompt again"
    );
    drop(recovery);
    drop(reopened);

    let mut reopened = StateStore::open_at(&database)?;
    let mut recovery = InterruptedPromptRecovery::recover(&reopened)?;
    assert_eq!(
        recovery
            .requests_for(&target, temp.path(), Some(&session))
            .len(),
        1
    );
    assert!(matches!(
        recovery.resolve(
            &mut reopened,
            &target,
            temp.path(),
            Some(&session),
            &ExtensionUiResponse::Value {
                id: format!("farcaster-recovery-{id}"),
                value: DISCARD_INTERRUPTED_PROMPT.into(),
            },
        )?,
        RecoveryResolution::Resolved { .. }
    ));
    drop(recovery);
    drop(reopened);
    let reopened = StateStore::open_at(&database)?;
    assert!(reopened.unknown_prompts()?.is_empty());
    assert!(!reopened.has_queued_prompts_for(&[session])?);
    Ok(())
}
