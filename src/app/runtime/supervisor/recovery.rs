use super::*;
use crate::app::runtime::recovery::RecoveryResolution;
use std::path::Path;

impl Supervisor {
    pub(super) fn publish_recovery_statuses(&mut self) {
        for prompt in self.recovery.prompts() {
            publish_session_status_if_changed(
                &self.event_tx,
                &mut self.published_statuses,
                &prompt.target,
                prompt.session.clone(),
                "Needs input",
            );
        }
    }

    pub(super) fn publish_selected_recovery_dialogs(&mut self) {
        let selection = (
            self.generation,
            self.selected.clone(),
            self.selected_project.clone(),
            self.selected_session.clone(),
        );
        if self.published_recovery_selection.as_ref() == Some(&selection) {
            return;
        }
        self.published_recovery_selection = Some(selection);
        publish_recovery_dialogs(
            &self.recovery,
            &self.event_tx,
            self.generation,
            &self.selected,
            &self.selected_project,
            self.selected_session.as_deref(),
        );
    }

    pub(super) fn recovery_target_for_snapshot(
        &self,
        key: &str,
        snapshot: &RuntimeSnapshot,
    ) -> Option<&str> {
        let session = snapshot
            .live_session
            .as_deref()
            .or(snapshot.selected_session.as_deref());
        self.recovery.target_for(key, &snapshot.project, session)
    }

    pub(super) fn resolve_recovery_response(&mut self, response: &ExtensionUiResponse) -> bool {
        let Some(state) = self.catalog_state.as_mut() else {
            return false;
        };
        match self.recovery.resolve(
            state,
            &self.selected,
            &self.selected_project,
            self.selected_session.as_deref(),
            response,
        ) {
            Ok(RecoveryResolution::NotRecovery) => false,
            Ok(RecoveryResolution::Pending) => {
                self.published_recovery_selection = None;
                true
            }
            Ok(RecoveryResolution::Resolved { target, session }) => {
                let still_pending = !self
                    .recovery
                    .requests_for(
                        &self.selected,
                        &self.selected_project,
                        self.selected_session.as_deref(),
                    )
                    .is_empty();
                let status = if still_pending {
                    "Needs input"
                } else {
                    self.latest
                        .get(&self.selected)
                        .map_or("Done", |snapshot| semantic_status(snapshot))
                };
                publish_session_status_if_changed(
                    &self.event_tx,
                    &mut self.published_statuses,
                    &target,
                    session,
                    status,
                );
                true
            }
            Err(error) => {
                self.published_recovery_selection = None;
                let _ = self.event_tx.send(RuntimeEvent::SystemNotification {
                    title: "Farcaster: Prompt recovery failed".into(),
                    body: error,
                    target: self
                        .selected_session
                        .clone()
                        .map(|session| (session, self.selected_project.clone())),
                });
                true
            }
        }
    }
}

fn publish_recovery_dialogs(
    recovery: &crate::app::runtime::recovery::InterruptedPromptRecovery,
    event_tx: &UiEventSender,
    generation: u64,
    target: &str,
    project: &Path,
    session: Option<&Path>,
) {
    for request in recovery.requests_for(target, project, session) {
        let _ = event_tx.send(RuntimeEvent::ExtensionUi {
            generation,
            request,
            system_notification_target: None,
        });
    }
}
