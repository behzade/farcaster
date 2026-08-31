use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet, VecDeque},
    fs::{self, File},
    io::{BufRead as _, BufReader, Read as _},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

use serde_json::Value;

use crate::{
    agents::extensions::ExtensionUiRequest,
    sessions::activity::{ActivityBuilder, AgentActivity, parse_iso_timestamp},
};

use super::super::{
    LoadedHistory, RUNNING_ACTIVITY_TIMEOUT, SessionDiscovery, SessionSummary, UsageSummary,
    filter_session_tree, normalize_lexical,
};
#[cfg(test)]
use super::super::{descendant_sessions, is_subagent_path, root_session_for_path, root_sessions};

const MAX_CANDIDATES: usize = 2_000;
const MAX_DIRECTORIES: usize = 2_000;
const MAX_DEPTH: usize = 6;
const MAX_LINES_PER_FILE: usize = 10_000;
const MAX_SEARCH_BYTES: usize = 64 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;
impl SessionSummary {
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_cached(
        id: String,
        path: PathBuf,
        project: PathBuf,
        title: String,
        first_user_message: String,
        timestamp: String,
        parent_session: Option<String>,
        modified: SystemTime,
        message_count: usize,
        usage: UsageSummary,
        archived: bool,
        is_running: bool,
        search: String,
    ) -> Self {
        Self::from_cached_for_harness(
            id,
            "pi".into(),
            path,
            project,
            title,
            first_user_message,
            timestamp,
            parent_session,
            modified,
            message_count,
            usage,
            archived,
            is_running,
            search,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_cached_for_harness(
        id: String,
        harness: String,
        path: PathBuf,
        project: PathBuf,
        title: String,
        first_user_message: String,
        timestamp: String,
        parent_session: Option<String>,
        modified: SystemTime,
        message_count: usize,
        usage: UsageSummary,
        archived: bool,
        is_running: bool,
        search: String,
    ) -> Self {
        let is_running = recently_running(is_running, modified, SystemTime::now());
        let parent_session = parent_session
            .as_deref()
            .and_then(|parent| resolve_parent_session(&path, parent));
        Self {
            id,
            app_session_id: 0,
            harness,
            path,
            project,
            title,
            first_user_message,
            timestamp,
            parent_session,
            modified,
            message_count,
            usage,
            archived,
            is_running,
            model: None,
            thinking_level: None,
            search,
        }
    }
}

pub(crate) fn configured_session_root() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let agent = std::env::var_os("PI_CODING_AGENT_DIR").map(PathBuf::from);
    let session = std::env::var_os("PI_CODING_AGENT_SESSION_DIR").map(PathBuf::from);
    session_root_from(home.as_deref(), agent.as_deref(), session.as_deref())
}

fn session_root_from(
    home: Option<&Path>,
    agent: Option<&Path>,
    session: Option<&Path>,
) -> Result<PathBuf, String> {
    let cwd = std::env::current_dir()
        .map_err(|error| format!("resolve current directory for Pi sessions: {error}"))?;
    session_root_from_at(&cwd, home, agent, session)
}

fn session_root_from_at(
    cwd: &Path,
    home: Option<&Path>,
    agent: Option<&Path>,
    session: Option<&Path>,
) -> Result<PathBuf, String> {
    let configured = if let Some(session) = session {
        session.to_path_buf()
    } else if let Some(agent) = agent {
        agent.join("sessions")
    } else {
        home.map(|home| home.join(".pi/agent/sessions"))
            .ok_or_else(|| {
                "HOME is not set and no Pi session directory override is configured".to_owned()
            })?
    };
    Ok(if configured.is_absolute() {
        normalize_lexical(&configured)
    } else {
        normalize_lexical(&cwd.join(configured))
    })
}

#[derive(Clone)]
struct CachedCandidate {
    len: u64,
    modified: SystemTime,
    parsed: (SessionSummary, AgentActivity),
}

#[derive(Default)]
struct DiscoveryCache {
    candidates: HashMap<PathBuf, CachedCandidate>,
}

static DISCOVERY_CACHE: OnceLock<Mutex<DiscoveryCache>> = OnceLock::new();

pub(crate) fn discover(query: &str) -> Result<SessionDiscovery, String> {
    let root = configured_session_root()?;
    let cache = DISCOVERY_CACHE.get_or_init(|| Mutex::new(DiscoveryCache::default()));
    let mut cache = cache
        .lock()
        .map_err(|_| "session discovery cache is unavailable".to_owned())?;
    discover_in_cached(&root, query, &mut cache)
}

pub(crate) fn load_history(path: &Path) -> Result<LoadedHistory, String> {
    let mut timing =
        crate::performance::OperationTiming::new(crate::performance::OperationKind::HistoryLoad, 0);
    let file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut entries = Vec::new();
    loop {
        line.clear();
        if reader
            .read_until(b'\n', &mut line)
            .map_err(|error| format!("read {}: {error}", path.display()))?
            == 0
        {
            break;
        }
        trim_frame(&mut line);
        if let Ok(entry) = serde_json::from_slice::<Value>(&line)
            && entry.get("type").and_then(Value::as_str) != Some("session")
        {
            entries.push(entry);
        }
    }
    timing.set_work(entries.len());
    let branch = active_branch_entries(&entries);
    let (model, thinking_level) = session_settings(&branch);
    Ok(LoadedHistory {
        messages: project_display_history_from_branch(&branch),
        model,
        thinking_level: Some(thinking_level),
        pending_question: pending_question_from_branch(&branch),
    })
}

fn session_settings(branch: &[&Value]) -> (Option<(String, String)>, String) {
    let mut model = None;
    let mut thinking_level = "off".to_owned();
    for entry in branch {
        match entry.get("type").and_then(Value::as_str) {
            Some("model_change") => {
                if let (Some(provider), Some(model_id)) = (
                    entry.get("provider").and_then(Value::as_str),
                    entry.get("modelId").and_then(Value::as_str),
                ) {
                    model = Some((provider.to_owned(), model_id.to_owned()));
                }
            }
            Some("thinking_level_change") => {
                if let Some(level) = entry.get("thinkingLevel").and_then(Value::as_str) {
                    thinking_level = level.to_owned();
                }
            }
            Some("message") => {
                let Some(message) = entry.get("message") else {
                    continue;
                };
                if message.get("role").and_then(Value::as_str) != Some("assistant") {
                    continue;
                }
                if let (Some(provider), Some(model_id)) = (
                    message.get("provider").and_then(Value::as_str),
                    message.get("model").and_then(Value::as_str),
                ) {
                    model = Some((provider.to_owned(), model_id.to_owned()));
                }
            }
            _ => {}
        }
    }
    (model, thinking_level)
}

fn is_application_exit(entry: &Value) -> bool {
    matches!(
        entry.get("customType").and_then(Value::as_str),
        Some("farcaster-application-exit" | "pi-gpui-application-exit")
    )
}

fn pending_question_from_branch(branch: &[&Value]) -> Option<ExtensionUiRequest> {
    let mut answered = HashSet::new();
    let mut application_exited = false;
    for entry in branch.iter().rev() {
        if entry.get("type").and_then(Value::as_str) == Some("custom_message")
            && is_application_exit(entry)
        {
            application_exited = true;
            continue;
        }
        let Some(message) = entry.get("message") else {
            continue;
        };
        match message.get("role").and_then(Value::as_str) {
            Some("toolResult") => {
                if let Some(id) = message.get("toolCallId").and_then(Value::as_str) {
                    answered.insert(id);
                }
                continue;
            }
            Some("assistant") => {}
            Some("user") => return None,
            _ => continue,
        }
        if !application_exited
            || message.get("stopReason").and_then(Value::as_str) != Some("toolUse")
        {
            return None;
        }
        let blocks = message.get("content").and_then(Value::as_array)?;
        for block in blocks.iter().rev() {
            if block.get("type").and_then(Value::as_str) != Some("toolCall")
                || block.get("name").and_then(Value::as_str) != Some("request_user_input")
            {
                continue;
            }
            let Some(tool_call_id) = block.get("id").and_then(Value::as_str) else {
                continue;
            };
            if answered.contains(tool_call_id) {
                continue;
            }
            let Some(arguments) = block.get("arguments") else {
                continue;
            };
            let owned_arguments;
            let arguments = if let Some(raw) = arguments.as_str() {
                let Ok(parsed) = serde_json::from_str::<Value>(raw) else {
                    continue;
                };
                owned_arguments = parsed;
                &owned_arguments
            } else {
                arguments
            };
            let Some(question) = arguments.get("question").and_then(Value::as_str) else {
                continue;
            };
            let question = question.trim();
            if question.is_empty() {
                continue;
            }
            let options = arguments
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let id = format!("restored-question:{tool_call_id}");
            return Some(if options.is_empty() {
                ExtensionUiRequest::Input {
                    id,
                    title: question.to_owned(),
                    placeholder: None,
                    timeout: None,
                }
            } else {
                ExtensionUiRequest::Select {
                    id,
                    title: question.to_owned(),
                    options,
                    timeout: None,
                }
            });
        }
        return None;
    }
    None
}

pub(crate) fn project_display_history(entries: &[Value]) -> Vec<Value> {
    project_display_history_from_branch(&active_branch_entries(entries))
}

fn project_display_history_from_branch(branch: &[&Value]) -> Vec<Value> {
    branch
        .iter()
        .filter_map(|entry| entry_message(entry))
        .collect()
}

fn active_branch_entries(entries: &[Value]) -> Vec<&Value> {
    let by_id = entries
        .iter()
        .filter_map(|entry| {
            entry
                .get("id")
                .and_then(Value::as_str)
                .map(|id| (id, entry))
        })
        .collect::<HashMap<_, _>>();
    let Some(mut current) = entries.last() else {
        return Vec::new();
    };
    if current.get("id").and_then(Value::as_str).is_none() {
        return entries.iter().collect();
    }
    let mut branch = Vec::new();
    let mut seen = HashSet::new();
    while let Some(id) = current.get("id").and_then(Value::as_str) {
        if !seen.insert(id) {
            break;
        }
        branch.push(current);
        let Some(parent) = current.get("parentId").and_then(Value::as_str) else {
            break;
        };
        let Some(entry) = by_id.get(parent) else {
            break;
        };
        current = entry;
    }
    branch.reverse();
    branch
}

fn entry_message(entry: &Value) -> Option<Value> {
    match entry.get("type").and_then(Value::as_str)? {
        "message" => entry.get("message").cloned(),
        "custom_message" => Some(json_object([
            ("role", Value::String("custom".into())),
            (
                "customType",
                entry.get("customType").cloned().unwrap_or(Value::Null),
            ),
            (
                "content",
                entry.get("content").cloned().unwrap_or(Value::Null),
            ),
            (
                "display",
                entry.get("display").cloned().unwrap_or(Value::Bool(true)),
            ),
        ])),
        "branch_summary" => Some(json_object([
            ("role", Value::String("branchSummary".into())),
            (
                "summary",
                entry.get("summary").cloned().unwrap_or(Value::Null),
            ),
        ])),
        "compaction" => Some(json_object([
            ("role", Value::String("compactionSummary".into())),
            (
                "summary",
                entry.get("summary").cloned().unwrap_or(Value::Null),
            ),
        ])),
        _ => None,
    }
}

fn json_object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

#[cfg(test)]
fn discover_in(root: &Path, query: &str) -> Result<Vec<SessionSummary>, String> {
    discover_in_with_status(root, query).map(|discovery| discovery.sessions)
}

#[cfg(test)]
fn discover_in_with_status(root: &Path, query: &str) -> Result<SessionDiscovery, String> {
    discover_in_cached(root, query, &mut DiscoveryCache::default())
}

fn discover_in_cached(
    root: &Path,
    query: &str,
    cache: &mut DiscoveryCache,
) -> Result<SessionDiscovery, String> {
    crate::performance::count_catalog_scan();
    match fs::metadata(root) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SessionDiscovery {
                sessions: Vec::new(),
                activities: HashMap::new(),
                exhaustive: true,
            });
        }
        Err(error) => {
            return Err(format!("inspect session root {}: {error}", root.display()));
        }
    }
    let mut candidates = BinaryHeap::<Reverse<(SystemTime, PathBuf)>>::new();
    let mut exhaustive = true;
    let mut directories_seen = 0_usize;
    let mut queue = VecDeque::from([(root.to_path_buf(), 0_usize)]);
    while let Some((directory, depth)) = queue.pop_front() {
        if directories_seen >= MAX_DIRECTORIES {
            exhaustive = false;
            break;
        }
        directories_seen = directories_seen.saturating_add(1);
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => {
                exhaustive = false;
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    exhaustive = false;
                    continue;
                }
            };
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => {
                    exhaustive = false;
                    continue;
                }
            };
            if file_type.is_dir() {
                if depth < MAX_DEPTH {
                    queue.push_back((path, depth + 1));
                } else {
                    exhaustive = false;
                }
            } else if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
                let metadata = match entry.metadata() {
                    Ok(metadata) => metadata,
                    Err(_) => {
                        exhaustive = false;
                        continue;
                    }
                };
                let modified = match metadata.modified() {
                    Ok(modified) => modified,
                    Err(_) => {
                        exhaustive = false;
                        continue;
                    }
                };
                let candidate = (modified, path);
                if candidates.len() < MAX_CANDIDATES {
                    candidates.push(Reverse(candidate));
                } else {
                    exhaustive = false;
                    if candidates.peek().is_some_and(|oldest| candidate > oldest.0) {
                        candidates.pop();
                        candidates.push(Reverse(candidate));
                    }
                }
            }
        }
    }
    let mut seen = HashSet::new();
    let mut parsed = Vec::new();
    for Reverse((modified, path)) in candidates {
        let normalized = normalize_session_path(&path);
        seen.insert(normalized.clone());
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                exhaustive = false;
                continue;
            }
        };
        let len = metadata.len();
        if let Some(cached) = cache.candidates.get(&normalized)
            && cached.len == len
            && cached.modified == modified
        {
            crate::performance::count_catalog_cache_hit();
            let mut value = cached.parsed.clone();
            value.0.is_running =
                recently_running(value.0.is_running, value.0.modified, SystemTime::now());
            if !value.0.is_running
                && matches!(
                    value.1.lifecycle,
                    crate::sessions::activity::AgentLifecycle::NeedsInput
                        | crate::sessions::activity::AgentLifecycle::Working
                )
            {
                value.1.lifecycle = crate::sessions::activity::AgentLifecycle::Unknown;
                value.1.current_tool = None;
            }
            parsed.push(value);
            continue;
        }
        match parse_candidate(&path) {
            Ok(Some(value)) => {
                cache.candidates.insert(
                    normalized,
                    CachedCandidate {
                        len,
                        modified,
                        parsed: value.clone(),
                    },
                );
                parsed.push(value);
            }
            Ok(None) | Err(_) => exhaustive = false,
        }
    }
    if exhaustive {
        cache.candidates.retain(|path, _| seen.contains(path));
    }
    let activities = parsed
        .iter()
        .map(|(_, activity)| (activity.session_id.clone(), activity.clone()))
        .collect();
    let mut sessions = parsed
        .into_iter()
        .map(|(session, _)| session)
        .collect::<Vec<_>>();
    sessions = filter_session_tree(sessions, query);
    sessions.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| right.timestamp.cmp(&left.timestamp))
    });
    Ok(SessionDiscovery {
        sessions,
        activities,
        exhaustive,
    })
}

fn parse_candidate(path: &Path) -> Result<Option<(SessionSummary, AgentActivity)>, String> {
    crate::performance::count_catalog_parse();
    let file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let metadata = file.metadata().ok();
    let modified = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let file_started = metadata
        .as_ref()
        .and_then(|metadata| metadata.created().ok())
        .unwrap_or(modified);
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    if reader
        .read_until(b'\n', &mut line)
        .map_err(|error| error.to_string())?
        == 0
    {
        return Ok(None);
    }
    trim_frame(&mut line);
    let header: Value = match serde_json::from_slice(&line) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    if header.get("type").and_then(Value::as_str) != Some("session") {
        return Ok(None);
    }
    let cwd = header
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if cwd.is_empty() {
        return Ok(None);
    }
    let project =
        normalize_existing(Path::new(cwd)).unwrap_or_else(|_| normalize_lexical(Path::new(cwd)));
    let id = header
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if id.is_empty() {
        return Ok(None);
    }
    let timestamp = header
        .get("timestamp")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let started = parse_iso_timestamp(&timestamp).unwrap_or(file_started);
    let parent_session = header
        .get("parentSession")
        .and_then(Value::as_str)
        .and_then(|parent| resolve_parent_session(path, parent))
        .or_else(|| parent_session_from_path(path));
    let mut name = None;
    let mut first_user_message = None;
    let mut activity = ActivityBuilder::default();
    let mut detail_limited = true;
    let mut message_count = 0_usize;
    let mut usage = UsageSummary::default();
    let mut is_running = false;
    let mut search = String::new();
    let mut activity_entries = Vec::new();
    for _ in 0..MAX_LINES_PER_FILE {
        line.clear();
        if reader
            .read_until(b'\n', &mut line)
            .map_err(|error| format!("read {}: {error}", path.display()))?
            == 0
        {
            detail_limited = false;
            break;
        }
        trim_frame(&mut line);
        let Ok(entry) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        match entry.get("type").and_then(Value::as_str) {
            Some("session_info") => {
                name = entry
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            }
            Some("message") => {
                message_count = message_count.saturating_add(1);
                if let Some(message) = entry.get("message") {
                    usage.add(message_usage(message));
                    is_running = message_keeps_session_running(message);
                    let text = visible_user_text(message);
                    if !text.is_empty()
                        && message.get("role").and_then(Value::as_str) == Some("user")
                        && first_user_message.is_none()
                    {
                        first_user_message = Some(text.clone());
                    }
                    append_bounded(&mut search, &text);
                }
            }
            Some("custom_message") => {
                if is_application_exit(&entry) {
                    is_running = false;
                }
                if entry.get("display").and_then(Value::as_bool) == Some(true) {
                    append_bounded(
                        &mut search,
                        entry
                            .get("content")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    );
                }
            }
            _ => {}
        }
        activity_entries.push(entry);
    }
    if detail_limited {
        line.clear();
        detail_limited = reader
            .read_until(b'\n', &mut line)
            .map_err(|error| format!("read {}: {error}", path.display()))?
            != 0;
    }
    let is_running = !detail_limited && recently_running(is_running, modified, SystemTime::now());
    let first_user_message = first_user_message.unwrap_or_default();
    let title = name
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback_title(&first_user_message, &timestamp));
    append_bounded(&mut search, &title);
    append_bounded(&mut search, &project.to_string_lossy());
    let session_path = normalize_session_path(path);
    let branch_entries = active_branch_entries(&activity_entries);
    let (model, thinking_level) = session_settings(&branch_entries);
    for entry in &branch_entries {
        activity.observe_entry(entry);
    }
    let mut activity = activity.finish(
        id.clone(),
        session_path.clone(),
        &project,
        &title,
        &first_user_message,
        usage,
        started,
        modified,
        is_running,
        detail_limited,
    );
    if detail_limited {
        activity.lifecycle = crate::sessions::activity::AgentLifecycle::Unknown;
        activity.current_tool = None;
        activity.ended = None;
        activity.elapsed = None;
    }
    Ok(Some((
        SessionSummary {
            id,
            app_session_id: 0,
            harness: "pi".into(),
            path: session_path,
            project,
            title,
            first_user_message,
            timestamp,
            parent_session,
            modified,
            message_count,
            usage,
            archived: false,
            is_running,
            model,
            thinking_level: (!detail_limited).then_some(thinking_level),
            search: search.to_lowercase(),
        },
        activity,
    )))
}

fn recently_running(incomplete: bool, modified: SystemTime, now: SystemTime) -> bool {
    incomplete && now.duration_since(modified).unwrap_or_default() <= RUNNING_ACTIVITY_TIMEOUT
}

fn message_keeps_session_running(message: &Value) -> bool {
    match message.get("role").and_then(Value::as_str) {
        Some("user" | "toolResult") => true,
        Some("assistant") => message.get("stopReason").and_then(Value::as_str) == Some("toolUse"),
        _ => false,
    }
}

fn message_usage(message: &Value) -> UsageSummary {
    let Some(usage) = message.get("usage") else {
        return UsageSummary::default();
    };
    let input = u64_field(usage, "input");
    let output = u64_field(usage, "output");
    let cache_read = u64_field(usage, "cacheRead");
    let cache_write = u64_field(usage, "cacheWrite");
    let total = u64_field(usage, "totalTokens").max(
        input
            .saturating_add(output)
            .saturating_add(cache_read)
            .saturating_add(cache_write),
    );
    let cost_micros = usage
        .get("cost")
        .and_then(|cost| cost.get("total"))
        .and_then(Value::as_f64)
        .filter(|cost| cost.is_finite() && *cost > 0.0)
        .map_or(0, |cost| (cost * 1_000_000.0).round() as u64);
    UsageSummary {
        input,
        output,
        cache_read,
        cache_write,
        total,
        cost_micros,
    }
}

fn u64_field(value: &Value, field: &str) -> u64 {
    value.get(field).and_then(Value::as_u64).unwrap_or(0)
}

fn resolve_parent_session(session_path: &Path, parent: &str) -> Option<String> {
    let parent = parent.trim();
    if parent.is_empty() {
        return None;
    }
    let reference = Path::new(parent);
    let path_like = reference.is_absolute()
        || reference
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("jsonl")
        || reference.components().count() > 1;
    if !path_like {
        return Some(parent.to_owned());
    }

    let referenced_path = if reference.is_absolute() {
        reference.to_owned()
    } else {
        session_path.parent()?.join(reference)
    };
    Some(
        session_header_id(&normalize_session_path(&referenced_path))
            .unwrap_or_else(|| unresolved_parent_id(reference)),
    )
}

fn unresolved_parent_id(reference: &Path) -> String {
    if let Some(stem) = reference
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
    {
        return stem.rsplit('_').next().unwrap_or(stem).to_owned();
    }
    let hash = reference
        .to_string_lossy()
        .bytes()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    format!("unresolved-parent-{hash:016x}")
}

fn session_header_id(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file).take((MAX_HEADER_BYTES + 1) as u64);
    let mut line = Vec::new();
    let read = reader.read_until(b'\n', &mut line).ok()?;
    if read == 0 || line.len() > MAX_HEADER_BYTES {
        return None;
    }
    trim_frame(&mut line);
    let header = serde_json::from_slice::<Value>(&line).ok()?;
    (header.get("type").and_then(Value::as_str) == Some("session"))
        .then(|| header.get("id").and_then(Value::as_str))
        .flatten()
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

fn parent_session_from_path(path: &Path) -> Option<String> {
    for directory in path.parent()?.ancestors() {
        let mut root_name = directory.file_name()?.to_os_string();
        root_name.push(".jsonl");
        let root_path = directory.with_file_name(root_name);
        let Ok(file) = File::open(root_path) else {
            continue;
        };
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        let Ok(read) = reader.read_until(b'\n', &mut line) else {
            continue;
        };
        if read == 0 {
            continue;
        }
        trim_frame(&mut line);
        let Ok(header) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if header.get("type").and_then(Value::as_str) == Some("session")
            && let Some(id) = header.get("id").and_then(Value::as_str)
            && !id.is_empty()
        {
            return Some(id.to_owned());
        }
    }
    None
}

fn visible_user_text(message: &Value) -> String {
    let Some(content) = message.get("content") else {
        return String::new();
    };
    if let Some(text) = content.as_str() {
        return text.to_owned();
    }
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| {
                    (block.get("type").and_then(Value::as_str) == Some("text"))
                        .then(|| block.get("text").and_then(Value::as_str))
                        .flatten()
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

fn fallback_title(message: &str, timestamp: &str) -> String {
    let title = message
        .split_whitespace()
        .take(10)
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        format!("Session {}", timestamp.get(..10).unwrap_or("unknown"))
    } else {
        title
    }
}

fn append_bounded(target: &mut String, value: &str) {
    if target.len() >= MAX_SEARCH_BYTES {
        return;
    }
    target.push(' ');
    for character in value.chars() {
        if target.len() + character.len_utf8() > MAX_SEARCH_BYTES {
            break;
        }
        target.push(character);
    }
}

fn trim_frame(frame: &mut Vec<u8>) {
    if frame.last() == Some(&b'\n') {
        frame.pop();
    }
    if frame.last() == Some(&b'\r') {
        frame.pop();
    }
}

fn normalize_existing(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map(|path| normalize_lexical(&path))
        .map_err(|error| format!("resolve project {}: {error}", path.display()))
}

pub(crate) fn normalize_session_path(path: &Path) -> PathBuf {
    path.canonicalize()
        .map(|canonical| normalize_lexical(&canonical))
        .unwrap_or_else(|_| normalize_lexical(path))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
