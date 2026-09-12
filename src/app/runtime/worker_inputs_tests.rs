use super::*;

use crate::agents::{
    CallerProfile, CallerRegistry, WorkerEvent, WorkerLaunch, WorkerSendMode, WorkerSession,
    WorkerSessionFactory,
};
use crate::app::runtime::tests::owner_without_process;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

struct InputWorkerFactory(Mutex<Option<mpsc::Receiver<WorkerEvent>>>);

impl WorkerSessionFactory for InputWorkerFactory {
    fn create(&self, _: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        Ok(Box::new(InputWorker(
            self.0.lock().unwrap().take().unwrap(),
        )))
    }
}

struct InputWorker(mpsc::Receiver<WorkerEvent>);
impl WorkerSession for InputWorker {
    fn send(&mut self, _: String, _: WorkerSendMode) -> Result<(), String> {
        Ok(())
    }
    fn respond(&mut self, _: agents::WorkerInputResponse) -> Result<(), String> {
        Ok(())
    }
    fn abort(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn poll(&mut self) -> Option<WorkerEvent> {
        self.0.try_recv().ok()
    }
    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn expired_child_lease_dismisses_the_dialog_and_late_answers_keep_parent_alive()
-> Result<(), String> {
    struct ParentTransport(std::rc::Rc<std::cell::Cell<usize>>);
    impl agents::SessionTransport for ParentTransport {
        fn send(&mut self, _: agents::SessionCommand) -> Result<String, String> {
            Ok("still-alive".into())
        }
        fn respond(&mut self, _: ExtensionUiResponse) -> Result<(), String> {
            Ok(())
        }
        fn poll(&mut self) -> Option<agents::SessionEvent> {
            None
        }
        fn close(&mut self) -> Result<(), String> {
            self.0.set(self.0.get() + 1);
            Ok(())
        }
    }
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let registry = CallerRegistry::shared();
    let parent = registry.issue(
        temp.path(),
        CallerProfile {
            backend: "pi".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    let path = temp.path().join("parent.jsonl");
    parent.bind(path.to_string_lossy());
    let caller = registry.resolve(parent.token())?;
    let (child_events, receiver) = mpsc::channel();
    let factory: Arc<dyn WorkerSessionFactory> =
        Arc::new(InputWorkerFactory(Mutex::new(Some(receiver))));
    let pool = agents::WorkerPool::new(
        std::collections::BTreeMap::from([("pi".into(), factory)]),
        "pi".into(),
        temp.path().into(),
        1,
    )?;
    let updates = pool.updates();
    pool.start(agents::StartWorker {
        project: temp.path().into(),
        name: "approval-child".into(),
        prompt: "work".into(),
        backend: "pi".into(),
        parent_session: caller.session,
        parent_worker_id: Some(caller.worker_id),
        context: agents::WorkerContext::Fresh,
        provider: None,
        model: None,
        effort: None,
        access_mode: agents::HarnessAccessMode::Auto,
    })?;
    let wait_update = || -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match updates.try_recv() {
                Ok(()) => return Ok(()),
                Err(async_channel::TryRecvError::Empty) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(2))
                }
                Err(error) => return Err(format!("worker update missing: {error}")),
            }
        }
    };
    wait_update()?;
    child_events
        .send(WorkerEvent::NeedsInput(agents::WorkerInput {
            id: "native-approval".into(),
            prompt: "Allow child?".into(),
            options: vec![],
            secret: false,
        }))
        .map_err(|e| e.to_string())?;
    wait_update()?;
    let (mut owner, events) = owner_without_process(temp.path().into());
    let closed = std::rc::Rc::new(std::cell::Cell::new(0));
    owner.process = Some(Box::new(ParentTransport(closed.clone())));
    owner.active_session = Some(path);
    Arc::make_mut(&mut owner.snapshot.conversation).running = true;
    owner.publish_child_inputs();
    let request = events
        .try_iter()
        .find_map(|event| match event {
            RuntimeEvent::ExtensionUi { request, .. } => Some(request),
            _ => None,
        })
        .ok_or("child dialog was not projected")?;
    let id = request.dialog_id().ok_or("not a dialog")?.to_owned();
    let mut ui = crate::app::extensions::ExtensionUiState::default();
    ui.apply(request);
    child_events
        .send(WorkerEvent::Settled {
            output: "finished".into(),
        })
        .map_err(|e| e.to_string())?;
    wait_update()?;
    owner.publish_child_inputs();
    let expired = events.try_iter().find_map(|event| match event {
        RuntimeEvent::ExtensionUiDismissed { id, .. } => Some(id),
        _ => None,
    });
    assert_eq!(
        expired.as_deref(),
        Some(id.as_str()),
        "lease expiration must dismiss the projected dialog"
    );
    ui.dismiss_dialog(&id);
    assert!(ui.dialog.is_none());
    for response in [
        ExtensionUiResponse::Cancelled {
            id: id.clone(),
            cancelled: true,
        },
        ExtensionUiResponse::Value {
            id: id.clone(),
            value: "late answer".into(),
        },
    ] {
        owner.apply_command(crate::app::runtime::RuntimeCommand::ExtensionResponse(
            response,
        ));
        assert_eq!(closed.get(), 0);
        assert!(owner.snapshot.connected);
        assert!(owner.snapshot.conversation.running);
        assert_eq!(
            owner
                .process
                .as_mut()
                .unwrap()
                .send(agents::SessionCommand::Abort)?,
            "still-alive"
        );
    }
    Ok(())
}

#[test]
fn child_questions_preserve_two_choices() {
    let request = child_interaction(agents::WorkerInput {
        id: "question".into(),
        prompt: "Choose".into(),
        options: vec!["First".into(), "Second".into()],
        secret: false,
    });
    assert!(matches!(request, ExtensionUiRequest::Select { options, .. }
            if options == ["First", "Second"]));
}
