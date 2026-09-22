use std::path::PathBuf;

use farcaster_contracts::Backend;

use super::*;

#[test]
fn only_unsubmitted_drafts_can_change_project() {
    let mut draft = DraftSession::new(
        Some(Backend::Pi),
        "draft".into(),
        1,
        PathBuf::from("/first"),
        1,
    );
    assert!(draft.change_project(PathBuf::from("/second")));
    assert_eq!(draft.project, PathBuf::from("/second"));
    assert!(!draft.change_project(PathBuf::from("/second")));

    draft.submitted = true;
    assert!(!draft.change_project(PathBuf::from("/third")));
    assert_eq!(draft.project, PathBuf::from("/second"));
}

#[test]
fn drafts_without_a_backend_do_not_decode_as_pi() {
    let draft = serde_json::json!({"id": "missing", "project": "/project", "created_ms": 3});
    assert!(serde_json::from_value::<DraftSession>(draft).is_err());
}

#[test]
fn draft_backend_serialization_preserves_empty_and_legacy_names() {
    for (name, expected) in [
        ("", None),
        ("pi", Some(Backend::Pi)),
        ("opencode2", Some(Backend::OpenCode)),
    ] {
        let draft: DraftSession = serde_json::from_value(serde_json::json!({
            "id": "draft", "harness": name, "project": "/project", "created_ms": 1
        }))
        .expect("decode fixture draft");
        assert_eq!(draft.harness, expected);
        assert_eq!(
            serde_json::to_value(draft).expect("encode draft")["harness"],
            expected.map(Backend::as_str).unwrap_or("")
        );
    }
    assert!(
        serde_json::from_value::<DraftSession>(serde_json::json!({
            "id": "draft", "harness": "unknown", "project": "/project", "created_ms": 1
        }))
        .is_err()
    );
}

#[test]
fn accepted_draft_moves_to_its_discovered_session_without_losing_identity() {
    let path = PathBuf::from("/sessions/accepted.jsonl");
    let mut drafts = vec![DraftSession::new(
        Some(Backend::Pi),
        "accepted".into(),
        42,
        PathBuf::from("/project"),
        1,
    )];
    let mut associations = HashMap::new();
    assert_eq!(
        establish_submission(&mut associations, "draft:accepted", true, None),
        Some("accepted".into())
    );
    assert!(reconciliation_candidates(&associations, [path.as_path()].into_iter()).is_empty());
    assert_eq!(
        fill_session_association(&mut associations, "draft:accepted", Some(&path)),
        Some(path.clone())
    );
    assert!(update_persisted_submission(
        &mut drafts,
        "accepted",
        Some(&path)
    ));
    assert_eq!(drafts[0].app_session_id, 42);
    assert_eq!(submitted_draft_associations(&drafts), associations);
    assert_eq!(
        reconciliation_candidates(&associations, [path.as_path()].into_iter()),
        vec![("accepted".into(), path.clone())]
    );
}
