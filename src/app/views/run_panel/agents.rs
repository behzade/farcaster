use gpui::{
    AnyElement, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, px,
};

use super::super::super::{FarcasterApp, RunPanelView};
use crate::{
    agent_activity::{AgentActivity, AgentLifecycle, AgentOutcome},
    app::ui::assets::AppIcon,
    app::ui::primitives::{AppIconSize, activates_button, app_icon, disclosure_button},
    app::ui::theme::THEME,
};

pub(super) const MAX_VISIBLE_COMPLETED_AGENTS: usize = 5;

impl FarcasterApp {
    pub(super) fn agent_card(
        &self,
        activity: &AgentActivity,
        depth: usize,
        limited: bool,
        entity: WeakEntity<Self>,
    ) -> Option<AnyElement> {
        let focus = self.agent_row_focus.get(&activity.session_id)?.clone();
        let session = self
            .all_sessions
            .iter()
            .find(|session| session.id == activity.session_id)?;
        let path = session.path.clone();
        let project = session.project.clone();
        let key_path = path.clone();
        let key_project = project.clone();
        let key_entity = entity.clone();
        let displayed_lifecycle = if limited {
            AgentLifecycle::Unknown
        } else {
            activity.lifecycle
        };
        let state = lifecycle_label(displayed_lifecycle);
        let activity_text = activity.activity.clone();
        let role = activity.role.clone();
        let marker = role_icon(&role);
        let registry = crate::agents::CallerRegistry::shared();
        let profile = registry
            .session_profile(&session.project, &session.harness, &session.id)
            .or_else(|| {
                registry.session_profile(
                    &session.project,
                    &session.harness,
                    &session.path.to_string_lossy(),
                )
            });
        let execution = execution_label(
            profile.as_ref(),
            session.model.as_ref(),
            session.thinking_level.as_deref(),
        );
        Some(
            div()
                .id(format!("agent-card-{}", activity.session_id))
                .track_focus(&focus)
                .role(Role::Button)
                .aria_label(format!("Show {role} transcript: {state}"))
                .tab_index(0)
                .ml(px(depth.saturating_sub(1) as f32 * 8.0))
                .px(px(2.0))
                .py(px(3.0))
                .flex()
                .items_stretch()
                .gap(THEME.space.sm)
                .hover(|card| card.bg(THEME.colors.surface))
                .focus(|card| card.bg(THEME.colors.surface))
                .cursor_pointer()
                .on_click(move |_, window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.select_session(path.clone(), project.clone(), window, cx);
                    });
                })
                .on_key_down(move |event, window, cx| {
                    if activates_button(event) {
                        cx.stop_propagation();
                        let _ = key_entity.update(cx, |this, cx| {
                            this.select_session(key_path.clone(), key_project.clone(), window, cx)
                        });
                    }
                })
                .child(
                    div()
                        .w(THEME.controls.agent_marker)
                        .flex_none()
                        .text_color(THEME.colors.muted)
                        .child(app_icon(marker, AppIconSize::Inline)),
                )
                .child(
                    div()
                        .w_0()
                        .min_w_0()
                        .flex_1()
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(THEME.type_scale.body_small)
                                .text_color(THEME.colors.text)
                                .child(if activity_text.is_empty() {
                                    role
                                } else {
                                    activity_text
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(THEME.space.xs)
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.muted)
                                .child(app_icon(
                                    AppIcon::for_harness(&session.harness),
                                    AppIconSize::Inline,
                                ))
                                .child(
                                    div()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(execution),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

pub(super) fn execution_label(
    profile: Option<&crate::agents::CallerProfile>,
    model: Option<&(String, String)>,
    effort: Option<&str>,
) -> String {
    let effort_fallback = if profile.is_some() || model.is_some() {
        "default"
    } else {
        "—"
    };
    let (provider, model, effort) = match profile {
        Some(profile) => (
            profile.provider.as_deref(),
            profile.model.as_deref(),
            profile.effort.as_deref(),
        ),
        None => (
            model.map(|(provider, _)| provider.as_str()),
            model.map(|(_, model)| model.as_str()),
            effort,
        ),
    };
    format!(
        "{} · {} · {}",
        provider.filter(|value| !value.is_empty()).unwrap_or("—"),
        model.filter(|value| !value.is_empty()).unwrap_or("—"),
        effort
            .filter(|value| !value.is_empty())
            .unwrap_or(effort_fallback),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AgentSection {
    Active,
    Completed,
    Limited,
    Hidden,
}

pub(super) fn agent_section(
    lifecycle: AgentLifecycle,
    limited: bool,
    is_running: bool,
) -> AgentSection {
    if is_running
        && matches!(
            lifecycle,
            AgentLifecycle::NeedsInput | AgentLifecycle::Working
        )
    {
        return AgentSection::Active;
    }
    if limited || matches!(lifecycle, AgentLifecycle::Unknown) {
        return AgentSection::Limited;
    }
    match lifecycle {
        AgentLifecycle::NeedsInput | AgentLifecycle::Working if is_running => AgentSection::Active,
        AgentLifecycle::Completed(_) => AgentSection::Completed,
        AgentLifecycle::NeedsInput | AgentLifecycle::Working | AgentLifecycle::Unknown => {
            AgentSection::Hidden
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum RunDisclosure {
    Completed,
    Limited,
}

pub(super) fn disclosure_control(
    id: &'static str,
    label: &'static str,
    expanded: bool,
    disclosure: RunDisclosure,
    entity: WeakEntity<RunPanelView>,
) -> AnyElement {
    disclosure_button(id, expanded, label, move |_, cx| {
        let _ = entity.update(cx, |view, cx| {
            match disclosure {
                RunDisclosure::Completed => view.toggle_completed_agents(),
                RunDisclosure::Limited => view.toggle_limited_agents(),
            }
            cx.notify();
        });
    })
}

pub(super) fn role_icon(role: &str) -> AppIcon {
    match role.to_ascii_lowercase().as_str() {
        "reviewer" => AppIcon::Eye,
        "scout" => AppIcon::Binoculars,
        "researcher" => AppIcon::Microscope,
        "worker" => AppIcon::Hammer,
        _ => AppIcon::UserFocus,
    }
}

pub(super) fn lifecycle_label(lifecycle: AgentLifecycle) -> &'static str {
    match lifecycle {
        AgentLifecycle::NeedsInput => "Needs input",
        AgentLifecycle::Working => "Working",
        AgentLifecycle::Unknown => "Unknown",
        AgentLifecycle::Completed(AgentOutcome::Complete) => "Complete",
        AgentLifecycle::Completed(AgentOutcome::Failed) => "Failed",
        AgentLifecycle::Completed(AgentOutcome::Incomplete) => "Incomplete",
    }
}
