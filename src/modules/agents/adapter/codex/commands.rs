use super::*;

const COMMANDS: &[(&str, &str)] = &[
    ("compact", "Compact this session's context"),
    (
        "review",
        "Review changes: /review [branch NAME | commit SHA]",
    ),
    ("model", "List models or select one: /model [ID [EFFORT]]"),
    (
        "permissions",
        "List permission profiles or select one: /permissions [ID]",
    ),
    ("status", "Show model, context usage, and account limits"),
];

pub(super) fn catalog(skills: &Skills) -> Vec<Value> {
    COMMANDS
        .iter()
        .map(|(name, description)| {
            json!({
                "name": name, "description": description, "source": "extension",
            })
        })
        .chain(skills.commands())
        .collect()
}

#[derive(Default)]
pub(super) struct State {
    pub(super) usage: Option<WorkerUsage>,
    permissions: Option<String>,
    models: Vec<Value>,
}

impl State {
    pub(super) fn new(access: crate::agents::HarnessAccessMode) -> Self {
        let permissions = match access {
            crate::agents::HarnessAccessMode::Full => "full access",
            crate::agents::HarnessAccessMode::Sandboxed => "sandboxed, user approvals",
            crate::agents::HarnessAccessMode::Auto => "sandboxed, automatic approvals",
        };
        Self {
            permissions: Some(permissions.into()),
            ..Self::default()
        }
    }
}

pub(super) struct Request {
    submission: Option<String>,
    step: Step,
}

enum Step {
    Models {
        choice: Vec<String>,
        rows: Vec<Value>,
    },
    Permissions {
        choice: Option<String>,
        rows: Vec<Value>,
    },
    SetModel {
        model: Value,
        effort: Option<String>,
    },
    SetPermissions(String),
    Status,
}

impl CodexWorkerSession {
    pub(super) fn observe_command_settings(&mut self, settings: &Value) {
        if let Some(model) = settings["model"].as_str() {
            self.model = Some(model.to_owned());
            self.caller_identity.select_model(
                settings["modelProvider"].as_str().unwrap_or("openai"),
                model,
            );
        }
        if let Some(effort) = settings.get("effort") {
            self.effort = effort.as_str().map(str::to_owned);
        }
        if let Some(profile) = settings
            .pointer("/activePermissionProfile/id")
            .and_then(Value::as_str)
        {
            self.command_state.permissions = Some(profile.to_owned());
        }
    }

    pub(super) fn dispatch_command(
        &mut self,
        message: &str,
        mode: WorkerSendMode,
        images: &[crate::protocol::PromptImage],
        submission: Option<String>,
    ) -> Result<bool, String> {
        let mut words = message.split_whitespace();
        let Some(name) = words.next().and_then(|word| word.strip_prefix('/')) else {
            return Ok(false);
        };
        if !COMMANDS.iter().any(|(command, _)| *command == name) {
            return Ok(false);
        }
        if mode != WorkerSendMode::Prompt
            || self.activity() != WorkerActivityState::Idle
            || self.manual_compaction
        {
            return Err(format!("Run /{name} when the current turn has finished"));
        }
        if !images.is_empty() {
            return Err(format!("/{name} does not accept images"));
        }
        let args: Vec<String> = words.map(str::to_owned).collect();
        match name {
            "compact" => {
                if !args.is_empty() {
                    return Err("Usage: /compact".into());
                }
                self.compact()?;
                if let Some(submission) = submission {
                    self.prompt_requests
                        .insert(CodexRequestId::Number(self.next_id), submission);
                }
            }
            "review" => {
                let target = match args.as_slice() {
                    [] => json!({"type":"uncommittedChanges"}),
                    [kind, value] if kind == "branch" => {
                        json!({"type":"baseBranch", "branch":value})
                    }
                    [kind, value] if kind == "commit" => json!({"type":"commit", "sha":value}),
                    _ => return Err("Usage: /review [branch NAME | commit SHA]".into()),
                };
                let id = self.request(
                    "review/start",
                    json!({"threadId":self.thread_id, "delivery":"inline", "target":target}),
                )?;
                self.pending.insert(id.clone(), PendingRequest::StartTurn);
                if let Some(submission) = submission {
                    self.prompt_requests.insert(id, submission);
                }
                self.caller_identity
                    .set_activity(WorkerActivityState::Starting);
            }
            "model" => {
                if args.len() > 2 {
                    return Err("Usage: /model [ID [EFFORT]]".into());
                }
                self.command_request(
                    "model/list",
                    json!({}),
                    submission,
                    Step::Models {
                        choice: args,
                        rows: Vec::new(),
                    },
                )?;
            }
            "permissions" => {
                if args.len() > 1 {
                    return Err("Usage: /permissions [ID]".into());
                }
                self.command_request(
                    "permissionProfile/list",
                    json!({"cwd":self.project}),
                    submission,
                    Step::Permissions {
                        choice: args.into_iter().next(),
                        rows: Vec::new(),
                    },
                )?;
            }
            "status" => {
                if !args.is_empty() {
                    return Err("Usage: /status".into());
                }
                self.command_request(
                    "account/rateLimits/read",
                    json!({}),
                    submission,
                    Step::Status,
                )?;
            }
            _ => unreachable!(),
        }
        Ok(true)
    }

    fn command_request(
        &mut self,
        method: &str,
        params: Value,
        submission: Option<String>,
        step: Step,
    ) -> Result<(), String> {
        let id = self.request(method, params)?;
        self.pending
            .insert(id, PendingRequest::Command(Request { submission, step }));
        self.caller_identity
            .set_activity(WorkerActivityState::Starting);
        Ok(())
    }

    pub(super) fn command_response(&mut self, request: Request, result: Result<Value, String>) {
        let submission = request.submission.clone();
        let outcome = self.apply_command_response(request, result);
        if matches!(outcome, Ok(None)) {
            return;
        }
        if let Some(id) = submission {
            self.prompt_acks
                .push_back((id, outcome.as_ref().map(|_| ()).map_err(Clone::clone)));
        }
        self.caller_identity.set_activity(WorkerActivityState::Idle);
        match outcome {
            Ok(Some(output)) => {
                self.events.push_back(WorkerEvent::Settled { output });
            }
            Err(error) => self.events.push_back(WorkerEvent::Failed(error)),
            Ok(None) => unreachable!(),
        }
    }

    fn apply_command_response(
        &mut self,
        request: Request,
        result: Result<Value, String>,
    ) -> Result<Option<String>, String> {
        let Request { submission, step } = request;
        if matches!(step, Step::Status) {
            return Ok(Some(self.command_status(result)));
        }
        let result = result?;
        let output = match step {
            Step::Models { choice, mut rows } => {
                append_page(&mut rows, &result)?;
                if let Some(cursor) = result["nextCursor"].as_str() {
                    self.command_request(
                        "model/list",
                        json!({"cursor":cursor}),
                        submission,
                        Step::Models { choice, rows },
                    )?;
                    return Ok(None);
                }
                self.command_state.models = rows.iter().filter_map(model_metadata).collect();
                if let Some(id) = choice.first() {
                    let model = self
                        .command_state
                        .models
                        .iter()
                        .find(|model| model["id"] == *id)
                        .ok_or_else(|| format!("Unknown Codex model: {id}"))?
                        .clone();
                    let supports_effort = |effort: &String| {
                        model["efforts"]
                            .as_array()
                            .is_some_and(|levels| levels.iter().any(|level| level == effort))
                    };
                    let effort = choice
                        .get(1)
                        .cloned()
                        .or_else(|| {
                            self.effort
                                .as_ref()
                                .filter(|effort| supports_effort(effort))
                                .cloned()
                        })
                        .or_else(|| model["defaultEffort"].as_str().map(str::to_owned));
                    if let Some(effort) = &effort {
                        if !supports_effort(effort) {
                            return Err(format!("Model {id} does not support effort {effort}"));
                        }
                    }
                    let mut params = json!({"threadId":self.thread_id, "model":id});
                    if let Some(effort) = &effort {
                        params["effort"] = json!(effort);
                    }
                    self.command_request(
                        "thread/settings/update",
                        params,
                        submission,
                        Step::SetModel { model, effort },
                    )?;
                    return Ok(None);
                }
                let mut lines = vec![format!(
                    "Model: {}\nUse /model ID [EFFORT].",
                    self.model.as_deref().unwrap_or("harness default")
                )];
                for model in &self.command_state.models {
                    let efforts = model["efforts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ");
                    lines.push(format!(
                        "{} — {}{}",
                        model["id"].as_str().unwrap_or_default(),
                        model["name"].as_str().unwrap_or_default(),
                        if efforts.is_empty() {
                            String::new()
                        } else {
                            format!(" ({efforts})")
                        }
                    ));
                }
                lines.join("\n")
            }
            Step::Permissions { choice, mut rows } => {
                append_page(&mut rows, &result)?;
                if let Some(cursor) = result["nextCursor"].as_str() {
                    self.command_request(
                        "permissionProfile/list",
                        json!({"cwd":self.project,"cursor":cursor}),
                        submission,
                        Step::Permissions { choice, rows },
                    )?;
                    return Ok(None);
                }
                if let Some(id) = choice {
                    if !rows
                        .iter()
                        .any(|profile| profile["id"] == id && profile["allowed"] == true)
                    {
                        return Err(format!(
                            "Permission profile is unavailable or disallowed: {id}"
                        ));
                    }
                    self.command_request(
                        "thread/settings/update",
                        json!({"threadId":self.thread_id,"permissions":id}),
                        submission,
                        Step::SetPermissions(id),
                    )?;
                    return Ok(None);
                }
                let mut lines = vec![format!(
                    "Permissions: {}\nUse /permissions ID for this session.",
                    self.command_state
                        .permissions
                        .as_deref()
                        .unwrap_or("harness launch settings")
                )];
                for profile in rows {
                    lines.push(format!(
                        "{}{} — {}",
                        profile["id"].as_str().unwrap_or_default(),
                        if profile["allowed"] == true {
                            ""
                        } else {
                            " (disallowed)"
                        },
                        profile["description"].as_str().unwrap_or_default()
                    ));
                }
                lines.join("\n")
            }
            Step::SetModel { model, effort } => {
                let id = model["id"].as_str().ok_or("Codex model has no id")?;
                self.select_model(model["provider"].as_str().unwrap_or("openai"), id)?;
                self.effort = effort;
                if let Some(effort) = self.effort.clone() {
                    self.caller_identity.select_effort(&effort);
                }
                // A collaboration preset must not override the explicit model choice.
                if let Some(mode) = self.collaboration_mode.as_mut() {
                    mode["settings"]["model"] = json!(id);
                    mode["settings"]["reasoning_effort"] = json!(self.effort);
                }
                let efforts = model["efforts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                let modes = self.collaboration_modes.values().map(|configuration| {
                    json!({"id":configuration["mode"], "name":configuration["mode"], "configuration":configuration})
                }).collect();
                self.events.push_back(WorkerEvent::Activity(
                    WorkerActivity::ConfigurationChanged {
                        models: self.command_state.models.clone(),
                        efforts,
                        modes,
                        selected_model: Some(model.clone()),
                        selected_effort: self.effort.clone(),
                    },
                ));
                format!(
                    "Model: {id}\nReasoning: {}",
                    self.effort.as_deref().unwrap_or("harness default")
                )
            }
            Step::SetPermissions(id) => {
                self.command_state.permissions = Some(id.clone());
                format!("Permission profile for this session: {id}")
            }
            Step::Status => unreachable!(),
        };
        Ok(Some(output))
    }

    fn command_status(&self, limits: Result<Value, String>) -> String {
        let mut lines = vec![format!(
            "Model: {}\nReasoning: {}\nPermissions: {}",
            self.model.as_deref().unwrap_or("harness default"),
            self.effort.as_deref().unwrap_or("harness default"),
            self.command_state
                .permissions
                .as_deref()
                .unwrap_or("harness launch settings")
        )];
        if let Some(usage) = self.command_state.usage {
            lines.push(format!(
                "Context: {} / {} tokens\nSession tokens: {}",
                usage.turn.total(),
                usage.context_window,
                usage.session.total()
            ));
        } else {
            lines.push("Context usage: not yet reported".into());
        }
        match limits {
            Ok(result) => {
                let limits = &result["rateLimits"];
                for key in ["primary", "secondary"] {
                    if let Some(used) = limits[key]["usedPercent"].as_f64() {
                        lines.push(format!("{key} limit: {used}% used"));
                    }
                }
                if limits.is_null() {
                    lines.push("Account limits: not reported".into());
                }
            }
            Err(error) => lines.push(format!("Account limits unavailable: {error}")),
        }
        lines.join("\n")
    }
}

fn append_page(rows: &mut Vec<Value>, result: &Value) -> Result<(), String> {
    rows.extend(
        result["data"]
            .as_array()
            .ok_or("Codex returned an invalid command catalog")?
            .iter()
            .cloned(),
    );
    Ok(())
}

fn model_metadata(model: &Value) -> Option<Value> {
    let id = model["id"].as_str()?;
    Some(
        json!({"id":id,"name":model["displayName"].as_str().unwrap_or(id),
        "provider":model["modelProvider"].as_str().unwrap_or("openai"),
        "contextWindow":model["contextWindow"].as_u64().unwrap_or(0),
        "reasoning":model.get("supportedReasoningEfforts").is_some(),
        "efforts":supported_model_efforts(model),"defaultEffort":model["defaultReasoningEffort"]}),
    )
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
