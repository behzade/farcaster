use super::*;
use crate::Backend;
use crate::{WorkerInput, WorkerInputResponse};

const INPUT_PREFIX: &str = "farcaster-worker-input-";

pub fn is_child_input_id(id: &str) -> bool {
    id.starts_with(INPUT_PREFIX)
}

pub(super) struct PendingInput {
    pub(super) parent_id: String,
    input: WorkerInput,
    original_id: String,
    delivered: bool,
    responses: mpsc::Sender<WorkerInputResponse>,
    pub(super) parent_session: CallerSession,
}

pub(super) struct ExpiredInput {
    pub(super) parent: CallerSession,
    pub(super) parent_id: String,
    id: String,
}

pub struct InputLease {
    registry: CallerRegistry,
    id: String,
}

impl Drop for InputLease {
    fn drop(&mut self) {
        if let Ok(mut inputs) = self.registry.inputs.lock() {
            let Some(index) = inputs
                .iter()
                .position(|pending| pending.input.id == self.id)
            else {
                return;
            };
            let pending = inputs.remove(index);
            drop(inputs);
            if !pending.delivered {
                return;
            }
            if let Ok(mut expired) = self.registry.expired_inputs.lock() {
                expired.push(ExpiredInput {
                    parent: pending.parent_session.clone(),
                    parent_id: pending.parent_id.clone(),
                    id: pending.input.id,
                });
            }
            if let Ok(callers) = self.registry.callers.lock()
                && let Ok(retired) = self.registry.retired_inboxes.lock()
                && let Some(parent) = callers
                    .values()
                    .chain(retired.values())
                    .find(|caller| caller.worker_id == pending.parent_id)
                && let Some(wake) = &parent.wake
            {
                wake.unpark();
            }
        }
    }
}

impl CallerRegistry {
    pub fn request_profile_input(
        &self,
        caller: &CallerContext,
        mut input: WorkerInput,
        responses: mpsc::Sender<WorkerInputResponse>,
    ) -> Result<InputLease, String> {
        let callers = self
            .callers
            .lock()
            .map_err(|_| "worker caller registry is unavailable")?;
        let parent = callers
            .values()
            .find(|registered| {
                registered.worker_id == caller.worker_id
                    && registered.project == caller.project
                    && registered.session.as_deref() == Some(&caller.session)
                    && registered.parent_worker_id.is_none()
            })
            .ok_or("caller is no longer available for worker profile selection")?;
        let parent_session = parent
            .session_key()
            .ok_or("caller has no persistent session")?;
        let original_id = input.id;
        input.id = new_identity("farcaster-worker-input");
        let id = input.id.clone();
        self.inputs
            .lock()
            .map_err(|_| "worker input registry is unavailable")?
            .push(PendingInput {
                parent_id: parent.worker_id.clone(),
                input,
                original_id,
                delivered: false,
                responses,
                parent_session,
            });
        if let Some(wake) = &parent.wake {
            wake.unpark();
        }
        Ok(InputLease {
            registry: self.clone(),
            id,
        })
    }

    pub fn take_child_inputs(
        &self,
        project: &Path,
        backend: Backend,
        session: &str,
    ) -> Vec<WorkerInput> {
        self.take_child_inputs_for_session(&NativeSessionIdentity {
            project: canonical_project(project),
            harness: backend,
            profile_id: None,
            id: session.into(),
        })
    }

    pub fn take_child_inputs_for_session(
        &self,
        identity: &NativeSessionIdentity,
    ) -> Vec<WorkerInput> {
        let Ok(callers) = self.callers.lock() else {
            return Vec::new();
        };
        let Ok(retired) = self.retired_inboxes.lock() else {
            return Vec::new();
        };
        let Some(parent) = input_parent(&callers, &retired, identity) else {
            return Vec::new();
        };
        let Ok(mut inputs) = self.inputs.lock() else {
            return Vec::new();
        };
        inputs
            .iter_mut()
            .filter_map(|pending| {
                if pending.parent_id != parent.worker_id || pending.delivered {
                    return None;
                }
                pending.delivered = true;
                Some(pending.input.clone())
            })
            .collect()
    }

    pub fn replay_child_inputs_for_session(&self, identity: &NativeSessionIdentity) {
        if let Ok(mut inputs) = self.inputs.lock() {
            for pending in inputs
                .iter_mut()
                .filter(|pending| pending.parent_session.native == *identity)
            {
                pending.delivered = false;
            }
        }
    }

    pub fn respond_to_child_input(&self, mut response: WorkerInputResponse) -> Result<(), String> {
        let mut inputs = self
            .inputs
            .lock()
            .map_err(|_| "worker input registry is unavailable")?;
        let index = inputs
            .iter()
            .position(|pending| pending.input.id == response.id)
            .ok_or("worker input request is no longer available")?;
        let pending = inputs.remove(index);
        response.id = pending.original_id;
        pending
            .responses
            .send(response)
            .map_err(|_| "worker is no longer available".into())
    }

    pub fn take_expired_child_inputs(
        &self,
        project: &Path,
        backend: Backend,
        session: &str,
    ) -> Vec<String> {
        self.take_expired_child_inputs_for_session(&NativeSessionIdentity {
            project: canonical_project(project),
            harness: backend,
            profile_id: None,
            id: session.into(),
        })
    }

    pub fn take_expired_child_inputs_for_session(
        &self,
        identity: &NativeSessionIdentity,
    ) -> Vec<String> {
        let Ok(callers) = self.callers.lock() else {
            return Vec::new();
        };
        let Ok(retired) = self.retired_inboxes.lock() else {
            return Vec::new();
        };
        let Some(parent) = input_parent(&callers, &retired, identity) else {
            return Vec::new();
        };
        let Ok(mut expired) = self.expired_inputs.lock() else {
            return Vec::new();
        };
        let mut ids = Vec::new();
        let mut index = 0;
        while index < expired.len() {
            if expired[index].parent_id == parent.worker_id {
                ids.push(expired.remove(index).id);
            } else {
                index += 1;
            }
        }
        ids
    }

    pub(in crate::core) fn request_child_input(
        &self,
        child: &WorkerParent,
        mut input: WorkerInput,
        responses: mpsc::Sender<WorkerInputResponse>,
    ) -> Result<InputLease, String> {
        let callers = self
            .callers
            .lock()
            .map_err(|_| "worker caller registry is unavailable")?;
        let retired = self
            .retired_inboxes
            .lock()
            .map_err(|_| "retired worker registry is unavailable")?;
        let mut parent_id = &child.id;
        let mut direct = true;
        let parent = loop {
            let parent = callers
                .values()
                .chain(retired.values())
                .find(|caller| caller.worker_id == *parent_id && caller.project == child.project)
                .or_else(|| direct.then(|| child.find(&callers)).flatten())
                .or_else(|| direct.then(|| child.find(&retired)).flatten())
                .ok_or("parent worker is unavailable")?;
            direct = false;
            match &parent.parent_worker_id {
                Some(id) => parent_id = id,
                None => break parent,
            }
        };
        let parent_session = parent
            .session_key()
            .ok_or("parent worker has no persistent session")?;
        let original_id = input.id;
        input.id = new_identity("farcaster-worker-input");
        input.prompt = format!("Child {}\n\n{}", child.child_name, input.prompt);
        let id = input.id.clone();
        self.inputs
            .lock()
            .map_err(|_| "worker input registry is unavailable")?
            .push(PendingInput {
                parent_id: parent.worker_id.clone(),
                input,
                original_id,
                delivered: false,
                responses,
                parent_session,
            });
        if let Some(wake) = &parent.wake {
            wake.unpark();
        }
        Ok(InputLease {
            registry: self.clone(),
            id,
        })
    }
}

fn input_parent<'a>(
    callers: &'a HashMap<String, RegisteredCaller>,
    retired: &'a HashMap<String, RegisteredCaller>,
    identity: &NativeSessionIdentity,
) -> Option<&'a RegisteredCaller> {
    unique_caller(callers.values().chain(retired.values()).filter(|caller| {
        caller.parent_worker_id.is_none()
            && caller
                .session_key()
                .is_some_and(|key| key.native == *identity)
    }))
}

#[cfg(test)]
#[path = "inputs_tests.rs"]
mod tests;
