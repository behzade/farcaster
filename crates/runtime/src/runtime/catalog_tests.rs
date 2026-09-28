use crate::agents::Backend;
use std::path::Path;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use super::*;

fn summary(path: &Path) -> SessionSummary {
    SessionSummary::from_cached(
        "external".into(),
        path.to_path_buf(),
        PathBuf::from("/project"),
        "External".into(),
        String::new(),
        String::new(),
        None,
        SystemTime::now(),
        0,
        crate::sessions::UsageSummary::default(),
        false,
        false,
        String::new(),
    )
}

#[test]
fn import_preview_skips_sessions_already_in_the_catalog() {
    let known = HashSet::from([PathBuf::from("/sessions/known.jsonl")]);
    let known_session = summary(Path::new("/sessions/known.jsonl"));
    let mut unknown = summary(Path::new("/sessions/new.jsonl"));
    unknown.id = "new".into();

    let candidates = unknown_import_candidates(vec![known_session, unknown.clone()], &known);

    assert_eq!(candidates, vec![unknown]);
}

#[test]
fn import_preview_skips_nested_child_workers() {
    let parent = summary(Path::new("/sessions/parent.jsonl"));
    let mut child = summary(Path::new("/sessions/child.jsonl"));
    child.id = "child".into();
    child.parent_session = Some("parent".into());

    let candidates = unknown_import_candidates(vec![parent.clone(), child], &HashSet::new());

    assert_eq!(candidates, vec![parent]);
}

#[test]
fn pool_snapshot_maps_to_persisted_child_and_projects_needs_input() {
    let mut parent = summary(Path::new("/sessions/parent.jsonl"));
    parent.id = "parent".into();
    let mut child = summary(Path::new("/sessions/child.jsonl"));
    child.id = "child".into();
    child.parent_session = Some(parent.id.clone());
    let snapshot = agents::WorkerSnapshot {
        id: "worker-1".into(),
        backend: child.harness,
        project: child.project.clone(),
        session_key: Some(child.key()),
        session_locator: Some(child.path.to_string_lossy().into_owned()),
        status: agents::WorkerStatus::NeedsInput,
        output: None,
        error: None,
        pending_input: None,
    };

    let sessions = [parent, child.clone()];
    let matched = session_for_worker_snapshot(&sessions, &snapshot)
        .expect("pool child should match its catalog row");
    let activity =
        AgentActivity::from_worker_status(matched, snapshot.status, snapshot.output.is_some());

    assert_eq!(matched.id, child.id);
    assert_eq!(
        activity.lifecycle,
        crate::agent_activity::AgentLifecycle::NeedsInput
    );
    assert!(activity.limited);
}

#[test]
fn pool_snapshot_key_matches_a_synthetic_child_locator() {
    let mut child = summary(Path::new("/locators/codex-cli/native-child"));
    child.id = "native-child".into();
    child.harness = Backend::Codex;
    child.parent_session = Some("parent".into());
    let snapshot = agents::WorkerSnapshot {
        id: "worker-1".into(),
        backend: Backend::Codex,
        project: child.project.clone(),
        session_key: Some(child.key()),
        session_locator: Some("native-child".into()),
        status: agents::WorkerStatus::Idle,
        output: Some("done".into()),
        error: None,
        pending_input: None,
    };

    let matched = session_for_worker_snapshot(std::slice::from_ref(&child), &snapshot)
        .expect("canonical identity should match the stored child");
    let activity =
        AgentActivity::from_worker_status(matched, snapshot.status, snapshot.output.is_some());

    assert_eq!(
        activity.lifecycle,
        crate::agent_activity::AgentLifecycle::Completed(
            crate::agent_activity::AgentOutcome::Complete
        )
    );
}

#[test]
fn pool_snapshot_key_is_scoped_by_backend_and_project() {
    let mut wrong_project = summary(Path::new("/other/child"));
    wrong_project.id = "shared-child".into();
    wrong_project.harness = Backend::Codex;
    wrong_project.parent_session = Some("other-parent".into());
    let mut wrong_backend = summary(Path::new("/project/pi-child"));
    wrong_backend.id = "shared-child".into();
    wrong_backend.parent_session = Some("pi-parent".into());
    let mut expected = summary(Path::new("/project/codex-child"));
    expected.id = "shared-child".into();
    expected.harness = Backend::Codex;
    expected.parent_session = Some("codex-parent".into());
    let snapshot = agents::WorkerSnapshot {
        id: "worker-1".into(),
        backend: Backend::Codex,
        project: expected.project.clone(),
        session_key: Some(expected.key()),
        session_locator: Some("shared-child".into()),
        status: agents::WorkerStatus::Running,
        output: None,
        error: None,
        pending_input: None,
    };
    wrong_project.project = PathBuf::from("/other-project");
    let sessions = [wrong_project, wrong_backend, expected.clone()];

    let matched = session_for_worker_snapshot(&sessions, &snapshot).expect("scoped child");

    assert_eq!(matched.path, expected.path);
    assert_eq!(matched.harness, expected.harness);
    assert_eq!(matched.project, expected.project);
}

#[test]
fn idle_pool_snapshot_without_a_settled_output_stays_unknown() {
    let mut child = summary(Path::new("/sessions/child.jsonl"));
    child.parent_session = Some("parent".into());
    let snapshot = agents::WorkerSnapshot {
        id: "worker-1".into(),
        backend: child.harness,
        project: child.project.clone(),
        session_key: Some(child.key()),
        session_locator: Some(child.path.to_string_lossy().into_owned()),
        status: agents::WorkerStatus::Idle,
        output: None,
        error: None,
        pending_input: None,
    };

    let activity =
        AgentActivity::from_worker_status(&child, snapshot.status, snapshot.output.is_some());

    assert_eq!(
        activity.lifecycle,
        crate::agent_activity::AgentLifecycle::Unknown
    );
}

#[test]
fn normalized_native_failure_becomes_a_terminal_child_activity() {
    let child = serde_json::json!({
        "harness": "codex-cli",
        "id": "child",
        "path": "/locators/codex-cli/child",
        "project": "/project",
        "title": "Reviewer",
        "first_user_message": null,
        "parent_session": "parent",
        "message_count": null,
        "model": null,
        "thinking_level": null,
        "service_tier": null,
        "usage": null,
        "is_running": false,
        "outcome": "failed"
    });
    let metadata: agents::SessionMetadata =
        serde_json::from_value(child.clone()).expect("native child metadata");

    let activity = native_child_activity(&child, &metadata);

    assert_eq!(
        activity.lifecycle,
        crate::agent_activity::AgentLifecycle::Completed(
            crate::agent_activity::AgentOutcome::Failed
        )
    );
    assert!(activity.limited);
}

#[derive(Default)]
struct CatalogWorkerFactory {
    events: Mutex<Option<mpsc::Sender<agents::WorkerEvent>>>,
}

struct CatalogWorkerSession {
    identity: agents::CallerIdentity,
    events: mpsc::Receiver<agents::WorkerEvent>,
}

impl agents::WorkerSessionFactory for CatalogWorkerFactory {
    fn create(
        &self,
        launch: agents::WorkerLaunch,
    ) -> Result<Box<dyn agents::WorkerSession>, String> {
        let (sender, events) = mpsc::channel();
        *self.events.lock().map_err(|_| "events unavailable")? = Some(sender);
        let identity = agents::CallerRegistry::shared().issue_as_with_access(
            &launch.project,
            agents::CallerProfile {
                backend: Backend::Pi,
                provider: None,
                model: None,
                effort: None,
            },
            None,
            launch.worker_id,
            launch.worker_name,
            launch.parent_worker_id,
            launch.access_mode,
        )?;
        Ok(Box::new(CatalogWorkerSession { identity, events }))
    }
}

impl agents::WorkerSession for CatalogWorkerSession {
    fn send(&mut self, _: String, _: agents::WorkerSendMode) -> Result<(), String> {
        Ok(())
    }

    fn respond(&mut self, _: agents::WorkerInputResponse) -> Result<(), String> {
        Ok(())
    }

    fn abort(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn poll(&mut self) -> Option<agents::WorkerEvent> {
        let event = self.events.try_recv().ok()?;
        if let agents::WorkerEvent::SessionChanged { locator } = &event {
            self.identity.bind(locator.clone());
        }
        Some(event)
    }

    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

fn wait_for_worker_status(
    pool: &agents::WorkerPool,
    status: agents::WorkerStatus,
) -> agents::WorkerSnapshot {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if let Some(snapshot) = pool
            .snapshots()
            .expect("pool snapshots")
            .into_iter()
            .find(|snapshot| snapshot.status == status)
        {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "worker never reached {status:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn pool_run_status_projects_through_catalog_matching_into_child_activity() {
    let project = tempfile::tempdir().expect("project");
    let factory = Arc::new(CatalogWorkerFactory::default());
    let pool = agents::WorkerPool::new(
        BTreeMap::from([(
            Backend::Pi,
            factory.clone() as Arc<dyn agents::WorkerSessionFactory>,
        )]),
        Backend::Pi,
        project.path().to_owned(),
        1,
    )
    .expect("worker pool");
    pool.start(agents::StartWorker {
        project: project.path().to_owned(),
        name: "child".into(),
        prompt: "work".into(),
        backend: Backend::Pi,
        parent_session: "/sessions/parent.jsonl".into(),
        parent_worker_id: None,
        context: agents::WorkerContext::Fresh,
        provider: None,
        model: None,
        effort: None,
        access_mode: agents::HarnessAccessMode::Auto,
    })
    .expect("start worker");
    let events = factory
        .events
        .lock()
        .expect("events")
        .clone()
        .expect("worker event sender");
    let locator = project.path().join("child.jsonl");
    events
        .send(agents::WorkerEvent::SessionChanged {
            locator: locator.to_string_lossy().into_owned(),
        })
        .expect("session locator");
    events
        .send(agents::WorkerEvent::NeedsInput(agents::WorkerInput {
            id: "approval".into(),
            prompt: "Proceed?".into(),
            options: vec!["Yes".into()],
            secret: false,
        }))
        .expect("needs input");
    let snapshot = wait_for_worker_status(&pool, agents::WorkerStatus::NeedsInput);
    let mut child = summary(&locator);
    child.project = project.path().canonicalize().expect("canonical project");
    child.parent_session = Some("parent".into());
    let sessions = [child];
    let activities = worker_activities(&sessions, vec![snapshot.clone()]);
    let matched = session_for_worker_snapshot(&sessions, &snapshot).expect("catalog child");
    let activity = activities
        .get(&crate::agent_activity::agent_activity_key(&matched.path))
        .expect("production catalog activity");
    assert_eq!(
        activity.lifecycle,
        crate::agent_activity::AgentLifecycle::NeedsInput
    );

    events
        .send(agents::WorkerEvent::Settled {
            output: "done".into(),
        })
        .expect("settled");
    let snapshot = wait_for_worker_status(&pool, agents::WorkerStatus::Idle);
    assert_eq!(snapshot.output.as_deref(), Some("done"));
    let activities = worker_activities(&sessions, vec![snapshot.clone()]);
    assert_eq!(
        activities
            .get(&crate::agent_activity::agent_activity_key(&matched.path))
            .expect("settled catalog activity")
            .lifecycle,
        crate::agent_activity::AgentLifecycle::Completed(
            crate::agent_activity::AgentOutcome::Complete
        )
    );
}

#[test]
fn canonical_activity_is_profile_exact_in_either_row_order_and_after_merge() {
    let mut base = summary(Path::new("/catalog/base/native"));
    base.harness = Backend::Codex;
    base.id = "same-native-id".into();
    base.app_session_id = 41;
    base.parent_app_session_id = Some(10);
    let mut named = base.clone();
    named.app_session_id = 99;
    named.profile_id = Some("named".into());
    named.path = "/catalog/named/native".into();
    let mut snapshot = agents::WorkerSnapshot {
        id: "worker".into(),
        backend: Backend::Codex,
        project: base.project.clone(),
        session_key: Some(base.key()),
        session_locator: Some(base.id.clone()),
        status: agents::WorkerStatus::NeedsInput,
        output: None,
        error: None,
        pending_input: None,
    };
    for merged in [false, true] {
        if merged {
            base.app_session_id = 84;
            snapshot.session_key = Some(base.key());
        }
        for rows in [[base.clone(), named.clone()], [named.clone(), base.clone()]] {
            let activities = worker_activities(&rows, vec![snapshot.clone()]);
            assert_eq!(activities.len(), 1);
            assert_eq!(activities.values().next().unwrap().session_path, base.path);
            assert_eq!(
                activities.values().next().unwrap().lifecycle,
                crate::agent_activity::AgentLifecycle::NeedsInput
            );
        }
    }
    snapshot.session_key = Some(crate::sessions::SessionKey::App(
        crate::sessions::AppSessionId::new(41).unwrap(),
    ));
    assert!(worker_activities(&[base.clone(), named.clone()], vec![snapshot.clone()]).is_empty());
    snapshot.session_key = None;
    assert!(worker_activities(&[base.clone(), named], vec![snapshot.clone()]).is_empty());
    snapshot.session_key = Some(base.key());
    assert!(worker_activities(&[base.clone(), base], vec![snapshot]).is_empty());
}

#[test]
fn locator_activity_keeps_profile_scope_when_native_id_and_path_match() {
    let mut base = summary(Path::new("/native/shared.jsonl"));
    base.parent_session = Some("parent".into());
    let mut named = base.clone();
    named.profile_id = Some("named".into());
    let snapshot = agents::WorkerSnapshot {
        id: "worker".into(),
        backend: named.harness,
        project: named.project.clone(),
        session_key: Some(named.key()),
        session_locator: Some(named.id.clone()),
        status: agents::WorkerStatus::Running,
        output: None,
        error: None,
        pending_input: None,
    };
    for rows in [[base.clone(), named.clone()], [named.clone(), base.clone()]] {
        assert_eq!(
            session_for_worker_snapshot(&rows, &snapshot)
                .unwrap()
                .profile_id
                .as_deref(),
            Some("named")
        );
    }
}
