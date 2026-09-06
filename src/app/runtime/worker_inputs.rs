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
                system_notification_target: None,
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
        // Two choices are not necessarily an approval. Preserve the exact values.
        ExtensionUiRequest::Select {
            id: input.id,
            title: input.prompt,
            options: input.options,
            timeout: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
