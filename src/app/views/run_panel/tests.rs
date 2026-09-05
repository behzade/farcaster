use super::{
    agents::{AgentSection, agent_section, lifecycle_label, role_icon},
    resize::clamped_run_panel_width,
};
use crate::{
    agent_activity::{AgentLifecycle, AgentOutcome},
    app::ui::{assets::AppIcon, theme::THEME},
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
        agent_section(AgentLifecycle::Unknown, true, false),
        AgentSection::Limited
    );
    assert_eq!(
        agent_section(
            AgentLifecycle::Completed(AgentOutcome::Complete),
            true,
            false
        ),
        AgentSection::Limited
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
    assert_eq!(execution_label(None, None, None), "— · — · default");
}
