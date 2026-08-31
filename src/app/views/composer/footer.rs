use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _, PathBuilder,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, canvas, div, point,
    prelude::FluentBuilder as _, px,
};

use super::{
    super::usage::{
        ComposerUsage, composer_usage, format_cost, format_tokens, has_meaningful_usage,
    },
    models,
};
use crate::{
    app::FarcasterApp,
    app::ui::assets::AppIcon,
    app::ui::primitives::{AppIconSize, ButtonTone, app_icon, prominent_icon_button},
    app::ui::theme::{MONO_FONT_FAMILY, THEME},
    runtime::RuntimeCommand,
};

impl FarcasterApp {
    pub(in crate::app::views) fn render_composer_controls(
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
            if has_meaningful_usage(&usage) {
                footer = footer.child(separator()).child(render_usage(&usage));
            }
        }
        footer.child(div().min_w_0().flex_1()).into_any_element()
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
            .right(px(12.0))
            .bottom(px(10.0))
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
        .text_size(THEME.type_scale.caption)
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
            Some(AppIcon::ArrowDown),
            "Input tokens",
            format_tokens(usage.aggregate.input),
            THEME.colors.muted,
        ));
    }
    if usage.aggregate.output > 0 {
        row = row.child(separator()).child(simple_metric(
            Some(AppIcon::ArrowUp),
            "Output tokens",
            format_tokens(usage.aggregate.output),
            THEME.colors.text,
        ));
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
        .gap(THEME.space.sm)
        .child(context_meter(percent, color))
        .child(
            div()
                .flex_none()
                .whitespace_nowrap()
                .text_color(THEME.colors.text)
                .child(value),
        )
        .into_any_element()
}

fn context_meter(percent: f64, color: gpui::Rgba) -> AnyElement {
    div()
        .size(px(14.0))
        .flex_none()
        .rounded_full()
        .overflow_hidden()
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.border)
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

pub(in crate::app::views) fn separator() -> AnyElement {
    div()
        .flex_none()
        .px(px(6.0))
        .text_align(gpui::TextAlign::Center)
        .font_family(MONO_FONT_FAMILY)
        .text_color(THEME.colors.subtle)
        .child("/")
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
