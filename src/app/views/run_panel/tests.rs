use super::{
    agents::{AgentSection, agent_section, lifecycle_label},
    ordered_worker_rows,
    resize::clamped_run_panel_width,
    run_panel_agent_rows, worker_navigation_rows,
};
use crate::agents::Backend;
use crate::{
    agent_activity::{AgentActivity, AgentLifecycle, AgentOutcome},
    app::ui::theme::theme,
};
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

#[test]
fn run_panel_resize_stays_within_design_bounds() {
    assert_eq!(clamped_run_panel_width(100.0), theme().layout.run_panel_min);
    assert_eq!(clamped_run_panel_width(332.0), theme().size(332.0));
    assert_eq!(clamped_run_panel_width(500.0), theme().layout.run_panel_max);
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
    assert_eq!(
        fallback.lifecycle,
        AgentLifecycle::Completed(AgentOutcome::Complete)
    );
    assert_eq!(fallback.ended, Some(session.modified));
    assert!(fallback.limited);
    assert_eq!(
        agent_section(fallback.lifecycle, fallback.limited, session.is_running),
        AgentSection::Completed
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
    child.harness = Backend::Codex;
    let sessions = crate::sessions::SessionCatalog::from(vec![root, child.clone()]);

    let rows = run_panel_agent_rows(
        &sessions,
        &std::collections::HashMap::new(),
        Some(PathBuf::from("/one/root.jsonl").as_path()),
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].2.path, child.path);
    assert_eq!(rows[0].0.session_path, child.path);
    assert_eq!(rows[0].3, AgentSection::Completed);
}

#[test]
fn worker_navigation_keeps_creation_order_when_statuses_differ() {
    let root = crate::sessions::SessionSummary::from_cached(
        "root".into(),
        "/project/root".into(),
        "/project".into(),
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
    let children = (0..7)
        .map(|index| {
            let mut child = crate::sessions::SessionSummary::from_cached(
                format!("child-{index}"),
                format!("/project/child-{index}").into(),
                "/project".into(),
                "worker".into(),
                String::new(),
                format!("2026-09-23T00:00:0{index}Z"),
                Some("root".into()),
                SystemTime::now(),
                0,
                crate::sessions::UsageSummary::default(),
                false,
                index == 0,
                String::new(),
            );
            child.harness = Backend::Codex;
            child
        })
        .collect::<Vec<_>>();
    let sessions = std::iter::once(root).chain(children).collect::<Vec<_>>();
    let mut limited = AgentActivity::limited_fallback(&sessions[2]);
    limited.lifecycle = AgentLifecycle::Unknown;
    let activities = std::collections::HashMap::from([(
        crate::agent_activity::agent_activity_key(&sessions[2].path),
        limited,
    )]);
    let sessions = crate::sessions::SessionCatalog::from(sessions);
    let rows = worker_navigation_rows(&sessions, &activities, Some(Path::new("/project/child-3")));
    let ids = rows
        .iter()
        .map(|session| session.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "root", "child-6", "child-5", "child-4", "child-3", "child-2", "child-1", "child-0"
        ]
    );
    let ordered = ordered_worker_rows(&sessions, &activities, Some(Path::new("/project/root")));
    assert_eq!(ordered[5].3, AgentSection::Limited);
    assert_eq!(ordered[6].3, AgentSection::Active);
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
            Backend::Codex,
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
    let sessions = vec![root_one, child_one, root_two, child_two.clone()];

    let sessions = crate::sessions::SessionCatalog::from(sessions);
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
        backend: Backend::Codex,
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
