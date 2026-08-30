use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, Context, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, RenderOnce, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, deferred, div, prelude::FluentBuilder as _, px,
};
use gpui_base::{Align, Positioner};
use gpui_component::{ElementExt as _, Placement};

use super::usage::{format_cost, format_tokens};
use crate::{
    projects::DraftSession,
    sessions::{SessionSummary, UsageSummary},
    theme::{MONO_FONT_FAMILY, THEME},
};

const OPEN_DELAY: Duration = Duration::from_millis(250);
const MIN_PANEL_WIDTH: f32 = 220.0;
const MAX_PANEL_WIDTH: f32 = 360.0;
const PREVIEW_CHARS: usize = 160;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SessionHoverDetails {
    pub title: String,
    pub rows: Vec<(String, String)>,
    pub preview: Option<String>,
}

pub(super) fn session_hover_details(
    session: &SessionSummary,
    status: &str,
    age: &str,
    subagents: usize,
) -> SessionHoverDetails {
    let mut rows = Vec::new();
    push_row(&mut rows, "Project", project_value(&session.project));
    if let Some((provider, model)) = &session.model {
        push_row(&mut rows, "Model", format!("{provider} / {model}"));
    }
    if let Some(level) = &session.thinking_level {
        push_row(&mut rows, "Effort", effort_label(level));
    }
    if !status.is_empty() {
        push_row(&mut rows, "State", status.to_owned());
    }
    if !age.is_empty() {
        push_row(&mut rows, "Updated", age.to_owned());
    }
    if session.message_count > 0 {
        push_row(&mut rows, "Messages", session.message_count.to_string());
    }
    if let Some(usage) = usage_value(&session.usage) {
        push_row(&mut rows, "Usage", usage);
    }
    if subagents > 0 {
        let plural = if subagents == 1 { "" } else { "s" };
        push_row(
            &mut rows,
            "Subagents",
            format!("{subagents} subagent{plural}"),
        );
    }
    SessionHoverDetails {
        title: session.title.clone(),
        rows,
        preview: preview_text(&session.first_user_message),
    }
}

pub(super) fn draft_hover_details(draft: &DraftSession, status: &str) -> SessionHoverDetails {
    let mut rows = vec![
        ("Project".into(), project_value(&draft.project)),
        ("State".into(), status.to_owned()),
    ];
    if draft.submitted {
        rows.push(("Draft".into(), "submitted".into()));
    }
    SessionHoverDetails {
        title: draft.title.clone().unwrap_or_else(|| "New session".into()),
        rows,
        preview: None,
    }
}

#[cfg(test)]
pub(super) fn session_tooltip_lines(session: &SessionSummary, subagents: usize) -> Vec<String> {
    flatten_details(&session_hover_details(session, "", "", subagents))
}

#[cfg(test)]
pub(super) fn flatten_details(details: &SessionHoverDetails) -> Vec<String> {
    let mut lines = vec![details.title.clone()];
    lines.extend(
        details
            .rows
            .iter()
            .map(|(label, value)| format!("{label}: {value}")),
    );
    if let Some(preview) = &details.preview {
        lines.push(preview.clone());
    }
    lines
}

pub(super) fn session_hover_panel(
    id: impl Into<SharedString>,
    details: SessionHoverDetails,
    trigger: AnyElement,
) -> AnyElement {
    SessionHoverPanel {
        id: id.into(),
        details,
        trigger,
    }
    .into_any_element()
}

#[derive(IntoElement)]
struct SessionHoverPanel {
    id: SharedString,
    details: SessionHoverDetails,
    trigger: AnyElement,
}

#[derive(Default)]
struct HoverState {
    open: bool,
    bounds: Bounds<Pixels>,
    open_task: Option<gpui::Task<()>>,
}

impl HoverState {
    fn set_bounds(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        if self.bounds != bounds {
            self.bounds = bounds;
            if self.open {
                cx.notify();
            }
        }
    }

    fn set_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.open_task = None;
        if hovered {
            if self.open {
                return;
            }
            self.open_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(OPEN_DELAY).await;
                let _ = this.update(cx, |this, cx| {
                    this.open = true;
                    cx.notify();
                });
            }));
            return;
        }
        if self.open {
            self.open = false;
            cx.notify();
        }
    }
}

impl RenderOnce for SessionHoverPanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(ElementId::Name(self.id.clone()), cx, |_, _| {
            HoverState::default()
        });
        let bounds = state.read(cx).bounds;
        let open = state.read(cx).open && bounds.size.width > px(0.0);
        let hover_state = state.clone();
        let width = panel_width(window.viewport_size().width);
        div()
            .id(ElementId::Name(format!("{}-host", self.id).into()))
            .w_full()
            .on_prepaint(move |bounds, _, cx| {
                hover_state.update(cx, |this, cx| this.set_bounds(bounds, cx));
            })
            .on_hover({
                let state = state.clone();
                move |hovered, _, cx| {
                    state.update(cx, |this, cx| this.set_hovered(*hovered, cx));
                }
            })
            .child(self.trigger)
            .when(open, |host| {
                host.child(
                    deferred(
                        Positioner::side(bounds)
                            .placement(Placement::Right)
                            .align(Align::Start)
                            .offset(THEME.space.sm)
                            .child(render_panel(&self.details, width)),
                    )
                    .with_priority(100),
                )
            })
    }
}

fn render_panel(details: &SessionHoverDetails, width: Pixels) -> impl IntoElement {
    div()
        .id("session-hover-panel")
        .w(width)
        .max_w(width)
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .px(THEME.space.md)
        .py(THEME.space.sm)
        .rounded(THEME.radius)
        .bg(THEME.colors.surface)
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .shadow_md()
        .occlude()
        .child(
            div()
                .text_size(THEME.type_scale.body_small)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(THEME.colors.text)
                .child(details.title.clone()),
        )
        .children(details.rows.iter().map(|(label, value)| {
            div()
                .flex()
                .items_start()
                .gap(THEME.space.sm)
                .child(
                    div()
                        .w(px(72.0))
                        .flex_none()
                        .text_size(THEME.type_scale.caption)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(THEME.colors.subtle)
                        .child(label.clone()),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .text_size(THEME.type_scale.caption)
                        .font_family(MONO_FONT_FAMILY)
                        .text_color(THEME.colors.text)
                        .child(value.clone()),
                )
        }))
        .when_some(details.preview.clone(), |panel, preview| {
            panel.child(
                div()
                    .mt(THEME.space.xs)
                    .pt(THEME.space.xs)
                    .border_t(THEME.border)
                    .border_color(THEME.colors.border)
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.muted)
                    .child(preview),
            )
        })
}

fn panel_width(viewport_width: Pixels) -> Pixels {
    let to_middle = f32::from(viewport_width) / 2.0 - f32::from(THEME.layout.session_rail);
    px(to_middle.clamp(MIN_PANEL_WIDTH, MAX_PANEL_WIDTH))
}

fn push_row(rows: &mut Vec<(String, String)>, label: &str, value: String) {
    if !value.is_empty() {
        rows.push((label.to_owned(), value));
    }
}

fn project_value(project: &std::path::Path) -> String {
    project
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| project.display().to_string(), str::to_owned)
}

fn preview_text(message: &str) -> Option<String> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut preview = trimmed.chars().take(PREVIEW_CHARS).collect::<String>();
    if trimmed.chars().count() > PREVIEW_CHARS {
        preview.push('…');
    }
    Some(preview)
}

fn usage_value(usage: &UsageSummary) -> Option<String> {
    if usage.input == 0 && usage.output == 0 && usage.cost_micros == 0 && usage.total == 0 {
        return None;
    }
    let mut parts = Vec::new();
    if usage.input > 0 || usage.output > 0 {
        parts.push(format!(
            "{} in · {} out",
            format_tokens(usage.input),
            format_tokens(usage.output)
        ));
    } else if usage.total > 0 {
        parts.push(format!("{} tok", format_tokens(usage.total)));
    }
    if usage.cost_micros > 0 {
        parts.push(format_cost(usage.cost_micros));
    }
    Some(parts.join(" · "))
}

fn effort_label(level: &str) -> String {
    let mut characters = level.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => level.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::SystemTime};

    use super::*;

    #[test]
    fn hover_details_include_model_effort_and_counts() {
        let mut session = SessionSummary::from_cached(
            "one".into(),
            PathBuf::from("/one.jsonl"),
            PathBuf::from("/project"),
            "Review the rail".into(),
            "Please inspect the sidebar hover card.".into(),
            String::new(),
            None,
            SystemTime::UNIX_EPOCH,
            4,
            UsageSummary {
                input: 1200,
                output: 80,
                cost_micros: 18_000,
                ..UsageSummary::default()
            },
            false,
            false,
            String::new(),
        );
        session.model = Some(("anthropic".into(), "claude-opus-4-5".into()));
        session.thinking_level = Some("high".into());

        let details = session_hover_details(&session, "Working", "2m", 1);
        assert_eq!(details.title, "Review the rail");
        assert!(
            details
                .rows
                .contains(&("Model".into(), "anthropic / claude-opus-4-5".into()))
        );
        assert!(details.rows.contains(&("Effort".into(), "High".into())));
        assert!(details.rows.contains(&("State".into(), "Working".into())));
        assert!(details.rows.contains(&("Messages".into(), "4".into())));
        assert!(
            details
                .rows
                .contains(&("Subagents".into(), "1 subagent".into()))
        );
        assert_eq!(
            details.preview.as_deref(),
            Some("Please inspect the sidebar hover card.")
        );
    }

    #[test]
    fn panel_width_stops_at_the_window_midline() {
        assert_eq!(panel_width(px(1200.0)), px(328.0));
        assert_eq!(panel_width(px(800.0)), px(MIN_PANEL_WIDTH));
        assert_eq!(panel_width(px(2000.0)), px(MAX_PANEL_WIDTH));
    }
}
