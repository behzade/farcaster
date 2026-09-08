use super::{WorkerExecution, WorkerProfile, WorkerProfiles};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyTasks {
    tasks: Vec<LegacyTask>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyTask {
    name: String,
    specified: WorkerExecution,
    guided: WorkerExecution,
    independent: WorkerExecution,
}

pub(super) fn migrate(value: serde_json::Value) -> Result<WorkerProfiles, String> {
    let legacy: LegacyTasks = serde_json::from_value(value).map_err(|error| error.to_string())?;
    // An empty list deliberately disables worker creation.
    if legacy.tasks.is_empty() {
        return Ok(WorkerProfiles { profiles: vec![] });
    }
    let mut result = WorkerProfiles::default();
    let mut names = std::collections::BTreeSet::new();
    for task in legacy.tasks {
        if !super::super::super::valid_worker_name(&task.name)
            || !names.insert(task.name.to_ascii_lowercase())
        {
            return Err(format!(
                "invalid or duplicate legacy worker task: {}",
                task.name
            ));
        }
        for (level, execution, old_model, old_effort) in [
            ("specified", task.specified, "gpt-5.6-luna", "high"),
            ("guided", task.guided, "gpt-5.6-sol", "medium"),
            ("independent", task.independent, "gpt-6-astra", "medium"),
        ] {
            execution.validate()?;
            let old_default = execution.harness == "pi"
                && execution.provider == "openai-codex"
                && execution.model == old_model
                && execution.effort.as_deref() == Some(old_effort);
            if old_default
                || result
                    .profiles
                    .iter()
                    .any(|profile| profile.models.contains(&execution))
            {
                continue;
            }
            // Keep distinct custom routes, even when long task names share a prefix.
            let mut suffix = format!("_{level}");
            let mut counter = 1;
            let name = loop {
                let candidate = format!(
                    "{}{}",
                    &task.name[..task.name.len().min(48 - suffix.len())],
                    suffix
                );
                if !result
                    .profiles
                    .iter()
                    .any(|profile| profile.name.eq_ignore_ascii_case(&candidate))
                {
                    break candidate;
                }
                suffix = format!("_{level}_{counter}");
                counter += 1;
            };
            result.profiles.push(WorkerProfile {
                name,
                description: format!("Saved custom worker for {} ({level}). Edit this description to explain when to use it.", task.name),
                models: vec![execution],
            });
        }
    }
    Ok(result)
}
