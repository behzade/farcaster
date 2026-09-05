//! User-owned worker routing. Task labels describe delegated work, not agent personas.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerJudgment {
    Specified,
    #[default]
    Guided,
    Independent,
}

impl WorkerJudgment {
    pub(crate) const ALL: [Self; 3] = [Self::Specified, Self::Guided, Self::Independent];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Specified => "specified",
            Self::Guided => "guided",
            Self::Independent => "independent",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerExecution {
    pub(crate) harness: String,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) effort: Option<String>,
}

impl WorkerExecution {
    pub(crate) fn validate(&self) -> Result<(), String> {
        let fields = [
            ("harness", self.harness.as_str()),
            ("provider", self.provider.as_str()),
            ("model", self.model.as_str()),
        ];
        for (name, value) in fields
            .into_iter()
            .chain(self.effort.as_deref().map(|value| ("effort", value)))
        {
            if value.is_empty() || value != value.trim() || value.chars().any(char::is_control) {
                return Err(format!(
                    "worker {name} must be nonempty, without surrounding whitespace or control characters"
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerTaskDefinition {
    pub(crate) name: String,
    pub(crate) specified: WorkerExecution,
    pub(crate) guided: WorkerExecution,
    pub(crate) independent: WorkerExecution,
}

impl WorkerTaskDefinition {
    pub(crate) fn new(name: String) -> Self {
        let execution = |model: &str, effort: &str| WorkerExecution {
            harness: "pi".into(),
            provider: "openai-codex".into(),
            model: model.into(),
            effort: Some(effort.into()),
        };
        Self {
            name,
            specified: execution("gpt-5.6-luna", "high"),
            guided: execution("gpt-5.6-sol", "medium"),
            independent: execution("gpt-6-astra", "medium"),
        }
    }

    pub(crate) fn execution(&self, judgment: WorkerJudgment) -> &WorkerExecution {
        match judgment {
            WorkerJudgment::Specified => &self.specified,
            WorkerJudgment::Guided => &self.guided,
            WorkerJudgment::Independent => &self.independent,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerTasks {
    pub(crate) tasks: Vec<WorkerTaskDefinition>,
}

impl Default for WorkerTasks {
    fn default() -> Self {
        Self {
            tasks: ["read", "implement", "review"]
                .map(|name| WorkerTaskDefinition::new(name.into()))
                .into(),
        }
    }
}

impl WorkerTasks {
    pub(crate) fn validate(&self) -> Result<(), String> {
        let mut names = BTreeSet::new();
        for task in &self.tasks {
            if !super::super::valid_worker_name(&task.name) {
                return Err("task names must be 1–48 ASCII letters, numbers, '-' or '_' and start with a letter or number".into());
            }
            if !names.insert(task.name.to_ascii_lowercase()) {
                return Err(format!("duplicate worker task: {}", task.name));
            }
            for judgment in WorkerJudgment::ALL {
                task.execution(judgment)
                    .validate()
                    .map_err(|error| format!("{} / {}: {error}", task.name, judgment.label()))?;
            }
        }
        Ok(())
    }

    pub(crate) fn resolve(
        &self,
        task: &str,
        judgment: WorkerJudgment,
    ) -> Result<WorkerAssignment, String> {
        self.validate()?;
        let definition = self
            .tasks
            .iter()
            .find(|definition| definition.name == task)
            .ok_or_else(|| {
                format!("unknown worker task: {task}; refresh the tool schema for configured tasks")
            })?;
        Ok(WorkerAssignment {
            task: task.into(),
            judgment,
            execution: definition.execution(judgment).clone(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct WorkerAssignment {
    pub(crate) task: String,
    pub(crate) judgment: WorkerJudgment,
    pub(crate) execution: WorkerExecution,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_is_independent_of_parent_and_task_kind() {
        let tasks = WorkerTasks::default();
        for task in ["read", "implement", "review"] {
            assert_eq!(
                tasks
                    .resolve(task, WorkerJudgment::Specified)
                    .unwrap()
                    .execution
                    .model,
                "gpt-5.6-luna"
            );
            assert_eq!(
                tasks
                    .resolve(task, WorkerJudgment::Guided)
                    .unwrap()
                    .execution
                    .effort
                    .as_deref(),
                Some("medium")
            );
            assert_eq!(
                tasks
                    .resolve(task, WorkerJudgment::Independent)
                    .unwrap()
                    .execution
                    .model,
                "gpt-6-astra"
            );
            assert_eq!(
                tasks
                    .resolve(task, WorkerJudgment::Independent)
                    .unwrap()
                    .execution
                    .effort
                    .as_deref(),
                Some("medium")
            );
        }
    }

    #[test]
    fn empty_is_deliberate_and_invalid_definitions_fail_closed() {
        let mut tasks = WorkerTasks { tasks: Vec::new() };
        assert!(tasks.validate().is_ok());
        assert!(tasks.resolve("implement", WorkerJudgment::Guided).is_err());
        tasks = WorkerTasks::default();
        tasks.tasks[1].name = "READ".into();
        assert!(tasks.validate().is_err());
        tasks = WorkerTasks::default();
        tasks.tasks[0].specified.model.clear();
        assert!(tasks.validate().is_err());
    }
}
