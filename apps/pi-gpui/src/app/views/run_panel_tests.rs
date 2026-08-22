use super::*;

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
