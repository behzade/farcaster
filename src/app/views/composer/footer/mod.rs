mod runtime;

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _, PathBuilder,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, canvas, div, point,
    prelude::FluentBuilder as _, px,
};
use gpui_component::tooltip::Tooltip;

use super::super::usage::{
    ComposerUsage, composer_usage, format_cost, format_tokens, has_meaningful_usage,
};
use crate::{
    app::FarcasterApp,
    app::ui::assets::AppIcon,
    app::ui::primitives::{AppIconSize, ButtonTone, app_icon, prominent_icon_button},
    app::ui::theme::{MONO_FONT_FAMILY, theme()},
    runtime::RuntimeCommand,
};

impl FarcasterApp {
    pub(in crate::app::views) fn render_composer_controls(
        &self,
        entity: WeakEntity<Self>,
        scroll: &gpui::ScrollHandle,
    ) -> AnyElement {
        let footer = div()
            .id("composer-footer-controls")
            .min_w_0()
            .flex_1()
            .flex()
            .items_center()
            .overflow_x_scroll()
            .track_scroll(scroll)
            .when_some(self.editable_draft_harness(), |footer, harness| {
                footer
                    .child(super::start::harness_selector(
                        harness,
                        self.active_profile_id(),
                        self.settings.harness_profiles.list().unwrap_or_default(),
                        entity.clone(),
                    ))
                    .child(separator())
            })
            .child(runtime::render(self, entity));

        footer.child(div().min_w_0().flex_1()).into_any_element()
    }

    pub(in crate::app::views) fn render_composer_status(
        &self,
        scroll: &gpui::ScrollHandle,
        mode: Option<&str>,
    ) -> AnyElement {
        let usage = composer_usage(self);
        div()
            .id("composer-status")
            .w_full()
            .min_w_0()
            .h(theme().size(28.0))
            .flex_none()
            .flex()
            .items_center()
            .px(theme().size(12.0))
            .overflow_x_scroll()
            .track_scroll(scroll)
            .when_some(mode, |row, mode| {
                row.child(
                    div()
                        .flex_none()
                        .pr(theme().space.sm)
                        .font_family(MONO_FONT_FAMILY)
                        .text_size(theme().type_scale.caption)
                        .text_color(theme().colors.indicator)
                        .whitespace_nowrap()
                        .child(mode.to_owned()),
                )
            })
            .child(div().min_w_0().flex_1())
            .when(has_meaningful_usage(&usage), |row| {
                row.child(render_usage(&usage))
            })
            .into_any_element()
    }

    pub(in crate::app::views) fn render_composer_actions(
        &self,
        entity: WeakEntity<Self>,
        primary_action: Option<&'static str>,
    ) -> AnyElement {
        let send_entity = entity.clone();
        let abort_entity = entity;
        div()
            .absolute()
            .right(theme().size(12.0))
            .bottom(theme().size(10.0))
            .occlude()
            .flex()
            .items_center()
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(theme().space.xs)
                    .when(self.snapshot.conversation.running, |actions| {
                        actions.child(
                            prominent_icon_button(
                                "abort",
                                AppIcon::Stop,
                                "Abort",
                                ButtonTone::Quiet,
                                move |_, cx| {
                                    let _ = abort_entity.update(cx, |this, cx| {
                                        this.send(RuntimeCommand::Abort, cx)
                                    });
                                },
                            )
                            .text_color(theme().colors.error),
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
                                    let value =
                                        this.composer.input.read(cx).value().trim().to_owned();
                                    if !value.is_empty() || this.has_composer_attachments() {
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

fn render_usage(usage: &ComposerUsage) -> AnyElement {
    let mut row = div()
        .flex_none()
        .flex()
        .items_center()
        .font_family(MONO_FONT_FAMILY)
        .text_size(theme().type_scale.caption)
        .child(context_metric(usage));
    if let Some(rate) = usage.cache_hit_rate {
        row = row.child(separator()).child(labeled_metric(
            "CH",
            "Cache hit rate",
            format!("{rate:.0}%"),
            theme().colors.success,
            None,
        ));
    }
    if usage.aggregate.input > 0 {
        row = row.child(separator()).child(simple_metric(
            Some(AppIcon::ArrowDown),
            "Input tokens",
            format_tokens(usage.aggregate.input),
            theme().colors.muted,
        ));
    }
    if usage.aggregate.output > 0 {
        row = row.child(separator()).child(simple_metric(
            Some(AppIcon::ArrowUp),
            "Output tokens",
            format_tokens(usage.aggregate.output),
            theme().colors.text,
        ));
    }
    if usage.aggregate.cost_micros > 0 {
        row = row.child(separator()).child(simple_metric(
            None,
            "Cost",
            format_cost(usage.aggregate.cost_micros),
            theme().colors.text,
        ));
    }
    if let Some(weekly) = usage.weekly {
        let remaining = weekly.remaining_percent.clamp(0.0, 100.0);
        let value = format!("{remaining:.0}%");
        let tooltip = weekly.resets_at.and_then(format_reset_time).map_or_else(
            || format!("Weekly usage remaining: {value}"),
            |reset| format!("Weekly usage remaining: {value} · resets {reset}"),
        );
        row = row.child(separator()).child(labeled_metric(
            "W",
            "Weekly usage remaining",
            value,
            weekly_color(remaining),
            Some(tooltip),
        ));
    }
    row.into_any_element()
}

fn format_reset_time(timestamp: i64) -> Option<String> {
    let value = time::OffsetDateTime::from_unix_timestamp(timestamp).ok()?;
    let format = time::format_description::parse_borrowed::<3>(
        "[month repr:short] [day padding:none] at [hour]:[minute] UTC",
    )
    .ok()?;
    value.format(&format).ok()
}

fn weekly_color(remaining: f64) -> gpui::Rgba {
    if remaining <= 10.0 {
        theme().colors.error
    } else if remaining <= 25.0 {
        theme().colors.warning
    } else {
        theme().colors.text
    }
}

fn context_metric(usage: &ComposerUsage) -> AnyElement {
    let value = match (usage.context_used, usage.context_total) {
        (Some(used), Some(total)) => format!("{}/{}", format_tokens(used), format_tokens(total)),
        (Some(used), None) => format!("{}/—", format_tokens(used)),
        (None, Some(total)) => format!("—/{}", format_tokens(total)),
        (None, None) => "—/—".into(),
    };
    let percent = usage.context_percent.unwrap_or(0.0).clamp(0.0, 100.0);
    let color = context_color(usage.context_percent);
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(theme().space.sm)
        .child(context_meter(percent, color))
        .child(
            div()
                .flex_none()
                .whitespace_nowrap()
                .text_color(theme().colors.text)
                .child(value),
        )
        .into_any_element()
}

fn context_meter(percent: f64, color: gpui::Rgba) -> AnyElement {
    div()
        .size(theme().size(14.0))
        .flex_none()
        .rounded_full()
        .overflow_hidden()
        .border(theme().border)
        .border_color(theme().colors.border)
        .bg(theme().colors.border)
        .child(
            canvas(
                |bounds, _, _| bounds,
                move |bounds, _, window, _| {
                    if percent <= 0.0 {
                        return;
                    }
                    let radius = f32::from(bounds.size.width.min(bounds.size.height)) / 2.0;
                    let center_x = f32::from(bounds.origin.x) + radius;
                    let center_y = f32::from(bounds.origin.y) + radius;
                    let fraction = (percent / 100.0) as f32;
                    let steps = (fraction * 32.0).ceil().max(1.0) as usize;
                    let mut points = Vec::with_capacity(steps + 2);
                    points.push(point(px(center_x), px(center_y)));
                    for step in 0..=steps {
                        let progress = fraction * step as f32 / steps as f32;
                        let angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * progress;
                        points.push(point(
                            px(center_x + radius * angle.cos()),
                            px(center_y + radius * angle.sin()),
                        ));
                    }
                    let mut builder = PathBuilder::fill();
                    builder.add_polygon(&points, true);
                    if let Ok(path) = builder.build() {
                        window.paint_path(path, color);
                    }
                },
            )
            .size_full(),
        )
        .into_any_element()
}

fn labeled_metric(
    label: &'static str,
    accessible_label: &'static str,
    value: String,
    value_color: gpui::Rgba,
    tooltip: Option<String>,
) -> AnyElement {
    let aria_label = format!("{accessible_label}: {value}");
    div()
        .id(accessible_label)
        .aria_label(aria_label)
        .flex_none()
        .flex()
        .items_center()
        .gap(theme().space.xs)
        .whitespace_nowrap()
        .child(
            div()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme().colors.subtle)
                .child(label),
        )
        .child(div().text_color(value_color).child(value))
        .when_some(tooltip, |metric, tooltip| {
            metric.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        })
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
        .gap(theme().space.xs)
        .whitespace_nowrap()
        .children(icon.map(|icon| {
            app_icon(icon, AppIconSize::Inline)
                .text_color(theme().colors.subtle)
                .into_any_element()
        }))
        .child(div().text_color(value_color).child(value))
        .into_any_element()
}

pub(in crate::app::views) fn separator() -> AnyElement {
    div()
        .flex_none()
        .px(theme().size(6.0))
        .text_align(gpui::TextAlign::Center)
        .font_family(MONO_FONT_FAMILY)
        .text_color(theme().colors.subtle)
        .child("/")
        .into_any_element()
}

fn context_color(percent: Option<f64>) -> gpui::Rgba {
    match percent {
        Some(percent) if percent > 90.0 => theme().colors.error,
        Some(percent) if percent > 70.0 => theme().colors.warning,
        Some(_) => theme().colors.success,
        None => theme().colors.border,
    }
}
