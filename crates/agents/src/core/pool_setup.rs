use std::{
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, atomic::Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::contract::{StartWorker, WorkerSnapshot, WorkerStatus};

use super::super::{
    CallerRegistry, WorkerAssignment,
    caller::WorkerParent,
    run,
    worker::{WorkerSendMode, WorkerSession},
};
use super::{ReservedStartResult, notify, snapshot};

pub(super) struct SetupCompletion(pub(super) Arc<(Mutex<bool>, Condvar)>);

impl Drop for SetupCompletion {
    fn drop(&mut self) {
        mark_setup_done(&self.0);
    }
}

pub(super) fn setup_is_cancelled(cancelled: &Mutex<bool>) -> Result<bool, String> {
    cancelled
        .lock()
        .map(|cancelled| *cancelled)
        .map_err(|_| "worker setup cancellation state is unavailable".to_owned())
}

pub(super) fn mark_setup_done(done: &Arc<(Mutex<bool>, Condvar)>) {
    let (done, changed) = &**done;
    if let Ok(mut done) = done.lock() {
        *done = true;
        changed.notify_all();
    }
}

pub(super) fn wait_for_setup(done: &Arc<(Mutex<bool>, Condvar)>) -> Result<(), String> {
    let (done, changed) = &**done;
    let mut done = done
        .lock()
        .map_err(|_| "worker setup state is unavailable".to_owned())?;
    while !*done {
        done = changed
            .wait(done)
            .map_err(|_| "worker setup state is unavailable".to_owned())?;
    }
    Ok(())
}

pub(super) fn validate_fixed_profile(
    assignment: &WorkerAssignment,
    profile: Option<&str>,
) -> Result<(), String> {
    if profile.is_some_and(|profile| profile != assignment.profile) {
        return Err(
            "a child's profile is fixed at creation; use a new child name for a different profile"
                .into(),
        );
    }
    Ok(())
}

pub(super) fn close_setup_session(
    session: &mut dyn WorkerSession,
    mut error: String,
) -> (String, bool) {
    match session.close() {
        Ok(()) => (error, true),
        Err(close_error) => {
            error.push_str(&format!("; worker cleanup failed: {close_error}"));
            (error, false)
        }
    }
}

pub(super) fn validate_start(request: &StartWorker) -> Result<(), String> {
    if !crate::valid_worker_name(&request.name) {
        return Err("worker name must be 1-48 ASCII letters, numbers, '-' or '_' and cannot start with punctuation".into());
    }
    if request.prompt.trim().is_empty() {
        return Err("worker prompt must not be empty".into());
    }
    if request.parent_session.trim().is_empty() {
        return Err("worker parent session must not be empty".into());
    }
    Ok(())
}

pub(super) fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("resolve worker project {}: {error}", path.display()))?;
    if !path.is_dir() {
        return Err(format!(
            "worker project is not a directory: {}",
            path.display()
        ));
    }
    Ok(path)
}

pub(super) fn worker_id(sequence: u64) -> Result<String, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is unavailable".to_owned())?
        .as_nanos();
    Ok(format!("worker-{nanos}-{sequence}"))
}

impl super::WorkerPool {
    pub(super) fn provision_reserved(
        &self,
        reserved: ReservedStartResult,
    ) -> Result<WorkerSnapshot, String> {
        let ReservedStartResult::Reserved(reserved) = reserved else {
            return Err("worker reservation unexpectedly reused an existing child".into());
        };
        let (
            factory,
            launch,
            shared,
            cleanup_confirmed,
            setup_done,
            setup_cancelled,
            setup_stop_requested,
        ) = {
            let state = self
                .inner
                .state
                .lock()
                .map_err(|_| "worker pool state is unavailable".to_owned())?;
            let record = state
                .records
                .get(&reserved.id)
                .ok_or("worker reservation is unavailable")?;
            (
                record.factory.clone(),
                record.launch.clone(),
                record.snapshot.clone(),
                record.cleanup_confirmed.clone(),
                record.setup_done.clone(),
                record.setup_cancelled.clone(),
                record.setup_stop_requested.clone(),
            )
        };
        let _setup_completion = SetupCompletion(setup_done);
        let slot = launch
            .slot
            .clone()
            .ok_or("worker reservation has no concurrency slot")?;
        if setup_stop_requested.load(Ordering::SeqCst)
            || setup_is_cancelled(&setup_cancelled)?
            || self.reservation_stopped(&reserved.id)?
        {
            self.finish_stopped_setup(&reserved.id, true);
            return Err("worker setup cancelled".into());
        }
        let created = {
            let cancelled = setup_cancelled
                .lock()
                .map_err(|_| "worker setup cancellation state is unavailable".to_owned())?;
            if *cancelled || setup_stop_requested.load(Ordering::SeqCst) {
                None
            } else {
                Some(factory.create(launch))
            }
        };
        let Some(created) = created else {
            self.finish_stopped_setup(&reserved.id, true);
            return Err("worker setup cancelled".into());
        };
        let mut session = match created {
            Ok(session) => session,
            Err(error) => {
                self.fail_reserved_setup(&reserved.id, error.clone(), true);
                return Err(error);
            }
        };
        if setup_stop_requested.load(Ordering::SeqCst) || self.reservation_stopped(&reserved.id)? {
            return self.cancel_reserved_session(&reserved.id, &mut *session);
        }
        if let Some(assignment) = reserved.assignment.clone()
            && let Err(error) = CallerRegistry::shared().set_assignment(&reserved.id, assignment)
        {
            let (error, cleanup) = close_setup_session(&mut *session, error);
            self.fail_reserved_setup(&reserved.id, error.clone(), cleanup);
            return Err(error);
        }
        if setup_stop_requested.load(Ordering::SeqCst) || self.reservation_stopped(&reserved.id)? {
            return self.cancel_reserved_session(&reserved.id, &mut *session);
        }
        let initial_send = {
            let cancelled = setup_cancelled
                .lock()
                .map_err(|_| "worker setup cancellation state is unavailable".to_owned())?;
            if *cancelled || setup_stop_requested.load(Ordering::SeqCst) {
                None
            } else {
                Some(session.send(reserved.prompt, WorkerSendMode::Prompt))
            }
        };
        let Some(initial_send) = initial_send else {
            return self.cancel_reserved_session(&reserved.id, &mut *session);
        };
        if let Err(error) = initial_send {
            let (error, cleanup) = close_setup_session(&mut *session, error);
            self.fail_reserved_setup(&reserved.id, error.clone(), cleanup);
            return Err(error);
        }

        loop {
            let mut state = self
                .inner
                .state
                .lock()
                .map_err(|_| "worker pool state is unavailable".to_owned())?;
            let record = state
                .records
                .get_mut(&reserved.id)
                .ok_or("worker reservation is unavailable")?;
            if setup_stop_requested.load(Ordering::SeqCst)
                || snapshot(record)?.status == WorkerStatus::Stopped
            {
                drop(state);
                return self.cancel_reserved_session(&reserved.id, &mut *session);
            }
            let pending = std::mem::take(&mut record.pending_messages);
            if !pending.is_empty() {
                drop(state);
                let pending_count = pending.len();
                for (index, message) in pending.into_iter().enumerate() {
                    let queued_send = {
                        let cancelled = setup_cancelled.lock().map_err(|_| {
                            "worker setup cancellation state is unavailable".to_owned()
                        })?;
                        if *cancelled || setup_stop_requested.load(Ordering::SeqCst) {
                            None
                        } else {
                            Some(session.send_peer_message(&message, WorkerSendMode::Queue))
                        }
                    };
                    let Some(queued_send) = queued_send else {
                        return self.cancel_reserved_session(&reserved.id, &mut *session);
                    };
                    if let Err(error) = queued_send {
                        let undelivered = pending_count - index;
                        let error = format!(
                            "{error}; delivery could not be confirmed for {undelivered} acknowledged queued message(s)"
                        );
                        let (error, cleanup) = close_setup_session(&mut *session, error);
                        self.fail_reserved_setup(&reserved.id, error.clone(), cleanup);
                        return Err(error);
                    }
                }
                continue;
            }
            if let Ok(mut current) = shared.lock() {
                current.status = WorkerStatus::Running;
            }
            let (commands, handle) = match run::spawn(
                &reserved.id,
                session,
                shared,
                slot.clone(),
                reserved.parent,
                self.inner.updates.clone(),
                cleanup_confirmed,
            ) {
                Ok(spawned) => spawned,
                Err(error) => {
                    drop(state);
                    self.fail_reserved_setup(&reserved.id, error.clone(), false);
                    return Err(error);
                }
            };
            record.commands = Some(commands);
            record.thread = Some(handle);
            let initial = snapshot(record)?;
            notify(&self.inner.updates);
            return Ok(initial);
        }
    }

    pub(super) fn cancel_reserved_session(
        &self,
        id: &str,
        session: &mut dyn WorkerSession,
    ) -> Result<WorkerSnapshot, String> {
        let (error, cleanup) = close_setup_session(session, "worker setup cancelled".into());
        if cleanup {
            self.finish_stopped_setup(id, true);
        } else {
            self.fail_reserved_setup(id, error.clone(), false);
        }
        Err(error)
    }

    fn reservation_stopped(&self, id: &str) -> Result<bool, String> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        let record = state
            .records
            .get(id)
            .ok_or("worker reservation is unavailable")?;
        Ok(snapshot(record)?.status == WorkerStatus::Stopped)
    }

    fn finish_stopped_setup(&self, id: &str, cleanup_confirmed: bool) {
        if let Ok(mut state) = self.inner.state.lock()
            && let Some(record) = state.records.get_mut(id)
        {
            if let Some(slot) = record.launch.slot.take() {
                slot.release();
            }
            record
                .cleanup_confirmed
                .store(cleanup_confirmed, Ordering::SeqCst);
            mark_setup_done(&record.setup_done);
        }
        notify(&self.inner.updates);
    }

    pub(super) fn fail_reserved_setup(&self, id: &str, error: String, cleanup_confirmed: bool) {
        let mut error = error;
        let parent = if let Ok(mut state) = self.inner.state.lock()
            && let Some(record) = state.records.get_mut(id)
        {
            if !record.pending_messages.is_empty() {
                let undelivered = record.pending_messages.len();
                record.pending_messages.clear();
                error = format!(
                    "{error}; delivery could not be confirmed for {undelivered} acknowledged queued message(s)"
                );
            }
            if let Some(slot) = record.launch.slot.take() {
                slot.release();
            }
            record
                .cleanup_confirmed
                .store(cleanup_confirmed, Ordering::SeqCst);
            if let Ok(mut current) = record.snapshot.lock() {
                current.status = WorkerStatus::Failed;
                current.error = Some(error.clone());
                current.pending_input = None;
            }
            mark_setup_done(&record.setup_done);
            record.launch.parent_worker_id.as_ref().map(|parent_id| {
                WorkerParent::new(
                    parent_id.clone(),
                    record.launch.project.clone(),
                    record.launch.worker_name.clone(),
                    record.launch.parent_session.clone(),
                )
            })
        } else {
            None
        };
        if let Some(parent) = parent {
            parent.report(format!("Worker failed to start: {error}"));
        }
        notify(&self.inner.updates);
    }
}
