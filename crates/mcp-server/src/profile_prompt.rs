use std::{sync::mpsc, time::Duration};

use crate::{SharedStore, agents, storage, with_store};
use farcaster_agent_protocol::extensions::{
    WORKER_MODEL_REQUEST_PREFIX, WorkerModelChoice, WorkerModelRequest, WorkerModelSelection,
};

fn choose(
    caller: &agents::CallerContext,
    title: String,
    options: Vec<String>,
) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    let lease = agents::CallerRegistry::shared().request_profile_input(
        caller,
        agents::WorkerInput {
            id: "profile-choice".into(),
            prompt: title,
            options,
            secret: false,
        },
        tx,
    )?;
    let response = rx
        .recv_timeout(Duration::from_secs(300))
        .map_err(|_| "worker profile selection timed out".to_owned())?;
    drop(lease);
    if response.cancel {
        return Err("worker creation cancelled".into());
    }
    response
        .value
        .ok_or_else(|| "worker creation cancelled".into())
}

pub(super) fn configure(
    profile: &str,
    caller: &agents::CallerContext,
    catalogs: &[storage::CachedConfigurationCatalog],
    backends: &[agents::Backend],
    store: &SharedStore,
) -> Result<agents::WorkerExecution, String> {
    let choices = catalogs
        .iter()
        .filter(|entry| entry.project == caller.project && backends.contains(&entry.harness))
        .flat_map(|entry| {
            entry.catalog.models.iter().filter_map(move |model| {
                let execution = agents::WorkerExecution {
                    harness: entry.harness,
                    provider: model.provider.clone(),
                    model: model.id.clone(),
                    effort: None,
                    service_tier: None,
                };
                let access =
                    super::workers::delegated_access_mode(caller.backend, caller.access_mode);
                super::workers::child_access_mode(
                    &execution,
                    &caller.project,
                    access,
                    backends,
                    catalogs,
                )
                .map(|_| (entry.harness, model.clone(), entry.catalog.efforts.clone()))
            })
        })
        .collect::<Vec<_>>();
    let (execution, save_choice) = if choices.is_empty() {
        let harnesses = backends.iter().map(ToString::to_string).collect::<Vec<_>>();
        if harnesses.is_empty() {
            return Err("no worker harness is installed".into());
        }
        let selected = choose(
            caller,
            format!("Worker '{profile}' needs a model. Choose a harness:"),
            harnesses,
        )?;
        let harness = selected
            .parse::<agents::Backend>()
            .map_err(|_| "worker harness choice is invalid")?;
        let provider = choose(caller, "Provider ID".into(), Vec::new())?
            .trim()
            .to_owned();
        let model = choose(caller, "Model ID".into(), Vec::new())?
            .trim()
            .to_owned();
        let effort = choose(
            caller,
            "Effort ID (leave blank for default)".into(),
            Vec::new(),
        )?;
        let service_tier = if harness == agents::Backend::Cursor {
            choose(
                caller,
                "Service tier ID (leave blank for default)".into(),
                Vec::new(),
            )?
        } else {
            String::new()
        };
        (
            agents::WorkerExecution {
                harness,
                provider,
                model,
                effort: (!effort.trim().is_empty()).then_some(effort.trim().to_owned()),
                service_tier: (!service_tier.trim().is_empty())
                    .then_some(service_tier.trim().to_owned()),
            },
            None,
        )
    } else {
        let request = WorkerModelRequest {
            profile: profile.to_owned(),
            choices: choices
                .iter()
                .map(|(harness, model, fallback_efforts)| WorkerModelChoice {
                    harness: *harness,
                    provider: model.provider.clone(),
                    id: model.id.clone(),
                    name: model.name.clone(),
                    efforts: if model.reasoning {
                        model.efforts.as_ref().unwrap_or(fallback_efforts).clone()
                    } else {
                        Vec::new()
                    },
                })
                .collect(),
        };
        let selected = choose(
            caller,
            format!(
                "{WORKER_MODEL_REQUEST_PREFIX}{}",
                serde_json::to_string(&request).map_err(|error| error.to_string())?
            ),
            vec!["Choose model".into()],
        )?;
        let selected: WorkerModelSelection =
            serde_json::from_str(&selected).map_err(|_| "worker model choice is invalid")?;
        let (harness, model, _) = choices
            .get(selected.choice)
            .ok_or("worker model choice is no longer available")?;
        let available = &request.choices[selected.choice].efforts;
        if selected
            .effort
            .as_ref()
            .is_some_and(|effort| !available.contains(effort))
        {
            return Err("worker effort choice is unavailable".into());
        }
        (
            agents::WorkerExecution {
                harness: *harness,
                provider: model.provider.clone(),
                model: model.id.clone(),
                effort: selected.effort,
                service_tier: None,
            },
            Some(selected.save),
        )
    };
    execution.validate()?;
    let access = super::workers::delegated_access_mode(caller.backend, caller.access_mode);
    if super::workers::child_access_mode(&execution, &caller.project, access, backends, catalogs)
        .is_none()
    {
        return Err("selected worker model is unavailable for this parent's access mode".into());
    }
    let save = match save_choice {
        Some(save) => save,
        None => match choose(
            caller,
            format!("Use this model for worker '{profile}'?"),
            vec!["Save for this profile".into(), "Use once".into()],
        )?
        .as_str()
        {
            "Save for this profile" => true,
            "Use once" => false,
            _ => return Err("worker profile choice is no longer available".into()),
        },
    };
    if save {
        with_store(store, |store| {
            let mut profiles = store.load_worker_profiles()?;
            let selected = profiles
                .profiles
                .iter_mut()
                .find(|item| item.name == profile && item.enabled)
                .ok_or("worker profile changed during selection")?;
            selected.models = vec![execution.clone()];
            store.save_worker_profiles(&profiles)
        })?;
    }
    Ok(execution)
}
