use super::*;

impl RuntimeOwner {
    pub(super) fn publish_child_inputs(&self) {
        if self.process.is_none() {
            return;
        }
        let Some(path) = self.active_session.as_deref() else {
            return;
        };
        let (backend, locator) = agents::external_session_identity(path)
            .unwrap_or_else(|| (self.harness.as_str(), path.to_string_lossy().into_owned()));
        for input in
            agents::CallerRegistry::shared().take_child_inputs(&self.project, backend, &locator)
        {
            let _ = self.event_tx.send(RuntimeEvent::ExtensionUi {
                generation: self.process_generation,
                request: child_interaction(input),
                system_notification_target: self.attention_target(),
            });
        }
    }

    pub(super) fn respond_to_child_input(&mut self, response: &ExtensionUiResponse) -> bool {
        let (id, value, cancel) = match response {
            ExtensionUiResponse::Value { id, value } => (id, Some(value.clone()), false),
            ExtensionUiResponse::Confirmed { id, confirmed } => (
                id,
                Some(if *confirmed { "allow" } else { "decline" }.into()),
                false,
            ),
            ExtensionUiResponse::Cancelled { id, .. } => (id, None, true),
        };
        if !agents::is_child_input_id(id) {
            return false;
        }
        let response = agents::WorkerInputResponse {
            id: id.clone(),
            value,
            cancel,
        };
        if let Err(error) = agents::CallerRegistry::shared().respond_to_child_input(response) {
            self.fail(error);
        }
        true
    }
}

fn child_interaction(input: agents::WorkerInput) -> ExtensionUiRequest {
    if input.options.is_empty() {
        ExtensionUiRequest::Input {
            id: input.id,
            title: input.prompt,
            placeholder: None,
            timeout: None,
        }
    } else {
        ExtensionUiRequest::Select {
            id: input.id,
            title: input.prompt,
            options: input.options,
            timeout: None,
        }
    }
}

#[cfg(test)]
#[path = "worker_inputs_tests.rs"]
mod tests;
