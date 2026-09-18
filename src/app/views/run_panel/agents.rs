use gpui::{
    AnyElement, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    ScrollAnchor, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};

use super::super::super::{FarcasterApp, RunPanelView};
use super::super::session_rail::{session_hover_details, session_tooltip_content, status_visual};
use super::WorkerProfileNames;
use crate::{
    agent_activity::{AgentActivity, AgentLifecycle, AgentOutcome},
    app::ui::assets::AppIcon,
    app::ui::primitives::{
        AppIconSize, AppTooltip as _, activates_button, app_icon, disclosure_button,
    },
    app::ui::theme::theme,
};

pub(super) fn conversation_row(id: impl Into<ElementId>, selected: bool) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_selected(selected)
        .tab_index(0)
        .relative()
        .flex()
        .items_center()
        .min_w_0()
        .px(theme().space.sm)
        .py(theme().size(4.0))
        .rounded(theme().radius)
        .bg(if selected {
            theme().colors.highlight
        } else {
            theme().colors.inspector
        })
        .hover(move |row| {
            row.bg(if selected {
                theme().colors.highlight
            } else {
                theme().colors.surface
            })
        })
        .when(selected, |row| {
            row.child(
                div()
                    .absolute()
                    .left_0()
                    .top(theme().space.xs)
                    .bottom(theme().space.xs)
                    .w(theme().size(2.0))
                    .bg(theme().colors.accent),
            )
        })
        .focus(|row| {
            row.border(theme().border)
                .border_color(theme().colors.accent)
        })
        .cursor_pointer()
}

impl FarcasterApp {
    pub(super) fn agent_card(
        &self,
        activity: &AgentActivity,
        session: &crate::sessions::SessionSummary,
        depth: usize,
        selected: bool,
        anchor: &ScrollAnchor,
        saved_profiles: &WorkerProfileNames,
        entity: WeakEntity<Self>,
    ) -> Option<AnyElement> {
        let activity_key = crate::agent_activity::agent_activity_key(&session.path);
        let focus = self.activity.row_focus.get(&activity_key)?.clone();
        let path = session.path.clone();
        let project = session.project.clone();
        let key_path = path.clone();
        let key_project = project.clone();
        let key_entity = entity.clone();
        let state = lifecycle_label(activity.lifecycle);
        let role = activity.role.clone();
        let registry = crate::agents::CallerRegistry::shared();
        let caller = registry
            .session_caller(&session.project, session.harness, &session.id)
            .or_else(|| {
                registry.session_caller(
                    &session.project,
                    session.harness,
                    &session.path.to_string_lossy(),
                )
            });
        let mut hover_details = session_hover_details(session, state, "", 0);
        let profile_name = registry
            .session_worker_profile(&session.project, session.harness, &session.id)
            .or_else(|| {
                registry.session_worker_profile(
                    &session.project,
                    session.harness,
                    &session.path.to_string_lossy(),
                )
            })
            .or_else(|| {
                saved_profiles
                    .get(&(session.project.clone(), session.harness, session.id.clone()))
                    .or_else(|| {
                        saved_profiles.get(&(
                            session.project.clone(),
                            session.harness,
                            session.path.to_string_lossy().into_owned(),
                        ))
                    })
                    .cloned()
            });
        if let Some(profile) = &profile_name {
            hover_details
                .rows
                .insert(0, ("Profile".into(), profile.clone()));
        }
        hover_details.rows.insert(
            0,
            (
                "Name".into(),
                caller
                    .as_ref()
                    .map(|(name, _)| name.clone())
                    .unwrap_or_else(|| role.clone()),
            ),
        );
        let execution = execution_label(
            caller.as_ref().map(|(_, profile)| profile),
            session.model.as_ref(),
            session.thinking_level.as_deref(),
        );
        let identity = profile_name
            .or_else(|| caller.as_ref().map(|(name, _)| name.clone()))
            .unwrap_or_else(|| role.clone());
        let card = conversation_row(format!("agent-card-{activity_key}"), selected)
            .anchor_scroll(selected.then(|| anchor.clone()))
            .debug_selector(move || format!("agent-card-{activity_key}"))
            .track_focus(&focus)
            .aria_label(format!("Show {identity} transcript: {state}"))
            .on_mouse_down(
                gpui::MouseButton::Left,
                crate::app::ui::primitives::preserve_pointer_focus,
            )
            .ml(px(depth.saturating_sub(1) as f32 * 8.0))
            .px(theme().size(2.0))
            .py(theme().size(3.0))
            .flex()
            .items_stretch()
            .hover(|card| card.bg(theme().colors.highlight))
            .focus(|card| card.bg(theme().colors.highlight))
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                let _ = entity.update(cx, |this, cx| {
                    this.select_session_and_focus(path.clone(), project.clone(), window, cx);
                });
            })
            .on_key_down(move |event, window, cx| {
                if activates_button(event) {
                    cx.stop_propagation();
                    let _ = key_entity.update(cx, |this, cx| {
                        this.select_session_and_focus(
                            key_path.clone(),
                            key_project.clone(),
                            window,
                            cx,
                        )
                    });
                }
            })
            .child(
                div()
                    .w_0()
                    .min_w_0()
                    .flex_1()
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .gap(theme().space.xs)
                    .text_size(theme().type_scale.caption)
                    .text_color(theme().colors.muted)
                    .when_some(status_visual(state), |row, (icon, color)| {
                        row.child(
                            div()
                                .flex_none()
                                .text_color(color)
                                .child(app_icon(icon, AppIconSize::Inline)),
                        )
                    })
                    .child(app_icon(
                        AppIcon::for_harness(session.harness),
                        AppIconSize::Inline,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap(theme().space.xs)
                            .child(
                                div()
                                    .flex_none()
                                    .max_w(theme().size(90.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_color(theme().colors.text)
                                    .child(identity),
                            )
                            .child(
                                div()
                                    .w_0()
                                    .min_w_0()
                                    .flex_1()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(format!("· {execution}")),
                            ),
                    ),
            )
            .app_tooltip_element(move |_, _| session_tooltip_content(&hover_details))
            .into_any_element();
        Some(card)
    }
}

pub(super) fn execution_label(
    profile: Option<&crate::agents::CallerProfile>,
    model: Option<&(String, String)>,
    effort: Option<&str>,
) -> String {
    let profile = profile.filter(|profile| {
        profile
            .model
            .as_deref()
            .is_some_and(|model| !model.is_empty())
    });
    let model = model.filter(|(_, model)| !model.is_empty());
    if profile.is_none() && model.is_none() {
        return "Model unavailable".into();
    }
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
        model.unwrap_or("—"),
        effort
            .filter(|value| !value.is_empty())
            .unwrap_or("default"),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum AgentSection {
    Active,
    Completed,
    Limited,
    Hidden,
}

pub(in crate::app) fn agent_section(
    lifecycle: AgentLifecycle,
    limited: bool,
    is_running: bool,
) -> AgentSection {
    if (is_running || limited)
        && matches!(
            lifecycle,
            AgentLifecycle::NeedsInput | AgentLifecycle::Working
        )
    {
        return AgentSection::Active;
    }
    if matches!(lifecycle, AgentLifecycle::Completed(_)) {
        return AgentSection::Completed;
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
