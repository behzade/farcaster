//! Transient, read-only activity derived from Pi session JSONL.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use serde_json::Value;

use crate::sessions::{UsageSummary, normalize_lexical};

const MAX_ACTIVITY_CHARS: usize = 160;
const MAX_TOOL_TARGET_CHARS: usize = 120;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentOutcome {
    Complete,
    Failed,
    Incomplete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentLifecycle {
    NeedsInput,
    Working,
    Unknown,
    Completed(AgentOutcome),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentToolActivity {
    pub name: String,
    pub target: String,
    pub failed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObservedPath {
    pub path: PathBuf,
    pub observed_at: SystemTime,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum FileMutationKind {
    Edit { patch: String, complete: bool },
    Write { content: String },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct FileMutation {
    pub path: PathBuf,
    pub observed_at: SystemTime,
    pub kind: FileMutationKind,
}

#[derive(Clone, Debug)]
struct PendingMutation {
    path: PathBuf,
    observed_at: Option<SystemTime>,
    kind: FileMutationKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentActivity {
    pub session_id: String,
    pub session_path: PathBuf,
    pub role: String,
    pub activity: String,
    pub lifecycle: AgentLifecycle,
    pub current_tool: Option<AgentToolActivity>,
    pub recent_tool: Option<AgentToolActivity>,
    pub tool_call_count: usize,
    pub limited: bool,
    pub usage: UsageSummary,
    pub started: SystemTime,
    pub ended: Option<SystemTime>,
    pub elapsed: Option<Duration>,
    pub changed_paths: Vec<ObservedPath>,
    pub file_mutations: Vec<FileMutation>,
}

#[derive(Default)]
pub(crate) struct ActivityBuilder {
    tools: HashMap<String, AgentToolActivity>,
    outstanding_tool_ids: Vec<String>,
    recent_tool: Option<AgentToolActivity>,
    tool_call_count: usize,
    outcome: Option<AgentOutcome>,
    terminal_time: Option<SystemTime>,
    pending_mutations: HashMap<String, PendingMutation>,
    observed_paths: Vec<(PathBuf, Option<SystemTime>)>,
    file_mutations: Vec<PendingMutation>,
}

impl ActivityBuilder {
    pub(crate) fn observe_entry(&mut self, entry: &Value) {
        if entry.get("type").and_then(Value::as_str) == Some("compaction") {
            let observed_at = entry
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_iso_timestamp);
            self.observe_compaction_paths(entry, observed_at);
            return;
        }
        let Some(message) = entry
            .get("message")
            .filter(|_| entry.get("type").and_then(Value::as_str) == Some("message"))
        else {
            return;
        };
        let observed_at = entry_timestamp(entry, message);
        match message.get("role").and_then(Value::as_str) {
            Some("assistant") => self.observe_assistant(message, observed_at),
            Some("toolResult") => self.observe_tool_result(message),
            Some("user") => {
                self.outcome = None;
                self.terminal_time = None;
            }
            _ => {}
        }
    }

    fn observe_assistant(&mut self, message: &Value, observed_at: Option<SystemTime>) {
        if let Some(blocks) = message.get("content").and_then(Value::as_array) {
            for block in blocks {
                if block.get("type").and_then(Value::as_str) != Some("toolCall") {
                    continue;
                }
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Tool")
                    .to_owned();
                let arguments = block.get("arguments").unwrap_or(&Value::Null);
                let target = tool_target(arguments);
                let mutation = pending_mutation(&name, arguments, observed_at);
                let tool = AgentToolActivity {
                    name,
                    target,
                    failed: false,
                };
                self.tool_call_count = self.tool_call_count.saturating_add(1);
                if !id.is_empty() {
                    if let Some(mutation) = mutation {
                        self.pending_mutations.insert(id.clone(), mutation);
                    }
                    self.tools.insert(id.clone(), tool);
                    self.outstanding_tool_ids.push(id);
                } else {
                    self.recent_tool = Some(tool);
                }
            }
        }
        self.outcome = match message.get("stopReason").and_then(Value::as_str) {
            Some("stop") => Some(AgentOutcome::Complete),
            Some("error") => Some(AgentOutcome::Failed),
            Some("aborted" | "length") => Some(AgentOutcome::Incomplete),
            Some("toolUse") | None => None,
            Some(_) => None,
        };
        self.terminal_time = self.outcome.and(observed_at);
    }

    fn observe_tool_result(&mut self, message: &Value) {
        let id = message
            .get("toolCallId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let failed = message
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut tool = self.tools.remove(id).unwrap_or_else(|| AgentToolActivity {
            name: message
                .get("toolName")
                .and_then(Value::as_str)
                .unwrap_or("Tool")
                .to_owned(),
            target: String::new(),
            failed,
        });
        tool.failed = failed;
        self.recent_tool = Some(tool);
        if let Some(mut mutation) = self.pending_mutations.remove(id)
            && !failed
        {
            if let FileMutationKind::Edit { patch, complete } = &mut mutation.kind
                && let Some(result_patch) = message
                    .pointer("/details/patch")
                    .or_else(|| message.pointer("/details/diff"))
                    .and_then(Value::as_str)
                    .filter(|patch| !patch.is_empty())
            {
                *patch = result_patch.to_owned();
                *complete = true;
            }
            self.observed_paths
                .push((mutation.path.clone(), mutation.observed_at));
            self.file_mutations.push(mutation);
        }
        self.outstanding_tool_ids
            .retain(|outstanding| outstanding != id);
        self.outcome = None;
        self.terminal_time = None;
    }

    fn observe_compaction_paths(&mut self, entry: &Value, observed_at: Option<SystemTime>) {
        for source in [
            entry.get("changedPaths"),
            entry.get("modifiedFiles"),
            entry.pointer("/details/changedPaths"),
            entry.pointer("/details/modifiedFiles"),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(paths) = source.as_array() {
                self.observed_paths.extend(
                    paths
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|path| !path.is_empty())
                        .map(|path| (PathBuf::from(path), observed_at)),
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn finish(
        self,
        session_id: String,
        session_path: PathBuf,
        project: &Path,
        title: &str,
        first_user_message: &str,
        usage: UsageSummary,
        started: SystemTime,
        modified: SystemTime,
        is_running: bool,
        limited: bool,
    ) -> AgentActivity {
        let lifecycle = if is_running {
            AgentLifecycle::Working
        } else if let Some(outcome) = self.outcome {
            AgentLifecycle::Completed(outcome)
        } else {
            AgentLifecycle::Unknown
        };
        let ended = matches!(lifecycle, AgentLifecycle::Completed(_))
            .then_some(self.terminal_time.unwrap_or(modified));
        let elapsed = ended.and_then(|ended| ended.duration_since(started).ok());
        let unmatched_tool = self
            .outstanding_tool_ids
            .iter()
            .rev()
            .find_map(|id| self.tools.get(id))
            .cloned();
        let (current_tool, recent_tool) = if matches!(lifecycle, AgentLifecycle::Completed(_)) {
            (None, unmatched_tool.or(self.recent_tool))
        } else {
            (unmatched_tool, self.recent_tool)
        };
        let mut changed_paths = Vec::<ObservedPath>::new();
        for (path, observed_at) in self.observed_paths {
            let path = if path.is_absolute() {
                normalize_lexical(&path)
            } else {
                normalize_lexical(&project.join(path))
            };
            let observed_at = observed_at.unwrap_or(modified);
            if let Some(observed) = changed_paths
                .iter_mut()
                .find(|observed| observed.path == path)
            {
                observed.observed_at = observed.observed_at.max(observed_at);
            } else {
                changed_paths.push(ObservedPath { path, observed_at });
            }
        }
        changed_paths.sort_by_key(|observed| observed.observed_at);
        let mut file_mutations = self
            .file_mutations
            .into_iter()
            .map(|mutation| {
                let path = if mutation.path.is_absolute() {
                    normalize_lexical(&mutation.path)
                } else {
                    normalize_lexical(&project.join(mutation.path))
                };
                FileMutation {
                    path,
                    observed_at: mutation.observed_at.unwrap_or(modified),
                    kind: mutation.kind,
                }
            })
            .collect::<Vec<_>>();
        file_mutations.sort_by_key(|mutation| mutation.observed_at);
        AgentActivity {
            session_id,
            session_path,
            role: role_label(title),
            activity: bounded(first_user_message, MAX_ACTIVITY_CHARS),
            lifecycle,
            current_tool,
            recent_tool,
            tool_call_count: self.tool_call_count,
            limited,
            usage,
            started,
            ended,
            elapsed,
            changed_paths,
            file_mutations,
        }
    }
}

fn pending_mutation(
    name: &str,
    arguments: &Value,
    observed_at: Option<SystemTime>,
) -> Option<PendingMutation> {
    let path = arguments
        .get("path")
        .or_else(|| arguments.get("file_path"))
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)?;
    let kind = match name.trim().to_ascii_lowercase().as_str() {
        "edit" => FileMutationKind::Edit {
            patch: edit_preview(arguments),
            complete: false,
        },
        "write" => FileMutationKind::Write {
            content: arguments.get("content").and_then(Value::as_str)?.to_owned(),
        },
        _ => return None,
    };
    Some(PendingMutation {
        path,
        observed_at,
        kind,
    })
}

fn edit_preview(arguments: &Value) -> String {
    let edits = arguments
        .get("edits")
        .and_then(Value::as_array)
        .map(|edits| {
            edits
                .iter()
                .filter_map(|edit| {
                    edit.get("oldText")
                        .and_then(Value::as_str)
                        .zip(edit.get("newText").and_then(Value::as_str))
                })
                .collect::<Vec<_>>()
        })
        .or_else(|| {
            arguments
                .get("oldText")
                .and_then(Value::as_str)
                .zip(arguments.get("newText").and_then(Value::as_str))
                .map(|edit| vec![edit])
        })
        .unwrap_or_default();
    let mut patch = String::new();
    for (index, (old, new)) in edits.into_iter().enumerate() {
        if index > 0 {
            patch.push_str("     ...\n");
        }
        for line in old.lines() {
            patch.push_str("- ");
            patch.push_str(line);
            patch.push('\n');
        }
        for line in new.lines() {
            patch.push_str("+ ");
            patch.push_str(line);
            patch.push('\n');
        }
    }
    patch
}

fn entry_timestamp(entry: &Value, message: &Value) -> Option<SystemTime> {
    entry
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_iso_timestamp)
        .or_else(|| {
            message
                .get("timestamp")
                .and_then(Value::as_u64)
                .map(|millis| SystemTime::UNIX_EPOCH + Duration::from_millis(millis))
        })
}

pub(crate) fn parse_iso_timestamp(value: &str) -> Option<SystemTime> {
    let (date, time) = value.strip_suffix('Z')?.split_once('T')?;
    let mut date = date.split('-');
    let year = date.next()?.parse::<i64>().ok()?;
    let month = date.next()?.parse::<i64>().ok()?;
    let day = date.next()?.parse::<i64>().ok()?;
    if date.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut time = time.split(':');
    let hour = time.next()?.parse::<u64>().ok()?;
    let minute = time.next()?.parse::<u64>().ok()?;
    let second_fraction = time.next()?;
    if time.next().is_some() || hour > 23 || minute > 59 {
        return None;
    }
    let (second, fraction) = second_fraction
        .split_once('.')
        .map_or((second_fraction, ""), |parts| parts);
    let second = second.parse::<u64>().ok()?;
    if second > 59 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let millis = fraction
        .chars()
        .take(3)
        .collect::<String>()
        .parse::<u64>()
        .unwrap_or(0)
        * match fraction.len().min(3) {
            0 | 3 => 1,
            1 => 100,
            2 => 10,
            _ => 1,
        };
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let days = u64::try_from(days).ok()?;
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(hour.checked_mul(3_600)?)?
        .checked_add(minute.checked_mul(60)?)?
        .checked_add(second)?;
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds) + Duration::from_millis(millis))
}

fn role_label(title: &str) -> String {
    let role = bounded(title, 28);
    if role.is_empty() {
        "Agent".into()
    } else {
        role
    }
}

fn tool_target(arguments: &Value) -> String {
    for key in ["path", "command", "query", "pattern", "action"] {
        if let Some(value) = arguments.get(key).and_then(Value::as_str)
            && !value.is_empty()
        {
            return bounded(value, MAX_TOOL_TARGET_CHARS);
        }
    }
    String::new()
}

fn bounded(value: &str, max: usize) -> String {
    let mut result = value.chars().take(max).collect::<String>();
    if value.chars().count() > max {
        result.push('…');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_timestamps_without_a_runtime_dependency() {
        assert_eq!(
            parse_iso_timestamp("1970-01-01T00:00:01.250Z")
                .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok()),
            Some(Duration::from_millis(1_250))
        );
        assert_eq!(parse_iso_timestamp("not-a-timestamp"), None);
    }

    #[test]
    fn pairs_current_and_recent_tools_and_collects_typed_paths() {
        let mut builder = ActivityBuilder::default();
        builder.observe_entry(&serde_json::json!({
            "type":"message",
            "message":{"role":"assistant","stopReason":"toolUse","content":[
                {"type":"toolCall","id":"one","name":"edit","arguments":{"path":"src/main.rs"}}
            ]}
        }));
        let active = builder.finish(
            "child".into(),
            PathBuf::from("/sessions/child"),
            Path::new("/project"),
            "Implementation session",
            "Implement the feature",
            UsageSummary::default(),
            SystemTime::UNIX_EPOCH,
            SystemTime::UNIX_EPOCH + Duration::from_secs(4),
            true,
            false,
        );
        assert_eq!(active.lifecycle, AgentLifecycle::Working);
        assert_eq!(
            active.current_tool.as_ref().map(|tool| tool.name.as_str()),
            Some("edit")
        );
        assert!(active.changed_paths.is_empty());

        let mut builder = ActivityBuilder::default();
        builder.observe_entry(&serde_json::json!({
            "type":"message",
            "message":{"role":"assistant","stopReason":"toolUse","content":[
                {"type":"toolCall","id":"one","name":"read","arguments":{"path":"README.md"}}
            ]}
        }));
        builder.observe_entry(&serde_json::json!({
            "type":"message","message":{"role":"toolResult","toolCallId":"one","toolName":"read","isError":false}
        }));
        builder.observe_entry(&serde_json::json!({
            "type":"message","message":{"role":"assistant","stopReason":"stop","content":[]}
        }));
        let done = builder.finish(
            "child".into(),
            PathBuf::from("/sessions/child"),
            Path::new("/project"),
            "Review session",
            "Review",
            UsageSummary::default(),
            SystemTime::UNIX_EPOCH,
            SystemTime::UNIX_EPOCH + Duration::from_secs(4),
            false,
            false,
        );
        assert_eq!(
            done.lifecycle,
            AgentLifecycle::Completed(AgentOutcome::Complete)
        );
        assert!(done.current_tool.is_none());
        assert_eq!(
            done.recent_tool.as_ref().map(|tool| tool.name.as_str()),
            Some("read")
        );
        assert_eq!(done.elapsed, Some(Duration::from_secs(4)));
    }

    #[test]
    fn out_of_order_parallel_results_leave_an_outstanding_current_tool() {
        let mut builder = ActivityBuilder::default();
        builder.observe_entry(&serde_json::json!({
            "type":"message","message":{"role":"assistant","stopReason":"toolUse","content":[
                {"type":"toolCall","id":"one","name":"read","arguments":{"path":"one"}},
                {"type":"toolCall","id":"two","name":"read","arguments":{"path":"two"}}
            ]}
        }));
        builder.observe_entry(&serde_json::json!({
            "type":"message","message":{"role":"toolResult","toolCallId":"two","toolName":"read","isError":false}
        }));
        let activity = builder.finish(
            "id".into(),
            PathBuf::new(),
            Path::new("/project"),
            "worker",
            "task",
            UsageSummary::default(),
            SystemTime::UNIX_EPOCH,
            SystemTime::UNIX_EPOCH,
            true,
            false,
        );
        assert_eq!(
            activity
                .current_tool
                .as_ref()
                .map(|tool| tool.target.as_str()),
            Some("one")
        );
    }

    #[test]
    fn successful_mutations_keep_call_timestamps_and_failed_mutations_are_omitted() {
        let mut builder = ActivityBuilder::default();
        for (timestamp, id, name, path) in [
            ("1970-01-01T00:00:03Z", "late", "edit", "src/late.rs"),
            ("1970-01-01T00:00:01Z", "early", "write", "src/early.rs"),
            ("1970-01-01T00:00:02Z", "failed", "edit", "src/failed.rs"),
        ] {
            builder.observe_entry(&serde_json::json!({
                "type":"message","timestamp":timestamp,
                "message":{"role":"assistant","stopReason":"toolUse","content":[
                    {"type":"toolCall","id":id,"name":name,"arguments":{
                        "path":path,
                        "content":"created\n",
                        "oldText":"before",
                        "newText":"after"
                    }}
                ]}
            }));
        }
        for (id, failed) in [("late", false), ("failed", true), ("early", false)] {
            builder.observe_entry(&serde_json::json!({
                "type":"message","message":{
                    "role":"toolResult",
                    "toolCallId":id,
                    "toolName":"edit",
                    "isError":failed,
                    "details":{"patch":"@@\n-before\n+after\n"}
                }
            }));
        }
        let activity = builder.finish(
            "id".into(),
            PathBuf::new(),
            Path::new("/project"),
            "worker",
            "task",
            UsageSummary::default(),
            SystemTime::UNIX_EPOCH,
            SystemTime::UNIX_EPOCH + Duration::from_secs(4),
            false,
            false,
        );
        assert_eq!(
            activity
                .changed_paths
                .iter()
                .map(|path| path.path.as_path())
                .collect::<Vec<_>>(),
            vec![
                Path::new("/project/src/early.rs"),
                Path::new("/project/src/late.rs")
            ]
        );
        assert_eq!(
            activity
                .changed_paths
                .iter()
                .map(|path| path
                    .observed_at
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .expect("timestamp")
                    .as_secs())
                .collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert_eq!(activity.file_mutations.len(), 2);
        assert!(matches!(
            &activity.file_mutations[0].kind,
            FileMutationKind::Write { content } if content == "created\n"
        ));
        assert!(matches!(
            &activity.file_mutations[1].kind,
            FileMutationKind::Edit { patch, complete: true } if patch.contains("+after")
        ));
    }

    #[test]
    fn structured_compaction_paths_and_limited_counts_are_preserved() {
        let mut builder = ActivityBuilder::default();
        builder.observe_entry(&serde_json::json!({
            "type":"compaction","details":{"changedPaths":["src/lib.rs", "src/lib.rs"],"modifiedFiles":["src/main.rs"]}
        }));
        let activity = builder.finish(
            "id".into(),
            PathBuf::new(),
            Path::new("/project"),
            "Implementation session",
            &"x".repeat(MAX_ACTIVITY_CHARS + 5),
            UsageSummary::default(),
            SystemTime::UNIX_EPOCH,
            SystemTime::UNIX_EPOCH,
            true,
            true,
        );
        assert!(activity.limited);
        assert!(activity.activity.ends_with('…'));
        assert_eq!(activity.changed_paths.len(), 2);
        assert!(activity.file_mutations.is_empty());
        assert_eq!(
            activity.changed_paths[0].path,
            PathBuf::from("/project/src/lib.rs")
        );
    }

    #[test]
    fn terminal_outcomes_use_only_explicit_stop_reasons() {
        for (reason, expected) in [
            ("error", AgentOutcome::Failed),
            ("aborted", AgentOutcome::Incomplete),
            ("length", AgentOutcome::Incomplete),
        ] {
            let mut builder = ActivityBuilder::default();
            builder.observe_entry(&serde_json::json!({
                "type":"message","message":{"role":"assistant","stopReason":reason,"content":[]}
            }));
            let activity = builder.finish(
                "id".into(),
                PathBuf::new(),
                Path::new("/project"),
                "worker",
                "task",
                UsageSummary::default(),
                SystemTime::UNIX_EPOCH,
                SystemTime::UNIX_EPOCH,
                false,
                false,
            );
            assert_eq!(activity.lifecycle, AgentLifecycle::Completed(expected));
        }
    }
}
