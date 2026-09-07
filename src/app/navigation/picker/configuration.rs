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
        let path = runtime_picker_path(
            &self.snapshot.models,
            self.snapshot.session_identity().model,
            &self.snapshot.thinking_levels,
        );
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
            PickerScope::Sandbox => crate::agents::supported_access_modes(&self.snapshot.harness)
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
                            &session.search_text(),
                        )
                    })
                    .collect()
            }
            _ => unreachable!("configuration picker scope"),
        }
    }
}

fn runtime_picker_path(
    models: &[Model],
    selected: Option<&Model>,
    catalog: &[String],
) -> Vec<PickerScope> {
    let mut path = vec![PickerScope::Providers];
    let Some(model) = selected.and_then(|selected| {
        models
            .iter()
            .find(|model| model.id == selected.id && model.provider == selected.provider)
    }) else {
        return path;
    };
    path.push(PickerScope::Models(model.provider.clone()));
    if !model_efforts(model, catalog).is_empty() {
        path.push(PickerScope::Efforts(model.clone()));
    }
    path
}

pub(super) fn selected_row(
    rows: &[PickerRow],
    commands: &HashMap<String, PickerCommand>,
    snapshot: &crate::runtime::RuntimeSnapshot,
) -> Option<usize> {
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
mod tests {
    use super::*;

    #[test]
    fn runtime_entry_keeps_models_and_providers_behind_current_effort() {
        let mut model = Model {
            id: "id".into(),
            name: "Model".into(),
            provider: "provider".into(),
            context_window: 0,
            reasoning: true,
            efforts: Some(vec!["high".into()]),
        };
        assert_eq!(
            runtime_picker_path(&[model.clone()], Some(&model), &[]),
            vec![
                PickerScope::Providers,
                PickerScope::Models("provider".into()),
                PickerScope::Efforts(model.clone())
            ]
        );
        let old = model.clone();
        model.efforts = Some(vec![]);
        assert_eq!(
            runtime_picker_path(&[model.clone()], Some(&old), &[]),
            vec![
                PickerScope::Providers,
                PickerScope::Models("provider".into())
            ]
        );
        assert_eq!(
            runtime_picker_path(&[], Some(&model), &[]),
            vec![PickerScope::Providers]
        );
        assert_eq!(
            runtime_picker_path(&[model], None, &[]),
            vec![PickerScope::Providers]
        );
    }

    #[test]
    fn runtime_selection_matches_model_identity_and_effort() {
        let model = Model {
            id: "id".into(),
            name: "Model".into(),
            provider: "provider".into(),
            context_window: 0,
            reasoning: true,
            efforts: None,
        };
        let snapshot = crate::runtime::RuntimeSnapshot {
            prefill_model: Some(model.clone()),
            prefill_thinking_level: Some("high".into()),
            ..Default::default()
        };
        let mut commands = HashMap::new();
        let rows = ["low", "high"].map(|effort| {
            picker_row(
                &mut commands,
                effort,
                PickerCommand::SetRuntime {
                    model: model.clone(),
                    effort: Some(effort.into()),
                },
                AppIcon::List,
                effort,
                None,
                None,
                "",
            )
        });
        assert_eq!(selected_row(&rows, &commands, &snapshot), Some(1));
        assert_eq!(selected_row(&[], &commands, &snapshot), None);
    }

    #[test]
    fn effort_choices_respect_each_models_limits() {
        let catalog = vec!["low".into(), "high".into()];
        let mut model = Model {
            id: "test".into(),
            name: "Test".into(),
            provider: "test".into(),
            context_window: 0,
            reasoning: false,
            efforts: None,
        };
        assert!(model_efforts(&model, &catalog).is_empty());
        model.reasoning = true;
        assert_eq!(model_efforts(&model, &catalog), catalog);
        model.efforts = Some(vec!["high".into()]);
        assert_eq!(model_efforts(&model, &catalog), &["high"]);
        model.efforts = Some(vec![]);
        assert!(model_efforts(&model, &catalog).is_empty());
    }
}
