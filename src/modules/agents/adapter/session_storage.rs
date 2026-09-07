use std::path::{Path, PathBuf};

use crate::sessions::{
    LoadedHistory, SessionSummary, SessionTarget, SessionTransfer, TransferMember,
};

use super::{codex, cursor, main_session::external_session_locator, opencode, pi};

pub(super) fn validate_session_locator(harness: &str, path: &Path) -> Result<(), String> {
    validated_locator(harness, path).map(|_| ())
}

fn validated_locator(harness: &str, path: &Path) -> Result<Option<String>, String> {
    match harness {
        "pi" => pi::session_files::validate_session_file(path).map(|_| None),
        "codex-cli" | "cursor-cli" | "opencode2" => external_session_locator(harness, path)
            .map(Some)
            .ok_or_else(|| format!("session locator does not belong to {harness}")),
        _ => Err(format!("unsupported session harness: {harness}")),
    }
}

pub(crate) fn validate_session_target(target: &SessionTarget) -> Result<(), String> {
    if validated_locator(&target.harness, &target.path)?.is_some_and(|id| id != target.id) {
        return Err(format!(
            "session locator does not match its {} identity",
            target.harness
        ));
    }
    Ok(())
}

pub(crate) fn supports_session_move(harness: &str) -> bool {
    super::known_backend_descriptors()
        .into_iter()
        .any(|backend| {
            backend.id.as_str() == harness
                && backend.capabilities.sessions.move_project
                    == crate::agents::contract::CapabilitySupport::Available
        })
}

pub(crate) fn validate_session_move(family: &[SessionSummary]) -> Result<(), String> {
    let root = family.first().ok_or("session family is empty")?;
    if !supports_session_move(&root.harness) {
        return Err(format!(
            "Moving {} sessions between projects is not supported",
            root.harness
        ));
    }
    for session in family {
        validate_session_target(&session.target())?;
        if session.harness != root.harness {
            return Err("Moving a session family across harnesses is not supported".into());
        }
    }
    Ok(())
}

pub(crate) fn move_session_family(
    family: &[SessionSummary],
    target_project: &Path,
) -> Result<SessionTransfer, String> {
    validate_session_move(family)?;
    let root = &family[0];
    match root.harness.as_str() {
        "pi" => {
            let members = family
                .iter()
                .map(|session| TransferMember {
                    path: session.path.clone(),
                    id: session.id.clone(),
                    parent_id: session.parent_session.clone(),
                })
                .collect::<Vec<_>>();
            pi::transfer::move_to_project(&members, &root.id, target_project, &root.path)
        }
        "opencode2" => opencode::move_family(family, target_project),
        "codex-cli" => codex::move_family(family, target_project),
        _ => Err(format!(
            "unsupported session move harness: {}",
            root.harness
        )),
    }
}

pub(crate) fn delete_session_family(
    targets: &[SessionTarget],
) -> Result<Vec<(PathBuf, String)>, String> {
    if targets.is_empty() {
        return Err("session family is empty".into());
    }
    for target in targets {
        validate_session_target(target)?;
    }
    let mut pi_paths = Vec::new();
    for target in targets.iter().rev() {
        match target.harness.as_str() {
            "pi" => pi_paths.push(target.path.clone()),
            "codex-cli" => codex::delete_session(&target.id)?,
            "cursor-cli" => cursor::delete_session(&target.id)?,
            "opencode2" => opencode::delete_session(&target.id)?,
            _ => unreachable!("all session targets were validated"),
        }
    }
    if pi_paths.is_empty() {
        Ok(Vec::new())
    } else {
        pi::deletion::delete_family(&pi_paths)
    }
}

pub(crate) fn load_session_history(harness: &str, path: &Path) -> Result<LoadedHistory, String> {
    validate_session_locator(harness, path)?;
    let history = match harness {
        "pi" => return pi::session_files::load_history(path),
        "codex-cli" => codex::load_history(path)?,
        "cursor-cli" => cursor::load_history(path)?,
        "opencode2" => opencode::load_history(path)?,
        _ => return Err(format!("unsupported session harness: {harness}")),
    };
    Ok(LoadedHistory {
        messages: history.messages,
        model: history.model,
        thinking_level: history.thinking_level,
        pending_question: None,
    })
}

pub(crate) fn discover_sessions_for(
    harness: &str,
    locator_root: Option<&Path>,
    query: &str,
) -> Result<Vec<SessionSummary>, String> {
    if harness == "pi" {
        return Ok(pi::session_files::discover(query)?.sessions);
    }
    super::discover_external_sessions_for(harness, locator_root, query)
        .map(|sessions| sessions.into_iter().map(import_session).collect())
}

pub(crate) fn discover_sessions(
    locator_root: Option<&Path>,
    query: &str,
) -> crate::sessions::SessionDiscovery {
    let pi = pi::session_files::discover(query);
    let (external, exhaustive) = super::discover_external_sessions(locator_root, query);
    merge_discovery(
        pi,
        external.into_iter().map(import_session).collect(),
        exhaustive,
    )
}

fn merge_discovery(
    pi: Result<crate::sessions::SessionDiscovery, String>,
    external: Vec<SessionSummary>,
    exhaustive: bool,
) -> crate::sessions::SessionDiscovery {
    let mut discovery = pi.unwrap_or_else(|error| {
        zlog::warn!("Pi session discovery failed: {error}");
        crate::sessions::SessionDiscovery {
            sessions: Vec::new(),
            activities: Default::default(),
            exhaustive: false,
        }
    });
    discovery.sessions.extend(external);
    discovery.exhaustive &= exhaustive;
    discovery
        .sessions
        .sort_by_key(|session| std::cmp::Reverse(session.modified));
    discovery
}

fn import_session(session: crate::agents::DiscoveredSession) -> SessionSummary {
    SessionSummary::import(crate::sessions::SessionImport {
        id: session.id,
        harness: session.harness,
        path: session.path,
        project: session.project,
        title: session.title,
        first_user_message: session.first_user_message,
        timestamp: session.timestamp,
        parent_session: session.parent_session,
        modified: session.modified,
        message_count: session.message_count,
        usage: crate::sessions::UsageSummary {
            input: session.usage.input,
            output: session.usage.output,
            cache_read: session.usage.cache_read,
            cache_write: session.usage.cache_write,
            total: session.usage.total,
            cost_micros: session.usage.cost_micros,
        },
        archived: session.archived,
        is_running: session.is_running,
        search: session.search,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PI_HEADER: &str = "{\"type\":\"session\",\"id\":\"root\",\"cwd\":\"/project\"}\n";

    fn summary(harness: &str, path: PathBuf, id: &str) -> SessionSummary {
        SessionSummary::from_cached_for_harness(
            id.into(),
            harness.into(),
            path,
            PathBuf::from("/project"),
            String::new(),
            String::new(),
            String::new(),
            None,
            std::time::SystemTime::now(),
            0,
            crate::sessions::UsageSummary::default(),
            true,
            false,
            String::new(),
        )
    }

    #[test]
    fn pi_discovery_failure_keeps_other_harnesses_and_prevents_pruning() {
        let other = summary(
            "codex-cli",
            PathBuf::from("/locators/codex-cli/thread"),
            "thread",
        );
        let discovery =
            merge_discovery(Err("unreadable Pi root".into()), vec![other.clone()], true);
        assert_eq!(discovery.sessions, vec![other]);
        assert!(!discovery.exhaustive);
    }

    #[test]
    fn unsupported_moves_never_touch_files_or_create_destination() {
        let temp = tempfile::tempdir().expect("test fixture");
        let source = temp.path().join("session.jsonl");
        let contents = r#"{"type":"session","id":"root","cwd":"/project"}"#;
        std::fs::write(&source, contents).expect("test fixture");
        let destination = temp.path().join("destination");
        for harness in ["cursor-cli", "unknown", ""] {
            let session = summary(harness, source.clone(), "root");
            assert!(move_session_family(&[session], &destination).is_err());
            assert_eq!(
                std::fs::read_to_string(&source).expect("test fixture"),
                contents
            );
            assert!(!destination.exists());
        }
    }

    #[test]
    fn mixed_harness_move_fails_before_pi_mutation() {
        let temp = tempfile::tempdir().expect("test fixture");
        let root = temp.path().join("root.jsonl");
        std::fs::write(&root, PI_HEADER).expect("test fixture");
        let family = [
            summary("pi", root.clone(), "root"),
            summary("codex-cli", temp.path().join("codex-cli/child"), "child"),
        ];
        assert!(
            move_session_family(&family, &temp.path().join("destination"))
                .expect_err("mixed family")
                .contains("across harnesses")
        );
        assert_eq!(
            std::fs::read_to_string(root).expect("test fixture"),
            PI_HEADER
        );
        assert!(!temp.path().join("destination").exists());
    }

    #[test]
    fn deletion_validates_all_members_before_touching_any_file() {
        let temp = tempfile::tempdir().expect("test fixture");
        let root = temp.path().join("root.jsonl");
        std::fs::write(&root, PI_HEADER).expect("test fixture");
        for harness in ["codex-cli", "cursor-cli", "opencode2", "unknown", ""] {
            let targets = [
                summary("pi", root.clone(), "root").target(),
                summary(harness, temp.path().join("unrecognized"), "child").target(),
            ];
            assert!(delete_session_family(&targets).is_err());
            assert_eq!(
                std::fs::read_to_string(&root).expect("test fixture"),
                PI_HEADER
            );
        }
    }

    #[test]
    fn pi_identity_does_not_depend_on_its_parent_directory_name() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let directory = temp.path().join("codex-cli");
        std::fs::create_dir(&directory).expect("session directory");
        let path = directory.join("root.jsonl");
        std::fs::write(&path, "{\"type\":\"session\",\"id\":\"root\"}\n").expect("Pi header");
        assert!(load_session_history("pi", &path).is_ok());
        assert!(delete_session_family(&[summary("pi", path.clone(), "root").target()]).is_ok());
        assert!(!path.exists());
    }

    #[test]
    fn history_requires_explicit_matching_harness_without_pi_fallback() {
        let temp = tempfile::tempdir().expect("test fixture");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            "{\"type\":\"session\",\"id\":\"root\",\"cwd\":\"/project\"}\n",
        )
        .expect("test fixture");
        assert!(load_session_history("pi", &path).is_ok());
        for harness in ["codex-cli", "cursor-cli", "opencode2", "unknown", ""] {
            assert!(load_session_history(harness, &path).is_err(), "{harness}");
        }
    }

    #[test]
    fn external_identity_must_match_both_harness_and_id() {
        let path = PathBuf::from("/locators/codex-cli/thread");
        assert!(
            validate_session_target(&summary("codex-cli", path.clone(), "thread").target()).is_ok()
        );
        assert!(
            validate_session_target(&summary("codex-cli", path.clone(), "other").target()).is_err()
        );
        assert!(validate_session_target(&summary("cursor-cli", path, "thread").target()).is_err());
    }
}
