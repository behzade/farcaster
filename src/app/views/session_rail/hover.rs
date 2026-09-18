use gpui::{
    AnyElement, FontWeight, IntoElement, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _,
};

use super::super::usage::{format_cost, format_tokens};
use crate::{
    app::ui::theme::{MONO_FONT_FAMILY, theme},
    sessions::DraftSession,
    sessions::{SessionSummary, UsageSummary},
};

const PREVIEW_CHARS: usize = 160;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::app) struct SessionHoverDetails {
    pub title: String,
    pub rows: Vec<(String, String)>,
    pub preview: Option<String>,
}

pub(in crate::app) fn session_hover_details(
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

pub(in crate::app) fn session_tooltip_content(details: &SessionHoverDetails) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(theme().space.xs)
        .child(
            div()
                .text_size(theme().type_scale.body_small)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme().colors.text)
                .child(details.title.clone()),
        )
        .children(details.rows.iter().map(|(label, value)| {
            div()
                .flex()
                .items_start()
                .gap(theme().space.sm)
                .child(
                    div()
                        .w(theme().size(72.0))
                        .flex_none()
                        .text_size(theme().type_scale.caption)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme().colors.subtle)
                        .child(label.clone()),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .text_size(theme().type_scale.caption)
                        .font_family(MONO_FONT_FAMILY)
                        .text_color(theme().colors.text)
                        .child(value.clone()),
                )
        }))
        .when_some(details.preview.clone(), |content, preview| {
            content.child(
                div()
                    .mt(theme().space.xs)
                    .pt(theme().space.xs)
                    .border_t(theme().border)
                    .border_color(theme().colors.border)
                    .text_size(theme().type_scale.caption)
                    .text_color(theme().colors.muted)
                    .child(preview),
            )
        })
        .into_any_element()
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
#[path = "hover_tests.rs"]
mod tests;
