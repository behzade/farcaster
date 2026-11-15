mod runtime;
mod usage;

#[cfg(test)]
#[path = "footer_tests.rs"]
mod tests;

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _, PathBuilder,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, canvas, div, point,
    prelude::FluentBuilder as _, px,
};

use super::super::usage::{ComposerUsage, composer_usage, format_tokens};
use super::ComposerAction;
use crate::{
    app::FarcasterApp,
    app::ui::assets::AppIcon,
    app::ui::primitives::{AppIconSize, ButtonTone, icon_button_with_size},
    app::ui::theme::{MONO_FONT_FAMILY, theme},
    runtime::RuntimeSignal,
};

impl FarcasterApp {
    pub(in crate::app::views) fn render_composer_controls(
        &self,
        entity: WeakEntity<Self>,
        scroll: &gpui::ScrollHandle,
    ) -> AnyElement {
        let settings = div()
            .id("composer-settings")
            .min_w_0()
            .flex_1()
            .flex()
            .items_center()
            .gap(theme().space.md)
            .overflow_x_scroll()
            .track_scroll(scroll)
            .child(div().min_w_0().flex_1())
            .when_some(self.editable_draft_harness(), |footer, harness| {
                footer.child(super::start::harness_selector(
                    harness,
                    self.active_profile_id(),
                    self.settings.harness_profiles.list().unwrap_or_default(),
                    entity.clone(),
                ))
            })
            .child(runtime::render(self, entity.clone()));

        div()
            .id("composer-footer-controls")
            .min_w_0()
            .flex_1()
            .flex()
            .items_center()
            .gap(theme().space.md)
            .child(super::attachments::attach_button(entity))
            .child(settings)
            .into_any_element()
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
            .h(theme().controls.icon_button)
            .flex_none()
            .flex()
            .items_center()
            .px(theme().space.sm)
            .text_size(theme().type_scale.caption)
            .line_height(theme().type_scale.line_body)
            .text_color(theme().colors.muted)
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
            .when_some(context_value(&usage), |row, value| {
                row.child(context_metric(&usage, value))
                    .child(div().w(theme().space.md))
            })
            .child(usage::render(usage))
            .into_any_element()
    }

    pub(in crate::app::views) fn render_composer_actions(
        &self,
        entity: WeakEntity<Self>,
        primary_action: Option<ComposerAction>,
    ) -> AnyElement {
        div()
            .flex_none()
            .w(theme().controls.icon_button)
            .flex()
            .items_center()
            .justify_end()
            .when_some(primary_action, |slot, action| {
                let (icon, label) = match action {
                    ComposerAction::Send => (AppIcon::ArrowUp, "Send"),
                    ComposerAction::Steer => (AppIcon::ArrowUp, "Steer"),
                    ComposerAction::Stop => (AppIcon::Stop, "Stop"),
                    ComposerAction::Run => (AppIcon::ArrowUp, "Run"),
                };
                slot.child(
                    icon_button_with_size(
                        "composer-primary-action",
                        icon,
                        label,
                        AppIconSize::Control,
                        ButtonTone::Quiet,
                        move |window, cx| {
                            let _ = entity.update(cx, |this, cx| {
                                if action == ComposerAction::Stop {
                                    this.runtime.signal(
                                        this.composer.sessions.current_target(),
                                        RuntimeSignal::Abort,
                                    );
                                } else {
                                    let value =
                                        this.composer.input.read(cx).value().trim().to_owned();
                                    if !value.is_empty() || this.has_composer_attachments() {
                                        this.submit(value, this.enter_mode(), window, cx);
                                    }
                                }
                            });
                        },
                    )
                    .debug_selector(|| "composer-primary-action".into())
                    .rounded_full()
                    .bg(theme().colors.highlight)
                    .text_color(if action == ComposerAction::Stop {
                        theme().colors.error
                    } else {
                        theme().colors.text
                    }),
                )
            })
            .into_any_element()
    }
}

fn context_metric(usage: &ComposerUsage, value: String) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(theme().space.sm)
        .child(div().text_color(theme().colors.muted).child("Context"))
        .when_some(usage.context_percent, |metric, percent| {
            metric.child(context_meter(
                percent.clamp(0.0, 100.0),
                context_color(Some(percent)),
            ))
        })
        .child(
            div()
                .flex_none()
                .whitespace_nowrap()
                .text_color(theme().colors.text)
                .child(value),
        )
        .into_any_element()
}

fn context_value(usage: &ComposerUsage) -> Option<String> {
    match (usage.context_used, usage.context_total) {
        (Some(used), Some(total)) => {
            Some(format!("{}/{}", format_tokens(used), format_tokens(total)))
        }
        (Some(used), None) => Some(format_tokens(used)),
        (None, _) => usage
            .context_percent
            .map(|percent| format!("{percent:.0}%")),
    }
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

fn context_color(percent: Option<f64>) -> gpui::Rgba {
    match percent {
        Some(percent) if percent > 90.0 => theme().colors.error,
        Some(percent) if percent > 70.0 => theme().colors.warning,
        Some(_) => theme().colors.success,
        None => theme().colors.border,
    }
}
