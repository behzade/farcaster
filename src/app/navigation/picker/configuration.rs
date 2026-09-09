use super::*;
use crate::{
    protocol::Model,
    runtime::{ConfigurationStatus, HarnessAccessMode},
};

fn model_efforts<'a>(model: &'a Model, catalog: &'a [String]) -> &'a [String] {
    if model.reasoning {
        model.efforts.as_deref().unwrap_or(catalog)
    } else {
        &[]
    }
}

impl FarcasterApp {
    pub(in crate::app) fn open_runtime_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_picker(window, cx);
        self.show_chat_surface(window, cx);
        self.set_runtime_picker_open(true, window, cx);
    }

    pub(super) fn configuration_picker_rows(
        &self,
        scope: PickerScope,
        commands: &mut HashMap<String, PickerCommand>,
    ) -> Vec<PickerRow> {
        let identity = self.snapshot.session_identity();
        let current_model = identity.model;
        match scope {
            PickerScope::Harnesses => crate::agents::backend_statuses()
                .into_iter()
                .map(|backend| {
                    let detail = if !backend.available {
                        Some(format!(
                            "Not installed (expected: {})",
                            backend.program.display()
                        ))
                    } else {
                        (backend.id == self.active_harness()).then(|| "Current".into())
                    };
                    picker_row(
                        commands,
                        &format!("harness:{}", backend.id),
                        PickerCommand::SetHarness(backend.id.clone()),
                        AppIcon::for_harness(&backend.id),
                        &backend.name,
                        detail,
                        None,
                        &backend.id,
                    )
                    .disabled(!backend.available)
                })
                .collect(),
            PickerScope::Sandbox => self
                .snapshot
                .available_access_modes()
                .iter()
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
                        ConfigurationStatus::Loading => {
                            "Models are loading. Reopen this picker to refresh.".into()
                        }
                        ConfigurationStatus::Loaded => {
                            "No models were advertised by this harness.".into()
                        }
                    };
                    return vec![PickerRow::new(
                        "runtime:status",
                        AppIcon::List,
                        label,
                        None,
                        None,
                        "",
                    )];
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
                    .enumerate()
                    .map(|(index, provider)| {
                        picker_row(
                            commands,
                            &format!("provider:{index}"),
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
            PickerScope::Models(provider) => self
                .snapshot
                .models
                .iter()
                .filter(|model| model.provider == provider)
                .enumerate()
                .map(|(index, model)| {
                    let command = if model_efforts(model, &self.snapshot.thinking_levels).is_empty()
                    {
                        PickerCommand::SetRuntime {
                            model: model.clone(),
                            effort: None,
                        }
                    } else {
                        PickerCommand::OpenScope(PickerScope::Efforts(model.clone()))
                    };
                    picker_row(
                        commands,
                        &format!("model:{index}"),
                        command,
                        AppIcon::List,
                        &model.name,
                        Some(
                            if current_model.is_some_and(|current| {
                                current.id == model.id && current.provider == model.provider
                            }) {
                                format!("{} · Current", model.id)
                            } else {
                                model.id.clone()
                            },
                        ),
                        None,
                        &provider,
                    )
                })
                .collect(),
            PickerScope::Efforts(model) => model_efforts(&model, &self.snapshot.thinking_levels)
                .iter()
                .enumerate()
                .map(|(index, effort)| {
                    let selected = current_model.is_some_and(|current| {
                        current.id == model.id && current.provider == model.provider
                    }) && identity.effort == Some(effort.as_str());
                    picker_row(
                        commands,
                        &format!("effort:{index}"),
                        PickerCommand::SetRuntime {
                            model: model.clone(),
                            effort: Some(effort.clone()),
                        },
                        AppIcon::List,
                        effort,
                        Some(format!(
                            "{} / {}{}",
                            model.provider,
                            model.name,
                            if selected { " · Current" } else { "" }
                        )),
                        None,
                        "reasoning thinking effort",
                    )
                })
                .collect(),
            PickerScope::ArchivedSessions => {
                let mut sessions = self
                    .all_sessions
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

pub(super) fn selected_row(
    rows: &[PickerRow],
    commands: &HashMap<String, PickerCommand>,
    snapshot: &crate::runtime::RuntimeSnapshot,
    harness: Option<&str>,
) -> Option<usize> {
    if let Some(harness) = harness {
        return rows.iter().position(|row| {
            matches!(commands.get(&row.id), Some(PickerCommand::SetHarness(id)) if id == harness)
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
                    && effort
                        .as_deref()
                        .is_none_or(|effort| identity.effort == Some(effort))
            }
            _ => false,
        })
        .or_else(|| (!rows.is_empty()).then_some(0))
}

#[cfg(test)]
#[path = "configuration_tests.rs"]
mod tests;
