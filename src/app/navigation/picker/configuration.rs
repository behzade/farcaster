use super::*;
use crate::agents::model_efforts;
use crate::{
    protocol::Model,
    runtime::{ConfigurationStatus, HarnessAccessMode},
};

impl FarcasterApp {
    pub(in crate::app) fn open_runtime_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.runtime_picker.open {
            self.set_runtime_picker_open(false, window, cx);
        }
        let mut path = vec![PickerScope::Providers];
        if let Some(model) = self.snapshot.session_identity().model.and_then(|selected| {
            self.snapshot
                .models
                .iter()
                .find(|model| model.id == selected.id && model.provider == selected.provider)
        }) {
            path.push(PickerScope::Models(model.provider.clone()));
            if !model_efforts(model, &self.snapshot.thinking_levels).is_empty() {
                path.push(PickerScope::Efforts(model.clone()));
            }
        }
        for scope in path {
            self.open_picker(scope, window, cx);
        }
    }

    pub(super) fn configuration_picker_rows(
        &self,
        scope: PickerScope,
        commands: &mut HashMap<String, PickerCommand>,
    ) -> Vec<PickerRow> {
        let identity = self.snapshot.session_identity();
        let current_model = identity.model;
        match scope {
            PickerScope::Harnesses => {
                let mut rows = crate::agents::backend_statuses()
                    .into_iter()
                    .map(|backend| {
                        let detail = if !backend.available {
                            Some(format!(
                                "Not installed (expected: {})",
                                backend.program.display()
                            ))
                        } else {
                            (Some(backend.id) == self.active_harness()
                                && self.active_profile_id().is_none())
                            .then(|| "Current".into())
                        };
                        picker_row(
                            commands,
                            &format!("harness:{}", backend.id),
                            PickerCommand::SetHarness(backend.id),
                            AppIcon::for_harness(backend.id),
                            &backend.name,
                            detail,
                            None,
                            backend.id.as_str(),
                        )
                        .disabled(!backend.available)
                    })
                    .collect::<Vec<_>>();
                for profile in self.settings.harness_profiles.list().unwrap_or_default() {
                    let current = self.active_profile_id().as_deref() == Some(profile.id.as_str());
                    let available = profile.is_selectable();
                    rows.push(
                        picker_row(
                            commands,
                            &format!("profile:{}", profile.id),
                            PickerCommand::SetHarnessProfile(profile.backend, profile.id.clone()),
                            AppIcon::for_harness(profile.backend),
                            &profile.name,
                            Some(if current {
                                "Current".into()
                            } else if available {
                                format!(
                                    "{} · {}",
                                    crate::agents::backend_display_name(profile.backend),
                                    profile.executable.display()
                                )
                            } else {
                                format!("Not installed: {}", profile.executable.display())
                            }),
                            None,
                            profile.backend.as_str(),
                        )
                        .disabled(!available),
                    );
                }
                rows
            }
            PickerScope::Sandbox => self
                .snapshot
                .available_access_modes()
                .iter()
                .filter(|_| self.snapshot.sandbox_controls_available())
                .enumerate()
                .map(|(index, mode)| {
                    let label = match mode {
                        HarnessAccessMode::Sandboxed => "On",
                        HarnessAccessMode::Full => "Off",
                        HarnessAccessMode::Auto => "Auto",
                    };
                    picker_row(
                        commands,
                        &format!("sandbox:{index}"),
                        PickerCommand::SetSandbox(*mode),
                        AppIcon::Shield,
                        label,
                        (*mode == self.snapshot.access_mode).then(|| "Current".into()),
                        None,
                        "sandbox access",
                    )
                })
                .collect(),
            PickerScope::Providers => {
                if self.snapshot.models.is_empty() {
                    let label = match &self.snapshot.configuration_status {
                        ConfigurationStatus::Failed(error) => {
                            format!("Models unavailable: {error}")
                        }
                        ConfigurationStatus::Loading => "Models are loading…".into(),
                        ConfigurationStatus::Loaded => {
                            "No models were advertised by this harness.".into()
                        }
                    };
                    return configuration_status_rows(
                        commands,
                        label,
                        matches!(
                            self.snapshot.configuration_status,
                            ConfigurationStatus::Failed(_)
                        ),
                    );
                }
                let mut providers = self
                    .snapshot
                    .models
                    .iter()
                    .map(|model| model.provider.clone())
                    .collect::<Vec<_>>();
                providers.sort();
                providers.dedup();
                providers
                    .into_iter()
                    .map(|provider| {
                        picker_row(
                            commands,
                            &format!("provider:{provider}"),
                            PickerCommand::OpenScope(PickerScope::Models(provider.clone())),
                            AppIcon::List,
                            &provider,
                            current_model
                                .is_some_and(|model| model.provider == provider)
                                .then(|| "Current".into()),
                            None,
                            "provider",
                        )
                    })
                    .collect()
            }
            PickerScope::Models(provider) => {
                let models = self
                    .snapshot
                    .models
                    .iter()
                    .filter(|model| model.provider == provider)
                    .collect::<Vec<_>>();
                if models.is_empty() {
                    let status = match &self.snapshot.configuration_status {
                        ConfigurationStatus::Loading => "Models are loading…".to_owned(),
                        ConfigurationStatus::Failed(error) => {
                            format!("Models unavailable: {error}")
                        }
                        ConfigurationStatus::Loaded => format!("No models from {provider}."),
                    };
                    return configuration_status_rows(
                        commands,
                        status,
                        matches!(
                            self.snapshot.configuration_status,
                            ConfigurationStatus::Failed(_)
                        ),
                    );
                }
                models
                    .into_iter()
                    .map(|model| {
                        let command =
                            if model_efforts(model, &self.snapshot.thinking_levels).is_empty() {
                                PickerCommand::SetRuntime {
                                    model: model.clone(),
                                    effort: None,
                                }
                            } else {
                                PickerCommand::OpenScope(PickerScope::Efforts(model.clone()))
                            };
                        let available = crate::agents::available_access_modes(
                            self.snapshot.harness,
                            Some(self.snapshot.catalog_model(model)),
                            self.snapshot.sandbox_adapter.as_deref(),
                        );
                        picker_row(
                            commands,
                            &format!("model:{}:{}", model.provider, model.id),
                            command,
                            AppIcon::List,
                            &model.name,
                            Some(if available.is_empty() {
                                "No access mode available".into()
                            } else if current_model.is_some_and(|current| {
                                current.id == model.id && current.provider == model.provider
                            }) {
                                format!("{} · Current", model.id)
                            } else {
                                model.id.clone()
                            }),
                            None,
                            &provider,
                        )
                        .disabled(available.is_empty())
                    })
                    .collect()
            }
            PickerScope::Efforts(model) => {
                if !self.snapshot.models.iter().any(|candidate| {
                    candidate.provider == model.provider && candidate.id == model.id
                }) {
                    return vec![picker_row(
                        commands,
                        "runtime:back-to-models",
                        PickerCommand::OpenScope(PickerScope::Models(model.provider)),
                        AppIcon::List,
                        "Model no longer available · Back to models",
                        None,
                        None,
                        "model removed back",
                    )];
                }
                effort_picker_rows(&self.snapshot, &model, commands)
            }
            PickerScope::ArchivedSessions => {
                let mut sessions = self
                    .sessions
                    .all
                    .iter()
                    .filter(|session| session.archived && session.parent_session.is_none())
                    .collect::<Vec<_>>();
                sessions.sort_by_key(|session| std::cmp::Reverse(session.modified));
                if sessions.is_empty() {
                    return vec![PickerRow::new(
                        "restore:empty",
                        AppIcon::ChatCircle,
                        "No archived sessions",
                        None,
                        None,
                        "",
                    )];
                }
                sessions
                    .into_iter()
                    .enumerate()
                    .map(|(index, session)| {
                        picker_row(
                            commands,
                            &format!("restore:{index}"),
                            PickerCommand::RestoreSession(session.path.clone()),
                            AppIcon::ChatCircle,
                            &session.title,
                            Some(session.project.display().to_string()),
                            None,
                            session.search_text(),
                        )
                    })
                    .collect()
            }
            _ => unreachable!("configuration picker scope"),
        }
    }
}

fn configuration_status_rows(
    commands: &mut HashMap<String, PickerCommand>,
    label: String,
    retry: bool,
) -> Vec<PickerRow> {
    let mut rows = Vec::new();
    if retry {
        rows.push(picker_row(
            commands,
            "runtime:retry",
            PickerCommand::RetryConfiguration,
            AppIcon::List,
            "Retry loading models",
            None,
            None,
            "reload retry models",
        ));
    }
    rows.push(
        PickerRow::new("runtime:status", AppIcon::List, label, None, None, "").disabled(true),
    );
    rows
}

fn effort_picker_rows(
    snapshot: &crate::runtime::RuntimeSnapshot,
    model: &Model,
    commands: &mut HashMap<String, PickerCommand>,
) -> Vec<PickerRow> {
    let model = snapshot.catalog_model(model);
    let identity = snapshot.session_identity();
    snapshot
        .effort_choices(model)
        .into_iter()
        .map(|effort| {
            let selected = identity.model.is_some_and(|current| {
                current.id == model.id && current.provider == model.provider
            }) && identity.effort == effort.as_deref();
            picker_row(
                commands,
                &effort.as_ref().map_or_else(
                    || "effort:<none>".to_owned(),
                    |effort| format!("effort:some:{effort}"),
                ),
                PickerCommand::SetRuntime {
                    model: model.clone(),
                    effort: effort.clone(),
                },
                AppIcon::List,
                effort.as_deref().unwrap_or("Default"),
                Some(format!(
                    "{} / {}{}",
                    model.provider,
                    model.name,
                    if selected { " · Current" } else { "" }
                )),
                None,
                "reasoning thinking effort variant default",
            )
        })
        .collect()
}

pub(super) fn selected_row(
    rows: &[PickerRow],
    commands: &HashMap<String, PickerCommand>,
    snapshot: &crate::runtime::RuntimeSnapshot,
    harness: Option<Backend>,
) -> Option<usize> {
    if let Some(harness) = harness {
        return rows.iter().position(|row| {
            matches!(commands.get(&row.id), Some(PickerCommand::SetHarness(id)) if *id == harness)
        });
    }

    let identity = snapshot.session_identity();
    let is_current_model = |model: &Model| {
        identity
            .model
            .is_some_and(|current| current.id == model.id && current.provider == model.provider)
    };
    rows.iter()
        .position(|row| match commands.get(&row.id) {
            Some(PickerCommand::SetSandbox(mode)) => *mode == snapshot.access_mode,
            Some(PickerCommand::OpenScope(PickerScope::Models(provider))) => {
                identity.provider == Some(provider.as_str())
            }
            Some(PickerCommand::OpenScope(PickerScope::Efforts(model))) => is_current_model(model),
            Some(PickerCommand::SetRuntime { model, effort }) => {
                is_current_model(model)
                    && (effort.as_deref() == identity.effort
                        || (effort.is_none()
                            && model_efforts(
                                snapshot.catalog_model(model),
                                &snapshot.thinking_levels,
                            )
                            .is_empty()))
            }
            _ => false,
        })
        .or_else(|| (!rows.is_empty()).then_some(0))
}

#[cfg(test)]
#[path = "configuration_tests.rs"]
mod tests;
