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
fn pool_snapshot_maps_to_persisted_child_and_projects_needs_input() {
    let mut parent = summary(Path::new("/sessions/parent.jsonl"));
    parent.id = "parent".into();
    let mut child = summary(Path::new("/sessions/child.jsonl"));
    child.id = "child".into();
    child.parent_session = Some(parent.id.clone());
    let snapshot = agents::WorkerSnapshot {
        id: "worker-1".into(),
        backend: child.harness.clone(),
        project: child.project.clone(),
        session_locator: Some(child.path.to_string_lossy().into_owned()),
        status: agents::WorkerStatus::NeedsInput,
        output: None,
        error: None,
        pending_input: None,
    };

    let sessions = [parent, child.clone()];
    let matched = session_for_worker_snapshot(&sessions, &snapshot)
        .expect("pool child should match its catalog row");
    let activity = AgentActivity::from_worker_snapshot(matched, &snapshot);

    assert_eq!(matched.id, child.id);
    assert_eq!(
        activity.lifecycle,
        crate::agent_activity::AgentLifecycle::NeedsInput
    );
    assert!(activity.limited);
}

#[test]
fn pool_snapshot_native_id_can_match_a_synthetic_child_locator() {
    let mut child = summary(Path::new("/locators/codex-cli/native-child"));
    child.id = "native-child".into();
    child.harness = "codex-cli".into();
    child.parent_session = Some("parent".into());
    let snapshot = agents::WorkerSnapshot {
        id: "worker-1".into(),
        backend: "codex-cli".into(),
        project: child.project.clone(),
        session_locator: Some("native-child".into()),
        status: agents::WorkerStatus::Idle,
        output: Some("done".into()),
        error: None,
        pending_input: None,
    };

    let matched = session_for_worker_snapshot(std::slice::from_ref(&child), &snapshot)
        .expect("native id should match the stored backend id");
    let activity = AgentActivity::from_worker_snapshot(matched, &snapshot);

    assert_eq!(
        activity.lifecycle,
        crate::agent_activity::AgentLifecycle::Completed(
            crate::agent_activity::AgentOutcome::Complete
        )
    );
}

#[test]
fn pool_snapshot_native_id_is_scoped_by_backend_and_project() {
    let mut wrong_project = summary(Path::new("/other/child"));
    wrong_project.id = "shared-child".into();
    wrong_project.harness = "codex-cli".into();
    wrong_project.parent_session = Some("other-parent".into());
    let mut wrong_backend = summary(Path::new("/project/pi-child"));
    wrong_backend.id = "shared-child".into();
    wrong_backend.parent_session = Some("pi-parent".into());
    let mut expected = summary(Path::new("/project/codex-child"));
    expected.id = "shared-child".into();
    expected.harness = "codex-cli".into();
    expected.parent_session = Some("codex-parent".into());
    let snapshot = agents::WorkerSnapshot {
        id: "worker-1".into(),
        backend: "codex-cli".into(),
        project: expected.project.clone(),
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
        backend: child.harness.clone(),
        project: child.project.clone(),
        session_locator: Some(child.path.to_string_lossy().into_owned()),
        status: agents::WorkerStatus::Idle,
        output: None,
        error: None,
        pending_input: None,
    };

    let activity = AgentActivity::from_worker_snapshot(&child, &snapshot);

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
    events: mpsc::Receiver<agents::WorkerEvent>,
}

impl agents::WorkerSessionFactory for CatalogWorkerFactory {
    fn create(&self, _: agents::WorkerLaunch) -> Result<Box<dyn agents::WorkerSession>, String> {
        let (sender, events) = mpsc::channel();
        *self.events.lock().map_err(|_| "events unavailable")? = Some(sender);
        Ok(Box::new(CatalogWorkerSession { events }))
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
        self.events.try_recv().ok()
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
            "pi".into(),
            factory.clone() as Arc<dyn agents::WorkerSessionFactory>,
        )]),
        "pi".into(),
        project.path().to_owned(),
        1,
    )
    .expect("worker pool");
    pool.start(agents::StartWorker {
        project: project.path().to_owned(),
        name: "child".into(),
        prompt: "work".into(),
        backend: "pi".into(),
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
