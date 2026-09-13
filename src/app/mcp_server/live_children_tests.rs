//! Opt-in, billable child-worker checks. These tests deliberately use the real
//! backend executable and model selected by `FARCASTER_E2E_HARNESS`.
use crate::agents::Backend;

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use super::workers::{self, SendParams};
use crate::{
    agent_activity::{AgentLifecycle, AgentOutcome},
    agents::{
        AgentLaunchConfig, CallerIdentity, CallerProfile, CallerRegistry, HarnessAccessMode,
        WorkerExecution, WorkerInput, WorkerInputResponse, WorkerPool, WorkerProfile,
        WorkerProfiles, WorkerSnapshot, WorkerStatus,
    },
    app::{
        runtime::catalog::live_worker_activities,
        views::run_panel::{agents::AgentSection, live_run_panel_agent_rows},
    },
    protocol::{ExtensionUiRequest, ExtensionUiResponse},
};

const TURN_TIMEOUT: Duration = Duration::from_secs(180);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const SHELL_GATE_HOLD_SECS: u64 = 480;
static LIVE_CHILD_LOCK: Mutex<()> = Mutex::new(());

#[test]
#[ignore = "runs a real selected harness/model through production child-worker routing"]
fn live_e2e_child_creation_visibility_report_and_named_follow_up() -> Result<(), String> {
    let _serial = LIVE_CHILD_LOCK.lock().expect("live child test lock");
    let harness = selected_harness()?;
    let fixture = LiveChildFixture::new(harness)?;
    let first_marker = marker("CHILD_FIRST");
    let first_gate = ShellGate::new(&fixture.project, "child-first")?;
    let child = fixture.start_child("live-child", first_gate.prompt(&first_marker))?;

    fixture.wait_for_gate(
        &child,
        &fixture.parent_session,
        &first_gate,
        "initial child shell command",
    )?;
    fixture.require_running_in_catalog_and_sidebar(&child)?;
    first_gate.require_holding()?;
    first_gate.release()?;
    fixture.require_report_once(&first_marker)?;
    let original = fixture.require_settled_in_catalog_and_sidebar(&child, &first_marker)?;

    let follow_up = marker("CHILD_FOLLOW_UP");
    let follow_gate = ShellGate::new(&fixture.project, "child-follow-up")?;
    let reused = fixture.send_to_child("live-child", follow_gate.prompt(&follow_up), None)?;
    if reused["created"] != false || reused["queued"] != true {
        return Err(format!("named child was not reused: {reused}"));
    }
    fixture.wait_for_gate(
        &child,
        &fixture.parent_session,
        &follow_gate,
        "named-child follow-up shell command",
    )?;
    let resumed = fixture.wait_snapshot(&child.id, TURN_TIMEOUT, |snapshot| {
        snapshot.status == WorkerStatus::Running && snapshot.session_locator.is_some()
    })?;
    let reused_identity = fixture.require_catalog_and_sidebar(
        &resumed,
        AgentLifecycle::Working,
        AgentSection::Active,
    )?;
    if reused_identity != original {
        return Err(format!(
            "named follow-up forked or replaced child identity: {original:?} -> {reused_identity:?}"
        ));
    }
    follow_gate.require_holding()?;
    follow_gate.release()?;
    fixture.require_report_once(&follow_up)?;
    fixture.require_settled_in_catalog_and_sidebar(&child, &follow_up)?;
    Ok(())
}

#[test]
#[ignore = "requires the real selected harness/model to issue a native child input request"]
fn live_e2e_child_needs_input_projects_to_catalog_and_sidebar() -> Result<(), String> {
    let _serial = LIVE_CHILD_LOCK.lock().expect("live child test lock");
    let harness = selected_harness()?;
    crate::agents::live_e2e_support::require_native_input_support(harness)?;
    let questions = crate::agents::live_e2e_support::native_questions_available(harness)?;
    let approvals = crate::agents::live_e2e_support::native_approvals_available(harness)?;
    let fixture = LiveChildFixture::new(harness)?;
    let mut approval_probe = None;
    let initial_prompt = if questions {
        native_question_prompt()
    } else {
        approval_probe = Some(NativeInputProbe::new(&fixture.project)?);
        approval_probe
            .as_ref()
            .expect("approval probe was initialized")
            .prompt()
    };
    let child = fixture.start_child("live-input", initial_prompt)?;
    let pending = match fixture.wait_for_native_input(&child.id, TURN_TIMEOUT, None)? {
        NativeInputObservation::Requested(snapshot) => snapshot,
        NativeInputObservation::SettledWithoutRequest(limit) if questions && approvals => {
            eprintln!(
                "E2E_LIMIT: {harness} declares native questions, but the live child settled without one; retrying its declared approval path on the same child: {limit}"
            );
            let before_retry = fixture.snapshot(&child.id)?;
            let original = before_retry
                .session_locator
                .clone()
                .ok_or("settled question attempt did not retain a native child locator")?;
            approval_probe = Some(NativeInputProbe::new(&fixture.project)?);
            let retry = fixture.send_to_child(
                "live-input",
                approval_probe
                    .as_ref()
                    .expect("approval probe was initialized")
                    .prompt(),
                None,
            )?;
            if retry["created"] != false || retry["queued"] != true {
                return Err(format!(
                    "E2E_LIMIT approval retry did not reuse the settled child: {retry}"
                ));
            }
            match fixture.wait_for_native_input(&child.id, TURN_TIMEOUT, Some(&before_retry))? {
                NativeInputObservation::Requested(snapshot) => {
                    if snapshot.session_locator.as_deref() != Some(original.as_str()) {
                        return Err(format!(
                            "E2E_LIMIT approval retry forked the child session: {original:?} -> {:?}",
                            snapshot.session_locator
                        ));
                    }
                    snapshot
                }
                NativeInputObservation::SettledWithoutRequest(retry_limit) => {
                    return Err(format!(
                        "E2E_BLOCKED: native question and approval stimuli both settled without a native input request; question={limit}; approval={retry_limit}"
                    ));
                }
            }
        }
        NativeInputObservation::SettledWithoutRequest(limit) => return Err(limit),
    };
    fixture.require_catalog_and_sidebar(
        &pending,
        AgentLifecycle::NeedsInput,
        AgentSection::Active,
    )
    .map_err(|error| {
        format!("native input reached the child pool, but catalog/sidebar projection failed: {error}")
    })?;
    Ok(())
}

#[test]
#[ignore = "runs two real selected-harness child families and verifies production family stop isolation"]
fn live_e2e_child_family_abort_stops_only_its_family_and_fences_restart() -> Result<(), String> {
    let _serial = LIVE_CHILD_LOCK.lock().expect("live child test lock");
    let harness = selected_harness()?;
    let fixture = LiveChildFixture::new(harness)?;
    let unrelated = fixture.second_parent()?;
    let first_gate = ShellGate::new(&fixture.project, "family-a")?;
    let second_gate = ShellGate::new(&fixture.project, "family-b")?;
    let first =
        fixture.start_child("family-a", first_gate.prompt("FARCASTER_FAMILY_A_RELEASED"))?;
    let second = fixture.start_child_for(
        &unrelated,
        "family-b",
        second_gate.prompt("FARCASTER_FAMILY_B_RELEASED"),
    )?;
    fixture.wait_for_gate(
        &first,
        &fixture.parent_session,
        &first_gate,
        "family-a shell command",
    )?;
    fixture.wait_for_gate(
        &second,
        &unrelated.session,
        &second_gate,
        "family-b shell command",
    )?;
    fixture.wait_snapshot(&first.id, TURN_TIMEOUT, |snapshot| {
        snapshot.status == WorkerStatus::Running && snapshot.session_locator.is_some()
    })?;
    fixture.wait_snapshot(&second.id, TURN_TIMEOUT, |snapshot| {
        snapshot.status == WorkerStatus::Running && snapshot.session_locator.is_some()
    })?;
    first_gate.require_holding()?;
    second_gate.require_holding()?;
    fixture.require_gate_is_running(&first, &first_gate)?;
    fixture.require_gate_is_running(&second, &second_gate)?;

    let stopped = fixture.pool.stop_session_family(
        &fixture.project,
        &[(
            fixture.harness.to_owned(),
            PathBuf::from(&fixture.parent_session),
        )],
    )?;
    if stopped != 1 {
        return Err(format!(
            "family abort stopped {stopped} workers, expected only family-a"
        ));
    }
    let after_stop = fixture.wait_snapshot(&first.id, Duration::from_secs(30), |snapshot| {
        snapshot.status == WorkerStatus::Stopped
    })?;
    if after_stop.status != WorkerStatus::Stopped {
        return Err(format!("family-a did not stop: {after_stop:?}"));
    }
    first_gate.require_not_timed_out()?;
    second_gate.require_holding()?;
    let unrelated_snapshot = fixture.snapshot(&second.id)?;
    if unrelated_snapshot.status != WorkerStatus::Running {
        return Err(format!(
            "family abort changed or naturally settled unrelated family-b worker: {unrelated_snapshot:?}"
        ));
    }
    let restart = fixture.send_to_child(
        "family-a",
        "This must not restart while the family stop is pending.".into(),
        None,
    );
    if !restart
        .as_ref()
        .err()
        .is_some_and(|error| error.contains("family is stopping"))
    {
        return Err(format!(
            "stopped family accepted or misreported a follow-up: {restart:?}"
        ));
    }
    thread::sleep(Duration::from_millis(250));
    if fixture.snapshot(&first.id)?.status != WorkerStatus::Stopped {
        return Err("stopped child restarted without a new family request".into());
    }

    fixture.pool.finish_session_family_stop(
        &fixture.project,
        &[(
            fixture.harness.to_owned(),
            PathBuf::from(&fixture.parent_session),
        )],
    )?;
    let stopped_unrelated = fixture.pool.stop_session_family(
        &fixture.project,
        &[(
            fixture.harness.to_owned(),
            PathBuf::from(&unrelated.session),
        )],
    )?;
    if stopped_unrelated != 1 {
        return Err(format!(
            "cleanup stopped {stopped_unrelated} unrelated workers"
        ));
    }
    second_gate.release()?;
    fixture.pool.finish_session_family_stop(
        &fixture.project,
        &[(
            fixture.harness.to_owned(),
            PathBuf::from(&unrelated.session),
        )],
    )
}

struct LiveChildFixture {
    harness: Backend,
    model_identity: String,
    pool: WorkerPool,
    profiles: WorkerProfiles,
    parent: CallerIdentity,
    parent_session: String,
    access_mode: HarnessAccessMode,
    _family_persistence: FamilyPersistence,
    project: PathBuf,
    _project_guard: tempfile::TempDir,
}

struct LiveParent {
    identity: CallerIdentity,
    session: String,
}

#[derive(Debug, Eq, PartialEq)]
struct ChildIdentity {
    worker_id: String,
    backend: Backend,
    session_locator: String,
    session_path: PathBuf,
}

struct NativeInputProbe {
    path: PathBuf,
    file: String,
}

impl NativeInputProbe {
    fn new(project: &Path) -> Result<Self, String> {
        let file = format!(
            "farcaster-live-native-input-{}.txt",
            uuid::Uuid::new_v4().simple()
        );
        let path = project.join(&file);
        std::fs::write(&path, "native input probe\n")
            .map_err(|error| format!("create native input probe: {error}"))?;
        Ok(Self { path, file })
    }

    fn prompt(&self) -> String {
        format!(
            "Use the native shell-command tool exactly once to run `rm {file}`. This file is a disposable test fixture in the current project directory. Do not answer in plain text, use another tool, change any other file, or retry. In the sandboxed user-approval mode, wait for the native permission request before the command executes.",
            file = self.file,
        )
    }
}

impl Drop for NativeInputProbe {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

enum NativeInputObservation {
    Requested(WorkerSnapshot),
    SettledWithoutRequest(String),
}

fn native_question_prompt() -> String {
    "Use your native question tool now. Ask exactly `Which live test option should Farcaster choose?` with exactly two choices: `Alpha` and `Beta`. Do not run a shell command, use another tool, or answer in plain text. Wait for the native answer.".into()
}

struct FamilyPersistence;

impl FamilyPersistence {
    fn install(database: PathBuf) -> Result<Self, String> {
        crate::app::persistence::StateStore::open_at(&database)?;
        CallerRegistry::shared().set_family_sink(Some(Arc::new(move |link| {
            crate::app::persistence::StateStore::open_at(&database)?.save_worker_family(link)
        })));
        Ok(Self)
    }
}

impl Drop for FamilyPersistence {
    fn drop(&mut self) {
        CallerRegistry::shared().set_family_sink(None);
    }
}

struct ShellGate {
    script_path: PathBuf,
    started_path: PathBuf,
    release_path: PathBuf,
    timed_out_path: PathBuf,
    script_file: String,
    started_file: String,
    release_file: String,
    timed_out_file: String,
}

impl ShellGate {
    fn new(project: &Path, label: &str) -> Result<Self, String> {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let script_file = format!("farcaster-live-child-{label}-{suffix}.sh");
        let started_file = format!("farcaster-live-child-{label}-{suffix}.started");
        let release_file = format!("farcaster-live-child-{label}-{suffix}.release");
        let timed_out_file = format!("farcaster-live-child-{label}-{suffix}.timed-out");
        let script_path = project.join(&script_file);
        let started_path = project.join(&started_file);
        let release_path = project.join(&release_file);
        let timed_out_path = project.join(&timed_out_file);
        if script_path.exists()
            || started_path.exists()
            || release_path.exists()
            || timed_out_path.exists()
        {
            return Err("live child shell gate unexpectedly already exists".into());
        }
        let gate = Self {
            script_path,
            started_path,
            release_path,
            timed_out_path,
            script_file,
            started_file,
            release_file,
            timed_out_file,
        };
        std::fs::write(&gate.script_path, gate.script()).map_err(|error| {
            format!(
                "write real child shell gate {}: {error}",
                gate.script_path.display()
            )
        })?;
        Ok(gate)
    }

    fn prompt(&self, marker: &str) -> String {
        let command = self.command();
        format!(
            "Use one shell tool in the current project directory to run exactly `{command}`. Do not answer before that command exits. Then reply with the exact token {marker}.",
        )
    }

    fn command(&self) -> String {
        format!("sh ./{}", self.script_file)
    }

    fn script(&self) -> String {
        format!(
            "#!/bin/sh\nprintf started > {started}\ndeadline=$(( $(date +%s) + {hold_seconds} ))\nwhile [ ! -s {release} ]; do\n  if [ \"$(date +%s)\" -ge \"$deadline\" ]; then\n    printf timed-out > {timed_out}\n    exit 124\n  fi\n  sleep 0.1\ndone\ncat {release}\n",
            started = self.started_file,
            release = self.release_file,
            timed_out = self.timed_out_file,
            hold_seconds = SHELL_GATE_HOLD_SECS,
        )
    }

    fn registered_permission_request(
        &self,
        input: &WorkerInput,
    ) -> Result<ExtensionUiRequest, String> {
        let (child, permission) = input.prompt.split_once("\n\n").ok_or_else(|| {
            format!(
                "E2E_BLOCKED: real child permission omitted the production child wrapper: {:?}",
                input.prompt
            )
        })?;
        if !child.starts_with("Child ") {
            return Err(format!(
                "E2E_BLOCKED: real child permission had an unexpected registry wrapper: {child:?}"
            ));
        }
        Ok(ExtensionUiRequest::Select {
            id: input.id.clone(),
            title: permission.into(),
            options: input.options.clone(),
            timeout: None,
        })
    }

    fn release(&self) -> Result<(), String> {
        std::fs::write(&self.release_path, "release\n")
            .map_err(|error| format!("release real child shell gate: {error}"))
    }

    fn require_holding(&self) -> Result<(), String> {
        if !self.started_path.is_file() {
            return Err("real child shell gate has not started".into());
        }
        if self.timed_out_path.is_file() {
            return Err(
                "real child shell gate timed out before the asserted lifecycle action".into(),
            );
        }
        if self.release_path.is_file() {
            return Err(
                "real child shell gate released before the asserted lifecycle action".into(),
            );
        }
        Ok(())
    }

    fn require_not_timed_out(&self) -> Result<(), String> {
        if self.timed_out_path.is_file() {
            return Err("real child shell gate timed out instead of being stopped".into());
        }
        Ok(())
    }
}

impl Drop for ShellGate {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

impl LiveChildFixture {
    fn new(harness: Backend) -> Result<Self, String> {
        let access_mode = crate::agents::live_e2e_support::live_access_mode_for_harness(harness)?;
        let project_guard = tempfile::tempdir_in(crate::agents::live_e2e_support::e2e_case_dir()?)
            .map_err(|error| format!("create live child project: {error}"))?;
        let project = project_guard
            .path()
            .canonicalize()
            .map_err(|error| format!("canonicalize live child project: {error}"))?;
        let config = AgentLaunchConfig {
            program: PathBuf::from(harness.as_str()),
            prefix_args: Vec::new(),
            access_mode,
            app_proxy: None,
            session_locator_root: Some(crate::agents::live_e2e_support::isolated_locator_root()?),
        };
        let model = crate::agents::live_e2e_support::selected_live_worker_model(harness)?;
        let model_identity = format!("{}/{}", model.provider, model.model);
        eprintln!(
            "E2E_CHILD_START: harness={harness} model={model_identity} access_mode={access_mode:?} project={}",
            project.display(),
        );
        let profiles = WorkerProfiles {
            profiles: vec![WorkerProfile {
                name: "live".into(),
                description: "Live child-worker E2E profile.".into(),
                models: vec![WorkerExecution {
                    harness: harness.into(),
                    provider: model.provider,
                    model: model.model,
                    effort: None,
                }],
            }],
        };
        let (factories, default_backend) = crate::agents::worker_factories(config);
        let pool = WorkerPool::new(factories, default_backend, project.clone(), 2)?;
        let family_persistence = FamilyPersistence::install(project.join("state.sqlite3"))?;
        let parent = new_parent(&project, harness, "parent-a", access_mode)?;
        let parent_session = parent.session.clone();
        Ok(Self {
            harness,
            model_identity,
            pool,
            profiles,
            parent: parent.identity,
            parent_session,
            access_mode,
            _family_persistence: family_persistence,
            project,
            _project_guard: project_guard,
        })
    }

    fn second_parent(&self) -> Result<LiveParent, String> {
        new_parent(&self.project, self.harness, "parent-b", self.access_mode)
    }

    fn start_child(
        &self,
        name: &str,
        message: impl Into<String>,
    ) -> Result<WorkerSnapshot, String> {
        self.start_child_with(self.parent.token(), name, message)
    }

    fn start_child_for(
        &self,
        parent: &LiveParent,
        name: &str,
        message: impl Into<String>,
    ) -> Result<WorkerSnapshot, String> {
        self.start_child_with(parent.identity.token(), name, message)
    }

    fn start_child_with(
        &self,
        token: &str,
        name: &str,
        message: impl Into<String>,
    ) -> Result<WorkerSnapshot, String> {
        eprintln!(
            "E2E_CHILD_PHASE: harness={} model={} phase=create-child name={name}",
            self.harness, self.model_identity,
        );
        let known = self
            .pool
            .snapshots()?
            .into_iter()
            .map(|snapshot| snapshot.id)
            .collect::<HashSet<_>>();
        let response = self
            .send(token, name, message.into(), Some("live".into()))
            .map_err(|error| {
                format!(
                    "E2E_CHILD_START failed: harness={} model={} name={name}: {error}",
                    self.harness, self.model_identity,
                )
            })?;
        if response["created"] != true || response["queued"] != true {
            return Err(format!("worker_send did not create {name}: {response}"));
        }
        self.wait_new_snapshot(&known, TURN_TIMEOUT)
    }

    fn send_to_child(
        &self,
        name: &str,
        message: String,
        profile: Option<String>,
    ) -> Result<serde_json::Value, String> {
        self.send(self.parent.token(), name, message, profile)
    }

    fn send(
        &self,
        token: &str,
        name: &str,
        message: String,
        profile: Option<String>,
    ) -> Result<serde_json::Value, String> {
        workers::send(
            &self.pool,
            SendParams {
                to: Some(name.into()),
                message,
                profile,
            },
            Some(token.into()),
            &self.profiles,
            |execution, _| execution.harness == self.harness,
        )
    }

    fn wait_new_snapshot(
        &self,
        known: &HashSet<String>,
        timeout: Duration,
    ) -> Result<WorkerSnapshot, String> {
        wait_until(timeout, "new child pool snapshot", || {
            self.pool.snapshots().map(|snapshots| {
                snapshots
                    .into_iter()
                    .find(|snapshot| !known.contains(&snapshot.id))
            })
        })
    }

    fn snapshot(&self, id: &str) -> Result<WorkerSnapshot, String> {
        self.pool
            .snapshots()?
            .into_iter()
            .find(|snapshot| snapshot.id == id)
            .ok_or_else(|| format!("worker snapshot disappeared: {id}"))
    }

    fn wait_snapshot(
        &self,
        id: &str,
        timeout: Duration,
        condition: impl Fn(&WorkerSnapshot) -> bool,
    ) -> Result<WorkerSnapshot, String> {
        wait_until(timeout, "child worker state", || {
            let snapshot = self.snapshot(id)?;
            Ok(condition(&snapshot).then_some(snapshot))
        })
    }

    fn wait_for_native_input(
        &self,
        id: &str,
        timeout: Duration,
        previous_attempt: Option<&WorkerSnapshot>,
    ) -> Result<NativeInputObservation, String> {
        let deadline = Instant::now() + timeout;
        let mut observed_new_turn = previous_attempt.is_none();
        loop {
            let snapshot = self.snapshot(id)?;
            observed_new_turn |= snapshot.status == WorkerStatus::Running;
            if snapshot.status == WorkerStatus::NeedsInput && snapshot.pending_input.is_some() {
                return Ok(NativeInputObservation::Requested(snapshot));
            }
            if snapshot.status == WorkerStatus::NeedsInput {
                return Err(format!(
                    "native input projection failed: child reports NeedsInput without a payload; final snapshot={snapshot:?}"
                ));
            }
            if snapshot.status == WorkerStatus::Idle
                && (observed_new_turn
                    || previous_attempt.is_some_and(|previous| {
                        (snapshot.output.is_some() && snapshot.output != previous.output)
                            || (snapshot.error.is_some() && snapshot.error != previous.error)
                    }))
            {
                return Ok(NativeInputObservation::SettledWithoutRequest(format!(
                    "E2E_BLOCKED: {harness} child completed without a native input request; final status={status:?}, output={output:?}, error={error:?}, pending_input={pending_input:?}",
                    harness = self.harness,
                    status = snapshot.status,
                    output = snapshot.output,
                    error = snapshot.error,
                    pending_input = snapshot.pending_input,
                )));
            }
            if matches!(
                snapshot.status,
                WorkerStatus::Failed | WorkerStatus::Stopped
            ) {
                return Err(format!(
                    "child terminated before producing a native input request; final snapshot={snapshot:?}"
                ));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "E2E_BLOCKED: {harness} child did not produce a native input request within {timeout:?}; final snapshot={snapshot:?}",
                    harness = self.harness,
                ));
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    fn wait_for_gate(
        &self,
        child: &WorkerSnapshot,
        parent_session: &str,
        gate: &ShellGate,
        phase: &str,
    ) -> Result<(), String> {
        eprintln!(
            "E2E_CHILD_PHASE: harness={} model={} worker={} phase={phase} waiting-for=shell-gate",
            self.harness, self.model_identity, child.id,
        );
        let deadline = Instant::now() + TURN_TIMEOUT;
        let mut approved_gate = false;
        loop {
            if gate.started_path.is_file() {
                eprintln!(
                    "E2E_CHILD_PHASE: harness={} model={} worker={} phase={phase} shell-gate=started",
                    self.harness, self.model_identity, child.id,
                );
                return Ok(());
            }
            approved_gate |= self.approve_registered_gate(parent_session, gate, phase)?;
            let snapshot = self.snapshot(&child.id)?;
            if snapshot.status == WorkerStatus::NeedsInput && !approved_gate {
                return Err(format!(
                    "E2E_BLOCKED: child requested native input that was not the registered shell-gate approval: harness={} model={} worker={} phase={phase} final_snapshot={snapshot:?}",
                    self.harness, self.model_identity, child.id,
                ));
            }
            if matches!(
                snapshot.status,
                WorkerStatus::Failed | WorkerStatus::Stopped
            ) {
                return Err(format!(
                    "child terminated before reaching its real shell gate: harness={} model={} worker={} phase={phase} final_snapshot={snapshot:?}",
                    self.harness, self.model_identity, child.id,
                ));
            }
            // A named child may retain an older Idle snapshot until its peer
            // message reaches the run loop. Only the real gate witness proves
            // the new turn started; Idle is not a rejection signal here.
            if Instant::now() >= deadline {
                return Err(format!(
                    "timed out waiting for real child shell gate: harness={} model={} worker={} phase={phase} final_snapshot={snapshot:?} gate_started={} gate_timed_out={}",
                    self.harness,
                    self.model_identity,
                    child.id,
                    gate.started_path.is_file(),
                    gate.timed_out_path.is_file(),
                ));
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    fn approve_registered_gate(
        &self,
        parent_session: &str,
        gate: &ShellGate,
        phase: &str,
    ) -> Result<bool, String> {
        let inputs =
            CallerRegistry::shared().take_child_inputs(&self.project, self.harness, parent_session);
        if inputs.is_empty() {
            return Ok(false);
        }
        if inputs.len() != 1 {
            return Err(format!(
                "E2E_BLOCKED: real child produced {} simultaneous native input requests; the gate test only approves one exact command",
                inputs.len()
            ));
        }
        let input = &inputs[0];
        let request = gate.registered_permission_request(input)?;
        let response = crate::agents::live_e2e_support::bounded_command_permission(
            &request,
            &[gate.command()],
        )?;
        let ExtensionUiResponse::Value { id, value } = response else {
            return Err("E2E_BLOCKED: gate permission helper returned a non-value response".into());
        };
        if id != input.id {
            return Err(format!(
                "E2E_BLOCKED: gate permission helper changed the registered input id: {:?} -> {id:?}",
                input.id
            ));
        }
        CallerRegistry::shared().respond_to_child_input(WorkerInputResponse {
            id,
            value: Some(value),
            cancel: false,
        })?;
        eprintln!(
            "E2E_CHILD_PHASE: harness={} model={} phase={phase} approved=registered-shell-gate input={}",
            self.harness, self.model_identity, input.id,
        );
        Ok(true)
    }

    fn require_report_once(&self, marker: &str) -> Result<(), String> {
        let message = wait_until(TURN_TIMEOUT, "child parent report", || {
            Ok(self
                .parent
                .try_recv()
                .filter(|report| report.message.contains(marker)))
        })?;
        if message.from != "live-child" {
            return Err(format!("unexpected child report sender: {}", message.from));
        }
        thread::sleep(Duration::from_millis(250));
        if let Some(duplicate) = self.parent.try_recv() {
            return Err(format!(
                "child reported more than once for one settled turn: {duplicate:?}"
            ));
        }
        Ok(())
    }

    fn require_running_in_catalog_and_sidebar(&self, child: &WorkerSnapshot) -> Result<(), String> {
        let running = self.wait_snapshot(&child.id, Duration::from_secs(30), |snapshot| {
            snapshot.status == WorkerStatus::Running && snapshot.session_locator.is_some()
        })?;
        self.require_catalog_and_sidebar(&running, AgentLifecycle::Working, AgentSection::Active)?;
        Ok(())
    }

    fn require_gate_is_running(
        &self,
        child: &WorkerSnapshot,
        gate: &ShellGate,
    ) -> Result<(), String> {
        gate.require_holding()?;
        let snapshot = self.snapshot(&child.id)?;
        if snapshot.status != WorkerStatus::Running {
            return Err(format!(
                "child settled before the lifecycle action despite an unreleased shell gate: {snapshot:?}"
            ));
        }
        Ok(())
    }

    fn require_settled_in_catalog_and_sidebar(
        &self,
        child: &WorkerSnapshot,
        marker: &str,
    ) -> Result<ChildIdentity, String> {
        let settled = self.wait_snapshot(&child.id, TURN_TIMEOUT, |snapshot| {
            snapshot.status == WorkerStatus::Idle
                && snapshot
                    .output
                    .as_deref()
                    .is_some_and(|output| output.contains(marker))
        })?;
        self.require_catalog_and_sidebar(
            &settled,
            AgentLifecycle::Completed(AgentOutcome::Complete),
            AgentSection::Completed,
        )
    }

    fn require_catalog_and_sidebar(
        &self,
        snapshot: &WorkerSnapshot,
        lifecycle: AgentLifecycle,
        section: AgentSection,
    ) -> Result<ChildIdentity, String> {
        let locator = snapshot
            .session_locator
            .as_deref()
            .ok_or("child has no native session locator")?;
        let database = self.project.join("state.sqlite3");
        let sessions = wait_until(
            Duration::from_secs(30),
            "persisted child catalog row",
            || {
                let sessions =
                    crate::app::persistence::StateStore::open_at(&database)?.cached_sessions("")?;
                Ok(sessions
                    .iter()
                    .any(|session| {
                        session.harness == self.harness
                            && session.parent_session.as_deref()
                                == Some(self.parent_session.as_str())
                            && (session.id == locator || session.path == Path::new(locator))
                    })
                    .then_some(sessions))
            },
        )?;
        let child = sessions
            .iter()
            .find(|session| {
                session.harness == self.harness
                    && session.parent_session.as_deref() == Some(self.parent_session.as_str())
                    && (session.id == locator || session.path == Path::new(locator))
            })
            .ok_or("persisted catalog omitted the live child row")?;
        let root = crate::sessions::root_session_for_path(&sessions, Some(&child.path))
            .ok_or("persisted child has no root session")?;
        let activities = live_worker_activities(&sessions, vec![snapshot.clone()]);
        let key = crate::agent_activity::agent_activity_key(&child.path);
        let activity = activities
            .get(&key)
            .ok_or("production catalog did not match the live child pool snapshot")?;
        if activity.lifecycle != lifecycle {
            return Err(format!(
                "catalog projected wrong child lifecycle: {:?}, expected {lifecycle:?}",
                activity.lifecycle
            ));
        }
        let rows = live_run_panel_agent_rows(&sessions, &activities, Some(&root.path));
        if rows.len() != 1
            || rows[0].2.path != child.path
            || rows[0].0.session_path != child.path
            || rows[0].3 != section
        {
            return Err(format!(
                "sidebar did not select the live child row: rows={rows:?}, expected={section:?}"
            ));
        }
        Ok(ChildIdentity {
            worker_id: snapshot.id.clone(),
            backend: snapshot.backend,
            session_locator: locator.to_owned(),
            session_path: child.path.clone(),
        })
    }
}

// The parent uses the production registered-caller route rather than asking a
// model parent to choose the MCP tool. `workers::send` still resolves that
// caller, launches the real child factory, and routes its reports through the
// same registry used by a live parent turn.
fn new_parent(
    project: &Path,
    harness: Backend,
    label: &str,
    access_mode: HarnessAccessMode,
) -> Result<LiveParent, String> {
    let identity = CallerRegistry::shared().issue_with_access(
        project,
        CallerProfile {
            backend: harness.into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
        access_mode,
    );
    let session = format!("live-{label}-{}", uuid::Uuid::new_v4());
    identity.bind(session.clone());
    Ok(LiveParent { identity, session })
}

fn selected_harness() -> Result<Backend, String> {
    match crate::agents::live_e2e_support::selected_live_harnesses()?.as_slice() {
        [harness] => harness.parse(),
        harnesses => Err(format!(
            "set FARCASTER_E2E_HARNESS to one harness for child E2E; selected {}",
            harnesses.join(", ")
        )),
    }
}

fn marker(prefix: &str) -> String {
    format!("FARCASTER_{prefix}_{}", uuid::Uuid::new_v4().simple())
}

fn wait_until<T>(
    timeout: Duration,
    description: &str,
    mut check: impl FnMut() -> Result<Option<T>, String>,
) -> Result<T, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = check()? {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for {description}"));
        }
        thread::sleep(POLL_INTERVAL);
    }
}
