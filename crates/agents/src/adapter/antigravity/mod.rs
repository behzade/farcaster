mod backend;
use super::acp::AcpProfile;
use crate::Backend;
pub(super) use backend::AntigravityAdapter;

pub(super) const PROFILE: AcpProfile = AcpProfile {
    backend: Backend::Antigravity,
    name: "Antigravity",
    command: if cfg!(windows) { "agy.exe" } else { "agy" },
    path_environment: "FARCASTER_ANTIGRAVITY_ACP_PATH",
    arguments: &[],
    auth_method: None,
    force_argument: None,
    resume_method: "session/resume",
    permission_modes: Some(("default", "yolo")),
};

pub(super) fn descriptor() -> crate::contract::AgentBackendDescriptor {
    super::acp::backend::descriptor(&PROFILE, false)
}

fn conversation_project(session_id: &str) -> Option<std::path::PathBuf> {
    let home = std::env::var_os("GEMINI_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".gemini"))
        })?;
    conversation_project_in(&home, session_id)
}

fn conversation_project_in(home: &std::path::Path, session_id: &str) -> Option<std::path::PathBuf> {
    let meta = home
        .join("antigravity-acp")
        .join("conversations")
        .join(format!("{session_id}.meta"));
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&meta).ok()?).ok()?;
    Some(std::path::PathBuf::from(value.get("cwd")?.as_str()?))
}

pub(super) fn load_history(
    config: &crate::AgentLaunchConfig,
    path: &std::path::Path,
    fallback_project: &std::path::Path,
) -> Result<crate::DiscoveredHistory, String> {
    let id = super::main_session::external_session_locator(PROFILE.backend, path)
        .ok_or_else(|| format!("invalid Antigravity session locator: {}", path.display()))?;
    let project = conversation_project(&id).unwrap_or_else(|| fallback_project.to_owned());
    super::acp::load_history(&PROFILE, config, path, &project)
}

pub(super) fn account_identity(
    response: &serde_json::Value,
) -> Option<crate::accounts::AccountIdentity> {
    let plan = response
        .pointer("/_meta/antigravity/tier/name")?
        .as_str()?
        .trim();
    if plan.is_empty() {
        return None;
    }
    Some(crate::accounts::AccountIdentity {
        authority: "google".into(),
        plan: Some(plan.into()),
        ..Default::default()
    })
}

pub(super) fn observe_account(
    config: &crate::AgentLaunchConfig,
    command: &std::process::Command,
    project: &std::path::Path,
    identity: crate::accounts::AccountIdentity,
) {
    use crate::accounts::*;
    let env = |name| super::process_command::environment(command, name);
    let Some(home) = env("HOME").or_else(|| env("USERPROFILE")) else {
        return;
    };
    let source = super::accounts::source(
        config,
        Backend::Antigravity,
        project,
        "google",
        format!(
            "native:{}",
            std::path::PathBuf::from(home)
                .join(".gemini")
                .join("antigravity-acp")
                .display()
        ),
    );
    publish_account_event(AccountEvent::Observed {
        observation: AccountObservation {
            source,
            identity: Some(identity),
            auth: AccountAuthState::SignedIn,
            limits: Vec::new(),
            complete_limits: true,
            limits_failed: false,
            observed_at: account_now(),
            capabilities: Default::default(),
        },
        run_id: None,
        evidence: AccountBindingEvidence::ObservedAtStartup,
    });
}

pub(super) fn integrated_cli(command: &std::process::Command) -> bool {
    std::path::Path::new(command.get_program()).file_stem() == Some("agy".as_ref())
}

pub(super) fn configure(
    command: &mut std::process::Command,
    mode: crate::HarnessAccessMode,
) -> Result<(), String> {
    if integrated_cli(command) {
        // Keep the existing Farcaster session IDs and conversation directory.
        command.args(["acp", "--app-data-dir", "antigravity-acp"]);
        if mode == crate::HarnessAccessMode::Full {
            command.arg("--dangerously-skip-permissions");
        } else {
            command.arg("--sandbox");
        }
        return Ok(());
    }
    if cfg!(target_os = "linux") {
        command.arg("--uid=");
    }
    let helper = std::path::Path::new(command.get_program())
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(if cfg!(windows) {
            "localharness_external.exe"
        } else {
            "localharness_external"
        });
    if !helper.is_file() {
        return Err(format!(
            "Antigravity ACP requires its matching helper beside the executable: {}",
            helper.display()
        ));
    }
    command.env("ANTIGRAVITY_HARNESS_PATH", helper);
    Ok(())
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
