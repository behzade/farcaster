use super::*;
use crate::agents::{WorkerInput, WorkerInputResponse};

const INPUT_PREFIX: &str = "farcaster-worker-input-";

pub(crate) fn is_child_input_id(id: &str) -> bool {
    id.starts_with(INPUT_PREFIX)
}

pub(super) struct PendingInput {
    parent_id: String,
    input: WorkerInput,
    original_id: String,
    delivered: bool,
    responses: mpsc::Sender<WorkerInputResponse>,
}

pub(in crate::modules::agents::core) struct InputLease {
    registry: CallerRegistry,
    id: String,
}

impl Drop for InputLease {
    fn drop(&mut self) {
        if let Ok(mut inputs) = self.registry.inputs.lock() {
            inputs.retain(|pending| pending.input.id != self.id);
        }
    }
}

impl CallerRegistry {
    pub(crate) fn take_child_inputs(
        &self,
        project: &Path,
        backend: &str,
        session: &str,
    ) -> Vec<WorkerInput> {
        let project = canonical_project(project);
        let Ok(callers) = self.callers.lock() else {
            return Vec::new();
        };
        let Some(parent) = callers.values().find(|caller| {
            caller.project == project
                && caller.backend == backend
                && caller.session.as_deref() == Some(session)
                && caller.parent_worker_id.is_none()
        }) else {
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

    pub(crate) fn respond_to_child_input(
        &self,
        mut response: WorkerInputResponse,
    ) -> Result<(), String> {
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

    pub(in crate::modules::agents::core) fn request_child_input(
        &self,
        child: &WorkerParent,
        mut input: WorkerInput,
        responses: mpsc::Sender<WorkerInputResponse>,
    ) -> Result<InputLease, String> {
        let callers = self
            .callers
            .lock()
            .map_err(|_| "worker caller registry is unavailable")?;
        let mut parent_id = &child.id;
        let parent = loop {
            let parent = callers
                .values()
                .find(|caller| caller.worker_id == *parent_id && caller.project == child.project)
                .ok_or("parent worker is unavailable")?;
            match &parent.parent_worker_id {
                Some(id) => parent_id = id,
                None => break parent,
            }
        };
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

#[cfg(test)]
#[path = "inputs_tests.rs"]
mod tests;
