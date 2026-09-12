use super::*;
use serde_json::json;

#[test]
fn submission_uses_caller_project_and_returns_validated_artifact() {
    let project = tempfile::tempdir().unwrap();
    let caller = crate::agents::CallerContext {
        worker_id: "worker".into(),
        worker_name: "Worker".into(),
        project: project.path().into(),
        session: "session".into(),
        backend: "test".into(),
        provider: None,
        model: None,
        effort: None,
        access_mode: crate::agents::HarnessAccessMode::Auto,
        parent_worker_id: None,
    };
    let params = || json!({"title":"Review this", "items":[{"path":"missing.rs", "note":"Inspect deletion", "start_line":1,"end_line":3}]});
    let result = submit(&caller, serde_json::from_value(params()).unwrap()).unwrap();
    assert_eq!(
        result["farcaster_review"]["project"],
        json!(project.path().canonicalize().unwrap())
    );
    assert_eq!(result["farcaster_review"]["review"], params());
    assert!(!project.path().join("missing.rs").exists());
    for field in ["project", "session", "sessionId"] {
        let mut value = params();
        value[field] = json!("injected");
        assert!(serde_json::from_value::<Params>(value).is_err());
    }
    let mut value = params();
    value["items"][0]["path"] = json!("../escape");
    assert!(submit(&caller, serde_json::from_value(value).unwrap()).is_err());
}
