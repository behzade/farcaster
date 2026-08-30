use std::{
    collections::HashMap,
    fs,
    net::IpAddr,
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
};

use rmcp::schemars;
use serde::{Deserialize, Serialize};

const SESSION_LABEL: &str = "Allow for all agents until Farcaster exits";
const PROJECT_LABEL: &str = "Add to project policy";
const DENY_LABEL: &str = "Deny";
const MAX_RIGHTS_PER_REQUEST: usize = 32;
const MAX_STORED_RIGHTS: usize = 256;
const MAX_PENDING_APPROVALS: usize = 16;
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub(crate) enum AccessRight {
    Filesystem {
        access: FilesystemRightAccess,
        path: String,
        scope: FilesystemScope,
    },
    NetworkHost {
        host: String,
    },
    NetworkEndpoint {
        host: String,
        port: u16,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FilesystemRightAccess {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FilesystemScope {
    File,
    Tree,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct RequestAccessParams {
    /// One or more exact capabilities to add.
    pub(crate) rights: Vec<AccessRight>,
    /// Why the task needs these capabilities.
    pub(crate) reason: String,
}

#[derive(Clone)]
pub(crate) struct ApprovalService {
    shared: Arc<Shared>,
    prompts: async_channel::Sender<ApprovalPrompt>,
}

#[derive(Clone)]
pub(crate) struct ApprovalUi {
    shared: Arc<Shared>,
    prompts: async_channel::Receiver<ApprovalPrompt>,
}

#[derive(Clone, Debug)]
pub(crate) struct ApprovalPrompt {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) options: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct GrantStore {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for GrantStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GrantStore")
            .field("project", &self.shared.project)
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
pub(crate) struct ResolvedGrants {
    pub(crate) readable: Vec<PathBuf>,
    pub(crate) writable: Vec<PathBuf>,
    pub(crate) writable_files: Vec<PathBuf>,
    pub(crate) network_hosts: Vec<String>,
    pub(crate) loopback_ports: Vec<u16>,
}

struct Shared {
    project: PathBuf,
    home: PathBuf,
    project_policy_path: PathBuf,
    agent_state: PathBuf,
    temporary: PathBuf,
    policy_validator: Arc<dyn super::PolicyValidator>,
    state: Mutex<GrantState>,
    filesystem_mode: AtomicU8,
    network_full: AtomicU8,
    project_trusted: AtomicU8,
}

#[derive(Default)]
struct GrantState {
    project_rights: Vec<AccessRight>,
    project_source: Option<String>,
    session_rights: Vec<AccessRight>,
    pending: HashMap<String, PendingApproval>,
}

struct PendingApproval {
    project_candidate: Option<Vec<AccessRight>>,
    session_candidate: Vec<AccessRight>,
    expected_project_source: Option<String>,
    response: async_channel::Sender<Result<ApprovalResult, String>>,
}

struct ApprovalResult {
    scope: &'static str,
    policy_path: String,
}

struct PendingGuard {
    shared: Arc<Shared>,
    id: String,
    armed: bool,
}

impl PendingGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        if self.armed {
            cancel_pending(&self.shared, &self.id);
        }
    }
}

type PreparedApproval = (
    String,
    ApprovalPrompt,
    async_channel::Receiver<Result<ApprovalResult, String>>,
);

#[derive(Debug, Deserialize, Serialize)]
struct ProjectPolicyRecord {
    version: u8,
    cwd: PathBuf,
    device: u64,
    inode: u64,
    rights: Vec<AccessRight>,
}

pub(crate) fn channel(
    project: &Path,
    home: &Path,
    data_root: &Path,
    agent_state: &Path,
    temporary: &Path,
    policy_validator: impl super::PolicyValidator + 'static,
) -> Result<(ApprovalService, ApprovalUi), String> {
    let project = canonical_directory(project, "project")?;
    let home = canonical_directory(home, "home")?;
    let data_root = canonical_directory(data_root, "application data")?;
    let agent_state = canonical_directory(agent_state, "agent state")?;
    let temporary = canonical_directory(temporary, "temporary directory")?;
    let project_policy_path = policy_path(&data_root, &project);
    let (project_rights, project_source) =
        match load_project_policy(&project_policy_path, &project, &home) {
            Ok(policy) => policy,
            Err(error) => {
                zlog::error!("Ignoring inactive project sandbox grants: {error}");
                (
                    Vec::new(),
                    read_policy_source(&project_policy_path).ok().flatten(),
                )
            }
        };
    let shared = Arc::new(Shared {
        project,
        home,
        project_policy_path,
        agent_state,
        temporary,
        policy_validator: Arc::new(policy_validator),
        state: Mutex::new(GrantState {
            project_rights,
            project_source,
            ..GrantState::default()
        }),
        filesystem_mode: AtomicU8::new(1),
        network_full: AtomicU8::new(0),
        project_trusted: AtomicU8::new(0),
    });
    let (sender, receiver) = async_channel::bounded(MAX_PENDING_APPROVALS);
    Ok((
        ApprovalService {
            shared: shared.clone(),
            prompts: sender,
        },
        ApprovalUi {
            shared,
            prompts: receiver,
        },
    ))
}

impl ApprovalService {
    pub(crate) async fn request_access(
        &self,
        params: RequestAccessParams,
    ) -> Result<String, String> {
        let prepared = prepare_request(&self.shared, params)?;
        let Some((id, prompt, response)) = prepared else {
            return Ok("All requested rights are already active. No command was retried.".into());
        };
        let mut pending = PendingGuard {
            shared: self.shared.clone(),
            id,
            armed: true,
        };
        if self.prompts.send(prompt).await.is_err() {
            return Err("Farcaster approval UI is unavailable. No command was retried.".into());
        }
        let result = response
            .recv()
            .await
            .map_err(|_| "Sandbox approval was cancelled. No command was retried.".to_owned())??;
        pending.disarm();
        Ok(format!(
            "Updated {} sandbox rights in {}. They activate after this agent turn ends; do not retry the denied operation in this turn.",
            result.scope, result.policy_path
        ))
    }
}

impl ApprovalUi {
    pub(crate) fn receiver(&self) -> async_channel::Receiver<ApprovalPrompt> {
        self.prompts.clone()
    }

    pub(crate) fn set_project_trusted(&self, trusted: bool) {
        self.shared
            .project_trusted
            .store(u8::from(trusted), Ordering::Release);
    }

    pub(crate) fn grants(&self) -> GrantStore {
        GrantStore {
            shared: self.shared.clone(),
        }
    }

    pub(crate) fn respond(&self, id: &str, choice: &str) -> Result<bool, String> {
        let pending = {
            let mut state = self
                .shared
                .state
                .lock()
                .map_err(|_| "sandbox grant state is unavailable".to_owned())?;
            state.pending.remove(id)
        };
        let Some(pending) = pending else {
            return Ok(false);
        };
        if choice == DENY_LABEL {
            let _ = pending
                .response
                .try_send(Err("Sandbox access denied. No command was retried.".into()));
            return Ok(false);
        }
        let response = pending.response.clone();
        let result = apply_approval(&self.shared, pending, choice);
        let activated = result.is_ok();
        let _ = response.try_send(result);
        Ok(activated)
    }

    pub(crate) fn cancel(&self, id: &str) -> bool {
        let Ok(mut state) = self.shared.state.lock() else {
            return false;
        };
        let Some(pending) = state.pending.remove(id) else {
            return false;
        };
        let _ = pending
            .response
            .try_send(Err("Sandbox access denied. No command was retried.".into()));
        true
    }

    pub(crate) fn cancel_all(&self) {
        let pending = self
            .shared
            .state
            .lock()
            .map(|mut state| state.pending.drain().map(|(_, pending)| pending).collect())
            .unwrap_or_else(|_| Vec::<PendingApproval>::new());
        for pending in pending {
            let _ = pending.response.try_send(Err(
                "Sandbox approval was cancelled. No command was retried.".into(),
            ));
        }
    }
}

fn apply_approval(
    shared: &Shared,
    pending: PendingApproval,
    choice: &str,
) -> Result<ApprovalResult, String> {
    let mut state = shared
        .state
        .lock()
        .map_err(|_| "sandbox grant state is unavailable".to_owned())?;
    match choice {
        SESSION_LABEL => {
            validate_rights(
                shared,
                &merged(&state.project_rights, &pending.session_candidate),
            )?;
            state.session_rights = pending.session_candidate;
            Ok(ApprovalResult {
                scope: "session",
                policy_path: "all agents in the current Farcaster process".into(),
            })
        }
        PROJECT_LABEL => {
            let candidate = pending.project_candidate.ok_or_else(|| {
                "Host-specific filesystem paths can be approved only for this session".to_owned()
            })?;
            validate_rights(shared, &merged(&candidate, &state.session_rights))?;
            let current_source = read_policy_source(&shared.project_policy_path)?;
            if current_source != pending.expected_project_source {
                return Err(
                    "Sandbox access policy changed while request_access was awaiting approval"
                        .into(),
                );
            }
            let source =
                save_project_policy(&shared.project_policy_path, &shared.project, &candidate)?;
            state.project_rights = candidate;
            state.project_source = Some(source);
            Ok(ApprovalResult {
                scope: "project",
                policy_path: shared.project_policy_path.display().to_string(),
            })
        }
        _ => Err("Unknown sandbox approval choice".into()),
    }
}

fn prepare_request(
    shared: &Arc<Shared>,
    params: RequestAccessParams,
) -> Result<Option<PreparedApproval>, String> {
    if shared.project_trusted.load(Ordering::Acquire) == 0 {
        return Err("Sandbox access can be changed only for a trusted project".into());
    }
    if params.rights.is_empty() || params.rights.len() > MAX_RIGHTS_PER_REQUEST {
        return Err("request_access accepts from 1 to 32 rights".into());
    }
    if params.reason.trim().is_empty() || params.reason.chars().count() > 2_000 {
        return Err("request_access reason must contain from 1 to 2000 characters".into());
    }
    let normalized = params
        .rights
        .iter()
        .map(|right| normalize_right(right, &shared.project, &shared.home))
        .collect::<Result<Vec<_>, _>>()?;
    if shared.filesystem_mode.load(Ordering::Acquire) == 0
        && normalized.iter().any(|right| {
            matches!(
                right,
                AccessRight::Filesystem {
                    access: FilesystemRightAccess::Write,
                    ..
                }
            )
        })
    {
        return Err("Filesystem writes cannot be granted while Files is Read-only".into());
    }
    let mut state = shared
        .state
        .lock()
        .map_err(|_| "sandbox grant state is unavailable".to_owned())?;
    if state.pending.len() >= MAX_PENDING_APPROVALS {
        return Err("Too many sandbox approvals are already pending".into());
    }
    let effective = merged(&state.project_rights, &state.session_rights);
    let additions = normalized
        .into_iter()
        .filter(|right| !right_is_active(shared, right, &effective))
        .collect::<Vec<_>>();
    if additions.is_empty() {
        return Ok(None);
    }
    let session_candidate = merged(&state.session_rights, &additions);
    let project_allowed = additions
        .iter()
        .all(|right| project_persistable(right, &shared.project, &shared.home));
    let project_candidate = project_allowed.then(|| merged(&state.project_rights, &additions));
    if session_candidate.len() > MAX_STORED_RIGHTS
        || project_candidate
            .as_ref()
            .is_some_and(|rights| rights.len() > MAX_STORED_RIGHTS)
    {
        return Err("Sandbox policy accepts at most 256 rights".into());
    }
    let id = format!(
        "farcaster-access-{}",
        REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let (response_tx, response_rx) = async_channel::bounded(1);
    let mut options = vec![SESSION_LABEL.into()];
    if project_candidate.is_some() {
        options.push(PROJECT_LABEL.into());
    }
    options.push(DENY_LABEL.into());
    let title = format!(
        "Grant sandbox rights\n{}\n\nReason: {}",
        summarize(&additions),
        params.reason.trim()
    );
    let expected_project_source = state.project_source.clone();
    state.pending.insert(
        id.clone(),
        PendingApproval {
            project_candidate,
            session_candidate,
            expected_project_source,
            response: response_tx,
        },
    );
    Ok(Some((
        id.clone(),
        ApprovalPrompt { id, title, options },
        response_rx,
    )))
}

impl GrantStore {
    pub(crate) fn set_access(
        &self,
        filesystem: super::FilesystemAccess,
        network: super::NetworkAccess,
    ) {
        let filesystem = match filesystem {
            super::FilesystemAccess::ReadOnly => 0,
            super::FilesystemAccess::Sandboxed => 1,
            super::FilesystemAccess::Full => 2,
        };
        self.shared
            .filesystem_mode
            .store(filesystem, Ordering::Release);
        self.shared.network_full.store(
            u8::from(matches!(network, super::NetworkAccess::Full)),
            Ordering::Release,
        );
    }

    pub(crate) fn resolve(&self) -> ResolvedGrants {
        let rights = self
            .shared
            .state
            .lock()
            .map(|state| merged(&state.project_rights, &state.session_rights))
            .unwrap_or_default();
        resolve_rights(&self.shared, &rights)
    }
}

fn resolve_rights(shared: &Shared, rights: &[AccessRight]) -> ResolvedGrants {
    let mut grants = ResolvedGrants::default();
    for right in rights {
        let Ok(right) = normalize_right(right, &shared.project, &shared.home) else {
            continue;
        };
        match right {
            AccessRight::Filesystem {
                access,
                path,
                scope,
            } => {
                let Ok(path) = expand_path(&path, &shared.project, &shared.home) else {
                    continue;
                };
                match access {
                    FilesystemRightAccess::Read => grants.readable.push(path),
                    FilesystemRightAccess::Write => match scope {
                        FilesystemScope::File => grants.writable_files.push(path),
                        FilesystemScope::Tree => grants.writable.push(path),
                    },
                }
            }
            AccessRight::NetworkHost { host } => grants.network_hosts.push(host),
            AccessRight::NetworkEndpoint { port, .. } => grants.loopback_ports.push(port),
        }
    }
    grants.readable.sort();
    grants.readable.dedup();
    grants.writable.sort();
    grants.writable.dedup();
    grants.writable_files.sort();
    grants.writable_files.dedup();
    grants.network_hosts.sort();
    grants.network_hosts.dedup();
    grants.loopback_ports.sort_unstable();
    grants.loopback_ports.dedup();
    grants
}

fn validate_rights(shared: &Shared, rights: &[AccessRight]) -> Result<(), String> {
    let filesystem = match shared.filesystem_mode.load(Ordering::Acquire) {
        0 => super::FilesystemAccess::ReadOnly,
        2 => super::FilesystemAccess::Full,
        _ => super::FilesystemAccess::Sandboxed,
    };
    let network = if shared.network_full.load(Ordering::Acquire) == 1 {
        super::NetworkAccess::Full
    } else {
        super::NetworkAccess::Sandboxed
    };
    let access = super::AccessPolicy {
        filesystem,
        network,
    };
    if access.unrestricted() {
        return Ok(());
    }
    let policy = super::policy::compile(
        &shared.project,
        &shared.home,
        &shared.agent_state,
        &shared.temporary,
        access,
        resolve_rights(shared, rights),
        &crate::network::NetworkConfiguration::default(),
    )?;
    shared.policy_validator.validate(&policy)
}

fn normalize_right(
    right: &AccessRight,
    project: &Path,
    home: &Path,
) -> Result<AccessRight, String> {
    match right {
        AccessRight::Filesystem {
            access,
            path,
            scope,
        } => {
            if path.is_empty() || path.len() > 1_024 || path.contains('\0') {
                return Err("filesystem path must contain from 1 to 1024 characters".into());
            }
            let lexical = expand_path(path, project, home)?;
            assert_symlink_free(&lexical)?;
            let actual = lexical.canonicalize().map_err(|error| {
                format!("resolve filesystem grant {}: {error}", lexical.display())
            })?;
            let metadata = actual.metadata().map_err(|error| {
                format!("inspect filesystem grant {}: {error}", actual.display())
            })?;
            if is_protected_path(&actual, *access, project, home) {
                return Err(format!(
                    "Sandbox policy cannot grant protected {} access: {}",
                    match access {
                        FilesystemRightAccess::Read => "read",
                        FilesystemRightAccess::Write => "write",
                    },
                    actual.display()
                ));
            }
            if metadata.is_dir() != matches!(scope, FilesystemScope::Tree) {
                return Err(format!(
                    "filesystem {} scope does not match the existing path type: {}",
                    match scope {
                        FilesystemScope::File => "file",
                        FilesystemScope::Tree => "tree",
                    },
                    actual.display()
                ));
            }
            Ok(AccessRight::Filesystem {
                access: *access,
                path: portable_path(&actual, project, home),
                scope: *scope,
            })
        }
        AccessRight::NetworkHost { host } => Ok(AccessRight::NetworkHost {
            host: normalize_host(host, false)?,
        }),
        AccessRight::NetworkEndpoint { host, port } => {
            let host = normalize_host(host, true)?;
            if *port == 0 {
                return Err("network_endpoint port must be from 1 to 65535".into());
            }
            if !matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1") {
                return Err("network_endpoint host must be localhost, 127.0.0.1, or ::1".into());
            }
            Ok(AccessRight::NetworkEndpoint {
                host: "localhost".into(),
                port: *port,
            })
        }
    }
}

fn is_protected_path(
    path: &Path,
    access: FilesystemRightAccess,
    project: &Path,
    home: &Path,
) -> bool {
    let inside = |root: &Path| path == root || path.starts_with(root);
    let hard_roots = [".ssh", ".aws", ".gnupg", ".nono", ".config/pi-nono"];
    let hard_files = [
        ".pi/agent/auth.json",
        ".pi/agent/extensions/sandbox.json",
        ".codex/auth.json",
    ];
    if hard_roots.iter().any(|root| inside(&home.join(root)))
        || hard_files.iter().any(|file| path == home.join(file))
        || inside(Path::new("/dev"))
    {
        return true;
    }
    if matches!(access, FilesystemRightAccess::Write)
        && ([".pi", ".codex"]
            .iter()
            .any(|root| inside(&home.join(root)))
            || inside(&project.join(".pi"))
            || inside(&project.join(".git")))
    {
        return true;
    }
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        name == ".env"
            || name.starts_with(".env.")
            || name.ends_with(".pem")
            || name.ends_with(".key")
    })
}

fn normalize_host(host: &str, allow_loopback: bool) -> Result<String, String> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.parse::<IpAddr>().is_ok() {
        return Ok(host);
    }
    if host.is_empty()
        || host.len() > 253
        || host.contains(['/', ':', '*', ' '])
        || host.starts_with('.')
        || host.ends_with('.')
    {
        if allow_loopback && host == "::1" {
            return Ok(host);
        }
        return Err(
            "network host must be one exact hostname or IP without scheme, port, path, or wildcard"
                .into(),
        );
    }
    if host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    }) {
        Ok(host)
    } else {
        Err("network host is invalid".into())
    }
}

fn right_is_active(shared: &Shared, right: &AccessRight, effective: &[AccessRight]) -> bool {
    if effective.contains(right) {
        return true;
    }
    match right {
        AccessRight::Filesystem { access, path, .. } => {
            let mode = shared.filesystem_mode.load(Ordering::Acquire);
            if mode == 2 {
                return true;
            }
            let Ok(path) = expand_path(path, &shared.project, &shared.home) else {
                return false;
            };
            path.starts_with(&shared.project)
                && (mode == 1 || matches!(access, FilesystemRightAccess::Read))
        }
        AccessRight::NetworkHost { host } => {
            shared.network_full.load(Ordering::Acquire) == 1
                || crate::network::base_network_host_allowed(host)
        }
        AccessRight::NetworkEndpoint { port, .. } => {
            shared.network_full.load(Ordering::Acquire) == 1
                || crate::network::base_loopback_port_allowed(*port)
        }
    }
}

fn project_persistable(right: &AccessRight, project: &Path, home: &Path) -> bool {
    match right {
        AccessRight::Filesystem { path, .. } => expand_path(path, project, home)
            .is_ok_and(|path| path.starts_with(project) || path.starts_with(home)),
        _ => true,
    }
}

fn merged(left: &[AccessRight], right: &[AccessRight]) -> Vec<AccessRight> {
    let mut merged = left.to_vec();
    for item in right {
        if !merged.contains(item) {
            merged.push(item.clone());
        }
    }
    merged.sort_by_key(right_key);
    merged
}

fn right_key(right: &AccessRight) -> String {
    serde_json::to_string(right).unwrap_or_default()
}

fn summarize(rights: &[AccessRight]) -> String {
    rights
        .iter()
        .map(|right| match right {
            AccessRight::Filesystem {
                access,
                path,
                scope,
            } => format!(
                "  {:<8}{:<11}{}",
                match access {
                    FilesystemRightAccess::Read => "read",
                    FilesystemRightAccess::Write => "write",
                },
                match scope {
                    FilesystemScope::File => "file",
                    FilesystemScope::Tree => "directory",
                },
                path
            ),
            AccessRight::NetworkHost { host } => format!("  network host       {host}"),
            AccessRight::NetworkEndpoint { host, port } => {
                format!("  network endpoint   {host}:{port}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn expand_path(value: &str, project: &Path, home: &Path) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if path.is_absolute() {
        return Ok(path.to_owned());
    }
    if value == "~" {
        return Ok(home.to_owned());
    }
    if let Some(relative) = value.strip_prefix("~/") {
        return safe_join(home, relative);
    }
    safe_join(project, value)
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let mut output = root.to_owned();
    for component in Path::new(relative).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => output.push(part),
            _ => return Err("filesystem path must not escape its root".into()),
        }
    }
    Ok(output)
}

fn portable_path(path: &Path, project: &Path, home: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(project) {
        return if relative.as_os_str().is_empty() {
            ".".into()
        } else {
            relative.to_string_lossy().into_owned()
        };
    }
    if let Ok(relative) = path.strip_prefix(home) {
        return if relative.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", relative.display())
        };
    }
    path.to_string_lossy().into_owned()
}

fn assert_symlink_free(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        let metadata = current
            .symlink_metadata()
            .map_err(|error| format!("inspect {}: {error}", current.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "filesystem grants cannot contain symlinks: {}",
                current.display()
            ));
        }
    }
    Ok(())
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("resolve sandbox {label}: {error}"))?;
    if !path.is_dir() {
        return Err(format!(
            "sandbox {label} is not a directory: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn policy_path(data_root: &Path, project: &Path) -> PathBuf {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in project.as_os_str().as_encoded_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    data_root
        .join("sandbox/projects")
        .join(format!("{hash:016x}.json"))
}

fn load_project_policy(
    path: &Path,
    project: &Path,
    home: &Path,
) -> Result<(Vec<AccessRight>, Option<String>), String> {
    let Some(source) = read_policy_source(path)? else {
        return Ok((Vec::new(), None));
    };
    let record: ProjectPolicyRecord = serde_json::from_str(&source)
        .map_err(|error| format!("parse project sandbox policy {}: {error}", path.display()))?;
    let (device, inode) = workspace_identity(project)?;
    if record.version != 1
        || record.cwd != project
        || record.device != device
        || record.inode != inode
    {
        return Err("Project sandbox policy identity does not match the active workspace".into());
    }
    if record.rights.len() > MAX_STORED_RIGHTS {
        return Err("Project sandbox policy accepts at most 256 rights".into());
    }
    let rights = record
        .rights
        .iter()
        .map(|right| normalize_right(right, project, home))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((rights, Some(source)))
}

fn read_policy_source(path: &Path) -> Result<Option<String>, String> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(format!(
            "project sandbox policy is not a regular file: {}",
            path.display()
        )),
        Ok(_) => fs::read_to_string(path)
            .map(Some)
            .map_err(|error| format!("read project sandbox policy {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "inspect project sandbox policy {}: {error}",
            path.display()
        )),
    }
}

fn save_project_policy(
    path: &Path,
    project: &Path,
    rights: &[AccessRight],
) -> Result<String, String> {
    let (device, inode) = workspace_identity(project)?;
    let source = serde_json::to_string_pretty(&ProjectPolicyRecord {
        version: 1,
        cwd: project.to_owned(),
        device,
        inode,
        rights: rights.to_vec(),
    })
    .map(|source| format!("{source}\n"))
    .map_err(|error| format!("encode project sandbox policy: {error}"))?;
    let parent = path
        .parent()
        .ok_or_else(|| "project policy path has no parent".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("create project sandbox policy directory: {error}"))?;
    assert_policy_directory(parent)?;
    let temporary = parent.join(format!(
        ".policy-{}-{}.tmp",
        std::process::id(),
        REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("create project sandbox policy: {error}"))?;
    use std::io::Write as _;
    file.write_all(source.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("write project sandbox policy: {error}"))?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("activate project sandbox policy: {error}"));
    }
    Ok(source)
}

fn assert_policy_directory(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "project policy directory has no sandbox parent".to_owned())?;
    for directory in [parent, path] {
        let metadata = directory.symlink_metadata().map_err(|error| {
            format!(
                "inspect project sandbox policy directory {}: {error}",
                directory.display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "project sandbox policy directory is not a real directory: {}",
                directory.display()
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn workspace_identity(project: &Path) -> Result<(u64, u64), String> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata = project
        .metadata()
        .map_err(|error| format!("inspect project identity: {error}"))?;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
fn workspace_identity(_project: &Path) -> Result<(u64, u64), String> {
    Ok((0, 0))
}

fn cancel_pending(shared: &Shared, id: &str) {
    if let Ok(mut state) = shared.state.lock() {
        state.pending.remove(id);
    }
}

#[cfg(test)]
#[path = "approval_tests.rs"]
mod tests;
