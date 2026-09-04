use std::time::Duration;

use super::{
    agents::{
        AgentSection, agent_section, format_duration, lifecycle_icon, lifecycle_label, role_icon,
    },
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
fn duration_and_lifecycle_labels_are_truthful() {
    assert_eq!(format_duration(Some(Duration::from_secs(65))), "1m 5s");
    assert_eq!(
        lifecycle_label(AgentLifecycle::Completed(AgentOutcome::Failed)),
        "Failed"
    );
    assert_eq!(lifecycle_label(AgentLifecycle::Unknown), "Unknown");
}

#[test]
fn every_lifecycle_has_a_compact_status_icon() {
    assert_eq!(lifecycle_icon(AgentLifecycle::Working), AppIcon::SpinnerGap);
    assert_eq!(
        lifecycle_icon(AgentLifecycle::NeedsInput),
        AppIcon::WarningCircle
    );
    assert_eq!(lifecycle_icon(AgentLifecycle::Unknown), AppIcon::Question);
    assert_eq!(
        lifecycle_icon(AgentLifecycle::Completed(AgentOutcome::Complete)),
        AppIcon::CheckCircle
    );
    assert_eq!(
        lifecycle_icon(AgentLifecycle::Completed(AgentOutcome::Failed)),
        AppIcon::XCircle
    );
    assert_eq!(
        lifecycle_icon(AgentLifecycle::Completed(AgentOutcome::Incomplete)),
        AppIcon::WarningCircle
    );
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
