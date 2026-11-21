use crate::app::ui::primitives::SelectionStyle as _;
use gpui::{
    AnyElement, Div, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Role, ScrollAnchor, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, div,
};

use super::super::super::FarcasterApp;
use super::super::session_rail::{session_hover_details, session_tooltip_content};
use crate::{
    agent_activity::{AgentLifecycle, AgentOutcome},
    app::ui::assets::AppIcon,
    app::ui::primitives::{AppIconSize, AppTooltip as _, activates_button, app_icon},
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
        .ml(-theme().space.xs)
        .pl(theme().space.xs)
        .pr_0()
        .py(theme().size(4.0))
        .rounded(theme().radius)
        .bg(theme().colors.composer)
        .selection_style(selected)
        .hover(move |row| {
            row.bg(if selected {
                theme().colors.highlight
            } else {
                theme().colors.surface
            })
        })
        .focus(|row| {
            row.shadow(crate::app::ui::primitives::focus_outline(
                theme().colors.focus_border,
            ))
        })
        .cursor_pointer()
}

#[cfg(test)]
thread_local! {
    pub(super) static CARD_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl FarcasterApp {
    pub(super) fn agent_card(
        &self,
        row: &std::rc::Rc<super::worker_list::WorkerRow>,
        selected: bool,
        anchor: &ScrollAnchor,
        entity: WeakEntity<Self>,
    ) -> Option<AnyElement> {
        #[cfg(test)]
        CARD_BUILDS.with(|count| count.set(count.get() + 1));
        let activity = &row.activity;
        let session = &row.session;
        let depth = row.depth;
        let focus = row.focus.clone()?;
        let click_key = row.session_key.clone();
        let keyboard_key = click_key.clone();
        let key_entity = entity.clone();
        let state = lifecycle_label(activity.lifecycle);
        let (icon, color) = agent_lifecycle_visual(activity.lifecycle);
        let role = activity.role.clone();
        let registry = crate::agents::CallerRegistry::shared();
        let key = &row.session_key;
        let presentation = registry.try_session_presentation(key);
        let caller = presentation
            .as_ref()
            .map(|(name, profile, _)| (name, profile));
        let profile_name = presentation
            .as_ref()
            .and_then(|(_, _, profile)| profile.clone())
            .or_else(|| {
                self.sessions
                    .all
                    .active_worker_profile(session.app_session_id)
                    .map(str::to_owned)
            });
        let name = caller
            .as_ref()
            .map(|(name, _)| (*name).clone())
            .unwrap_or(role);
        let tooltip_row = row.clone();
        let tooltip_name = name.clone();
        let tooltip_profile = profile_name.clone();
        let execution = execution_label(
            caller.as_ref().map(|(_, profile)| *profile),
            session.model.as_ref(),
            session.thinking_level.as_deref(),
        );
        let identity = profile_name.unwrap_or(name);
        let element_id = format!("agent-card-{:?}", row.session_key);
        let card = conversation_row(element_id, selected)
            .anchor_scroll(selected.then(|| anchor.clone()))
            .debug_selector(|| {
                if self.sessions.all.has_shared_path(&session.path) {
                    format!("agent-card-{:?}", row.session_key)
                } else {
                    format!("agent-card-{}", session.path.display())
                }
            })
            .track_focus(&focus)
            .aria_label(format!("Show {identity} transcript: {state}"))
            .on_mouse_down(
                gpui::MouseButton::Left,
                crate::app::ui::primitives::preserve_pointer_focus,
            )
            .ml(crate::app::ui::grid::tree_indent(depth.saturating_sub(1)) - theme().space.xs)
            .py(theme().space.xs)
            .flex()
            .items_stretch()
            .focus(|card| card.bg(theme().colors.highlight))
            .cursor_pointer()
            .on_click(move |event, window, cx| {
                let _ = entity.update(cx, |this, cx| {
                    this.select_catalog_session(
                        click_key.clone(),
                        matches!(event, gpui::ClickEvent::Keyboard(_)),
                        window,
                        cx,
                    );
                });
            })
            .on_key_down(move |event, window, cx| {
                if activates_button(event) {
                    cx.stop_propagation();
                    let _ = key_entity.update(cx, |this, cx| {
                        this.select_rail_session(keyboard_key.clone(), window, cx)
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
                    .items_start()
                    .gap(theme().space.xs)
                    .text_size(theme().type_scale.caption)
                    .line_height(theme().type_scale.line_body)
                    .text_color(theme().colors.muted)
                    .child(
                        crate::app::ui::grid::icon_slot()
                            .text_color(color)
                            .child(app_icon(icon, AppIconSize::Inline)),
                    )
                    .child(crate::app::ui::grid::icon_slot().child(app_icon(
                        AppIcon::for_harness(session.harness),
                        AppIconSize::Inline,
                    )))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(theme().type_scale.body)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme().colors.text)
                                    .child(identity),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_color(theme().colors.subtle)
                                    .child(execution),
                            ),
                    ),
            )
            .app_tooltip_element(move |_, _| {
                let mut details = session_hover_details(&tooltip_row.session, state, "", 0);
                if let Some(profile) = &tooltip_profile {
                    details.rows.insert(0, ("Profile".into(), profile.clone()));
                }
                details
                    .rows
                    .insert(0, ("Name".into(), tooltip_name.clone()));
                session_tooltip_content(&details)
            })
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

pub(super) fn agent_lifecycle_visual(lifecycle: AgentLifecycle) -> (AppIcon, gpui::Rgba) {
    match lifecycle {
        AgentLifecycle::NeedsInput | AgentLifecycle::Completed(AgentOutcome::Incomplete) => {
            (AppIcon::WarningCircle, theme().colors.warning)
        }
        AgentLifecycle::Working => (AppIcon::SpinnerGap, theme().colors.indicator),
        AgentLifecycle::Unknown => (AppIcon::Question, theme().colors.subtle),
        AgentLifecycle::Completed(AgentOutcome::Complete) => {
            (AppIcon::CheckCircle, theme().colors.success)
        }
        AgentLifecycle::Completed(AgentOutcome::Failed) => (AppIcon::XCircle, theme().colors.error),
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

#[cfg(test)]
#[path = "agents_tests.rs"]
mod tests;
