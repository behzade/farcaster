//! Composer footer layout, usage metrics, and primary actions.

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _,
};

use super::{
    super::PiApp,
    models,
    run_panel_changes::{SessionChangeTotals, session_change_totals},
    usage::{ComposerUsage, composer_usage, format_cost, format_tokens, has_meaningful_usage},
};
use crate::{
    assets::AppIcon,
    primitives::{AppIconSize, ButtonTone, app_icon, prominent_icon_button},
    runtime::RuntimeCommand,
    theme::{MONO_FONT_FAMILY, THEME},
};

const CONTEXT_SEGMENTS: usize = 14;

impl PiApp {
    pub(super) fn render_composer_controls(
        &self,
        entity: WeakEntity<Self>,
        show_usage: bool,
    ) -> AnyElement {
        let mut footer = div()
            .id("composer-footer-controls")
            .min_w_0()
            .flex_1()
            .flex()
            .items_center()
            .overflow_x_scroll()
            .track_scroll(&self.composer_footer_scroll)
            .child(models::render(self, entity));

        if show_usage {
            let usage = composer_usage(self);
            let session_totals = session_change_totals(&self.changes.set);
            if has_meaningful_usage(&usage)
                || session_totals.additions.is_some()
                || session_totals.deletions.is_some()
            {
                footer = footer
                    .child(separator())
                    .child(render_usage(&usage, &session_totals));
            }
        }
        footer.child(div().min_w_0().flex_1()).into_any_element()
    }

    pub(super) fn render_composer_actions(
        &self,
        entity: WeakEntity<Self>,
        primary_action: Option<&'static str>,
    ) -> AnyElement {
        let send_entity = entity.clone();
        let abort_entity = entity;
        div()
            .flex_none()
            .flex()
            .items_center()
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(THEME.space.xs)
                    .when(self.snapshot.conversation.running, |actions| {
                        actions.child(
                            prominent_icon_button(
                                "abort",
                                AppIcon::Stop,
                                "Abort",
                                ButtonTone::Quiet,
                                move |_, cx| {
                                    let _ = abort_entity
                                        .update(cx, |this, _| this.send(RuntimeCommand::Abort));
                                },
                            )
                            .text_color(THEME.colors.error),
                        )
                    })
                    .when_some(primary_action, |actions, label| {
                        actions.child(prominent_icon_button(
                            "send",
                            AppIcon::ArrowUp,
                            label,
                            ButtonTone::Accent,
                            move |window, cx| {
                                let _ = send_entity.update(cx, |this, cx| {
                                    let value = this.composer.read(cx).value().trim().to_owned();
                                    if !value.is_empty() || this.has_composer_images() {
                                        this.submit(value, this.enter_mode(), window, cx);
                                    }
                                });
                            },
                        ))
                    }),
            )
            .into_any_element()
    }
}

fn render_usage(usage: &ComposerUsage, session_totals: &SessionChangeTotals) -> AnyElement {
    let mut row = div()
        .flex_none()
        .flex()
        .items_center()
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small)
        .child(context_metric(usage));
    if let Some(rate) = usage.cache_hit_rate {
        row = row.child(separator()).child(labeled_metric(
            "CH",
            "Cache hit rate",
            format!("{rate:.0}%"),
            THEME.colors.success,
        ));
    }
    if usage.aggregate.input > 0 {
        row = row.child(separator()).child(simple_metric(
            Some(AppIcon::ArrowUp),
            "Input tokens",
            format_tokens(usage.aggregate.input),
            THEME.colors.text,
        ));
    }
    if usage.aggregate.output > 0 {
        row = row.child(separator()).child(simple_metric(
            Some(AppIcon::ArrowDown),
            "Output tokens",
            format_tokens(usage.aggregate.output),
            THEME.colors.text,
        ));
    }
    if session_totals.additions.is_some() || session_totals.deletions.is_some() {
        row = row
            .child(separator())
            .child(session_change_metric(session_totals));
    }
    if usage.aggregate.cost_micros > 0 {
        row = row.child(separator()).child(simple_metric(
            None,
            "Cost",
            format_cost(usage.aggregate.cost_micros),
            THEME.colors.text,
        ));
    }
    row.into_any_element()
}

fn session_change_metric(totals: &SessionChangeTotals) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .whitespace_nowrap()
        .child(
            div().text_color(THEME.colors.success).child(
                totals
                    .additions
                    .map_or_else(|| "+—".to_owned(), |count| format!("+{count}")),
            ),
        )
        .child(
            div().text_color(THEME.colors.error).child(
                totals
                    .deletions
                    .map_or_else(|| "-—".to_owned(), |count| format!("-{count}")),
            ),
        )
        .into_any_element()
}

fn context_metric(usage: &ComposerUsage) -> AnyElement {
    let value = match (usage.context_used, usage.context_total) {
        (Some(used), Some(total)) => format!("{} / {}", format_tokens(used), format_tokens(total)),
        (Some(used), None) => format!("{} / —", format_tokens(used)),
        (None, Some(total)) => format!("— / {}", format_tokens(total)),
        (None, None) => "— / —".into(),
    };
    let percent = usage.context_percent.unwrap_or(0.0).clamp(0.0, 100.0);
    let filled = if percent <= 0.0 {
        0
    } else {
        ((percent / 100.0 * CONTEXT_SEGMENTS as f64).round() as usize).clamp(1, CONTEXT_SEGMENTS)
    };
    let color = context_color(usage.context_percent);
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(THEME.space.sm)
        .child(
            div()
                .flex_none()
                .whitespace_nowrap()
                .text_color(THEME.colors.text)
                .child(value),
        )
        .child(context_meter(filled, color))
        .into_any_element()
}

fn context_meter(filled: usize, color: gpui::Rgba) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(THEME.border)
        .children((0..CONTEXT_SEGMENTS).map(|index| {
            div()
                .w(THEME.space.sm)
                .h(THEME.type_scale.caption)
                .bg(if index < filled {
                    color
                } else {
                    THEME.colors.border
                })
        }))
        .into_any_element()
}

fn labeled_metric(
    label: &'static str,
    accessible_label: &'static str,
    value: String,
    value_color: gpui::Rgba,
) -> AnyElement {
    let aria_label = format!("{accessible_label}: {value}");
    div()
        .id(accessible_label)
        .aria_label(aria_label)
        .flex_none()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .whitespace_nowrap()
        .child(
            div()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(THEME.colors.subtle)
                .child(label),
        )
        .child(div().text_color(value_color).child(value))
        .into_any_element()
}

fn simple_metric(
    icon: Option<AppIcon>,
    accessible_label: &'static str,
    value: String,
    value_color: gpui::Rgba,
) -> AnyElement {
    let aria_label = format!("{accessible_label}: {value}");
    div()
        .id(accessible_label)
        .aria_label(aria_label)
        .flex_none()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .whitespace_nowrap()
        .children(icon.map(|icon| {
            app_icon(icon, AppIconSize::Inline)
                .text_color(THEME.colors.subtle)
                .into_any_element()
        }))
        .child(div().text_color(value_color).child(value))
        .into_any_element()
}

pub(super) fn separator() -> AnyElement {
    div()
        .flex_none()
        .px(THEME.space.sm)
        .text_align(gpui::TextAlign::Center)
        .font_family(MONO_FONT_FAMILY)
        .text_color(THEME.colors.subtle)
        .child("•")
        .into_any_element()
}

fn context_color(percent: Option<f64>) -> gpui::Rgba {
    match percent {
        Some(percent) if percent > 90.0 => THEME.colors.error,
        Some(percent) if percent > 70.0 => THEME.colors.warning,
        Some(_) => THEME.colors.success,
        None => THEME.colors.border,
    }
}
