use super::{
    agents::{AgentSection, agent_section, lifecycle_label, role_icon},
    resize::clamped_run_panel_width,
    run_panel_agent_rows,
};
use crate::{
    agent_activity::{AgentActivity, AgentLifecycle, AgentOutcome},
    app::ui::{assets::AppIcon, theme::THEME},
};
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

#[test]
fn run_panel_resize_stays_within_design_bounds() {
    assert_eq!(clamped_run_panel_width(100.0), THEME.layout.run_panel_min);
    assert_eq!(clamped_run_panel_width(332.0), gpui::px(332.0));
    assert_eq!(clamped_run_panel_width(500.0), THEME.layout.run_panel_max);
}

#[test]
fn lifecycle_labels_are_truthful() {
    assert_eq!(
        lifecycle_label(AgentLifecycle::Completed(AgentOutcome::Failed)),
        "Failed"
    );
    assert_eq!(lifecycle_label(AgentLifecycle::Unknown), "Unknown");
}

#[test]
fn agent_roles_use_semantic_icons() {
    assert_eq!(role_icon("reviewer"), AppIcon::Eye);
    assert_eq!(role_icon("scout"), AppIcon::Binoculars);
    assert_eq!(role_icon("researcher"), AppIcon::Microscope);
    assert_eq!(role_icon("worker"), AppIcon::Hammer);
    assert_eq!(role_icon("other"), AppIcon::UserFocus);
}

#[test]
fn active_agents_are_never_hidden_by_limited_history() {
    assert_eq!(
        agent_section(AgentLifecycle::Working, true, true),
        AgentSection::Active
    );
    assert_eq!(
        agent_section(AgentLifecycle::NeedsInput, true, true),
        AgentSection::Active
    );
    assert_eq!(
        agent_section(AgentLifecycle::NeedsInput, true, false),
        AgentSection::Active
    );
    assert_eq!(
        agent_section(AgentLifecycle::Unknown, true, false),
        AgentSection::Limited
    );
    assert_eq!(
        agent_section(
            AgentLifecycle::Completed(AgentOutcome::Complete),
            true,
            false
        ),
        AgentSection::Completed
    );
    assert_eq!(
        agent_section(
            AgentLifecycle::Completed(AgentOutcome::Complete),
            false,
            false
        ),
        AgentSection::Completed
    );
}

#[test]
fn restored_child_without_activity_gets_a_visible_selectable_fallback() {
    let session = crate::sessions::SessionSummary::from_cached(
        "child".into(),
        PathBuf::from("/sessions/child.jsonl"),
        PathBuf::from("/project"),
        "reviewer".into(),
        "Review the patch".into(),
        String::new(),
        Some("parent".into()),
        SystemTime::now(),
        0,
        crate::sessions::UsageSummary::default(),
        false,
        false,
        String::new(),
    );

    let fallback = AgentActivity::limited_fallback(&session);

    assert_eq!(fallback.session_id, session.id);
    assert_eq!(fallback.session_path, session.path);
    assert_eq!(fallback.activity, "Review the patch");
    assert_eq!(fallback.lifecycle, AgentLifecycle::Unknown);
    assert!(fallback.limited);
    assert_eq!(
        agent_section(fallback.lifecycle, fallback.limited, session.is_running),
        AgentSection::Limited
    );
}

#[test]
fn running_child_without_activity_is_not_hidden_as_limited() {
    let session = crate::sessions::SessionSummary::from_cached(
        "child".into(),
        PathBuf::from("/sessions/child.jsonl"),
        PathBuf::from("/project"),
        "worker".into(),
        "Implement".into(),
        String::new(),
        Some("parent".into()),
        SystemTime::now(),
        0,
        crate::sessions::UsageSummary::default(),
        false,
        true,
        String::new(),
    );

    let fallback = AgentActivity::limited_fallback(&session);

    assert_eq!(fallback.lifecycle, AgentLifecycle::Working);
    assert_eq!(
        agent_section(fallback.lifecycle, fallback.limited, session.is_running),
        AgentSection::Active
    );
}

#[test]
fn production_rows_include_restored_children_from_an_empty_activity_map() {
    let root = crate::sessions::SessionSummary::from_cached(
        "shared-id".into(),
        PathBuf::from("/one/root.jsonl"),
        PathBuf::from("/one"),
        "root".into(),
        String::new(),
        String::new(),
        None,
        SystemTime::now(),
        0,
        crate::sessions::UsageSummary::default(),
        false,
        false,
        String::new(),
    );
    let mut child = crate::sessions::SessionSummary::from_cached(
        "shared-id".into(),
        PathBuf::from("/one/child.jsonl"),
        PathBuf::from("/one"),
        "child".into(),
        "Restored task".into(),
        String::new(),
        Some("shared-id".into()),
        SystemTime::now(),
        0,
        crate::sessions::UsageSummary::default(),
        false,
        false,
        String::new(),
    );
    child.harness = "codex-cli".into();
    let sessions = [root, child.clone()];

    let rows = run_panel_agent_rows(
        &sessions,
        &std::collections::HashMap::new(),
        Some(PathBuf::from("/one/root.jsonl").as_path()),
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].2.path, child.path);
    assert_eq!(rows[0].0.session_path, child.path);
    assert_eq!(rows[0].3, AgentSection::Limited);
}

#[test]
fn production_rows_resolve_same_native_id_by_scoped_session_path() {
    let session = |root: &str, path: &str, parent: Option<&str>| {
        crate::sessions::SessionSummary::from_cached_for_harness(
            if parent.is_some() {
                "shared-child"
            } else {
                root
            }
            .into(),
            "codex-cli".into(),
            PathBuf::from(path),
            PathBuf::from(format!("/{root}")),
            root.into(),
            String::new(),
            String::new(),
            parent.map(str::to_owned),
            SystemTime::now(),
            0,
            crate::sessions::UsageSummary::default(),
            false,
            false,
            String::new(),
        )
    };
    let root_one = session("one", "/one/root", None);
    let child_one = session("one", "/one/child", Some("one"));
    let root_two = session("two", "/two/root", None);
    let child_two = session("two", "/two/child", Some("two"));
    let mut first = AgentActivity::limited_fallback(&child_one);
    first.activity = "first project".into();
    let mut second = AgentActivity::limited_fallback(&child_two);
    second.activity = "second project".into();
    let activities = std::collections::HashMap::from([
        (
            crate::agent_activity::agent_activity_key(&child_one.path),
            first,
        ),
        (
            crate::agent_activity::agent_activity_key(&child_two.path),
            second,
        ),
    ]);
    let sessions = [root_one, child_one, root_two, child_two.clone()];

    let rows = run_panel_agent_rows(&sessions, &activities, Some(Path::new("/two/root")));

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].2.path, child_two.path);
    assert_eq!(rows[0].0.activity, "second project");
}

#[test]
fn worker_subtitle_uses_live_profile_then_catalog_identity() {
    use super::agents::execution_label;

    let cached_model = ("cached-provider".into(), "cached-model".into());
    let profile = crate::agents::CallerProfile {
        backend: "codex-cli".into(),
        provider: Some("openai".into()),
        model: Some("gpt-5.6-luna".into()),
        effort: Some("high".into()),
    };
    assert_eq!(
        execution_label(Some(&profile), Some(&cached_model), Some("low")),
        "openai · gpt-5.6-luna · high"
    );
    let profile = crate::agents::CallerProfile {
        effort: None,
        ..profile
    };
    assert_eq!(
        execution_label(Some(&profile), Some(&cached_model), Some("low")),
        "openai · gpt-5.6-luna · default"
    );
    assert_eq!(
        execution_label(None, Some(&cached_model), Some("low")),
        "cached-provider · cached-model · low"
    );
    assert_eq!(execution_label(None, None, None), "Model unavailable");
    let profile = crate::agents::CallerProfile {
        model: None,
        ..profile
    };
    assert_eq!(
        execution_label(Some(&profile), Some(&cached_model), Some("low")),
        "cached-provider · cached-model · low"
    );
}
