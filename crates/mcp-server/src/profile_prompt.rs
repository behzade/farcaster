use std::{sync::mpsc, time::Duration};

use crate::{SharedStore, agents, storage, with_store};

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
    let execution = if choices.is_empty() {
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
        agents::WorkerExecution {
            harness,
            provider,
            model,
            effort: (!effort.trim().is_empty()).then_some(effort.trim().to_owned()),
            service_tier: (!service_tier.trim().is_empty())
                .then_some(service_tier.trim().to_owned()),
        }
    } else {
        let labels = choices
            .iter()
            .enumerate()
            .map(|(index, (harness, model, _))| {
                format!(
                    "{}. {} · {} · {}",
                    index + 1,
                    harness,
                    model.provider,
                    model.name
                )
            })
            .collect::<Vec<_>>();
        let chosen = choose(
            caller,
            format!("Worker '{profile}' has no available model. Choose a model:"),
            labels.clone(),
        )?;
        let index = labels
            .iter()
            .position(|label| label == &chosen)
            .ok_or("worker model choice is no longer available")?;
        let (harness, model, fallback_efforts) = &choices[index];
        let efforts = if model.reasoning {
            model.efforts.as_ref().unwrap_or(fallback_efforts)
        } else {
            &Vec::new()
        };
        let effort = if efforts.is_empty() {
            None
        } else {
            let mut options = vec!["Default effort".to_owned()];
            options.extend(efforts.iter().cloned());
            let selected = choose(
                caller,
                format!("Choose effort for worker '{profile}':"),
                options,
            )?;
            (selected != "Default effort").then_some(selected)
        };
        agents::WorkerExecution {
            harness: *harness,
            provider: model.provider.clone(),
            model: model.id.clone(),
            effort,
            service_tier: None,
        }
    };
    execution.validate()?;
    let access = super::workers::delegated_access_mode(caller.backend, caller.access_mode);
    if super::workers::child_access_mode(&execution, &caller.project, access, backends, catalogs)
        .is_none()
    {
        return Err("selected worker model is unavailable for this parent's access mode".into());
    }
    let save = choose(
        caller,
        format!("Use this model for worker '{profile}'?"),
        vec!["Save for this profile".into(), "Use once".into()],
    )?;
    if save == "Save for this profile" {
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
    } else if save != "Use once" {
        return Err("worker profile choice is no longer available".into());
    }
    Ok(execution)
}
