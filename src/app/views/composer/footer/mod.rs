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

use super::super::usage::{ComposerUsage, composer_usage};
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
        let usage = composer_usage(self);
        let context = context_value(&usage);
        let settings = div()
            .id("composer-settings")
            .min_w_0()
            .flex_1()
            .flex()
            .items_center()
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
            .when_some(context, |row, value| {
                let ring = context_meter(usage.context_percent);
                row.child(usage::render(usage, ring, value))
            })
            .child(runtime::render(self, entity.clone()));

        div()
            .id("composer-footer-controls")
            .min_w_0()
            .flex_1()
            .flex()
            .items_center()
            .gap(theme().space.sm)
            .child(super::attachments::attach_button(entity))
            .child(settings)
            .into_any_element()
    }

    pub(in crate::app::views) fn render_composer_status(
        &self,
        scroll: &gpui::ScrollHandle,
        mode: Option<&str>,
    ) -> AnyElement {
        let Some(mode) = mode else {
            return div().into_any_element();
        };
        let status = div()
            .id("composer-status")
            .debug_selector(|| "composer-status".into())
            .w_full()
            .min_w_0()
            .h(theme().type_scale.line_body)
            .flex_none()
            .flex()
            .items_center()
            .px(theme().space.md)
            .text_size(theme().type_scale.caption)
            .line_height(theme().type_scale.line_body)
            .text_color(theme().colors.muted)
            .overflow_x_scroll()
            .track_scroll(scroll)
            .child(
                div()
                    .flex_none()
                    .pr(theme().space.sm)
                    .font_family(MONO_FONT_FAMILY)
                    .text_size(theme().type_scale.caption)
                    .text_color(theme().colors.indicator)
                    .whitespace_nowrap()
                    .child(mode.to_owned()),
            );
        status.into_any_element()
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

fn context_value(usage: &ComposerUsage) -> Option<String> {
    let count = match (usage.context_used, usage.context_total) {
        (Some(used), Some(total)) => Some(format!("{used} / {total} tokens")),
        (Some(used), None) => Some(format!("{used} tokens")),
        _ => None,
    };
    match (count, usage.context_percent) {
        (Some(count), Some(percent)) => Some(format!("{count} · {percent:.0}%")),
        (Some(count), None) => Some(count),
        (None, Some(percent)) => Some(format!("{percent:.0}%")),
        (None, None) => None,
    }
}

fn context_meter(percent: Option<f64>) -> AnyElement {
    canvas(
        |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let size = f32::from(bounds.size.width.min(bounds.size.height));
            // Match the circle and stroke in the app's 256-unit Phosphor icons.
            let stroke = size * 16.0 / 256.0;
            let radius = size * 96.0 / 256.0;
            let center = bounds.center();
            let point_at = |progress: f32| {
                let angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * progress;
                point(
                    center.x + px(radius * angle.cos()),
                    center.y + px(radius * angle.sin()),
                )
            };
            let mut paint_arc = |fraction: f32, color| {
                let steps = (fraction * 64.0).ceil().max(1.0) as usize;
                let mut path = PathBuilder::stroke(px(stroke));
                path.move_to(point_at(0.0));
                for step in 1..=steps {
                    path.line_to(point_at(fraction * step as f32 / steps as f32));
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            };
            paint_arc(1.0, theme().colors.border);
            if let Some(percent) = percent.filter(|percent| *percent > 0.0) {
                let fraction = (percent.clamp(0.0, 100.0) / 100.0) as f32;
                let color = context_color(percent);
                paint_arc(fraction, color);
                for end in [point_at(0.0), point_at(fraction)] {
                    window.paint_quad(
                        gpui::fill(
                            gpui::Bounds::new(
                                end - point(px(stroke / 2.0), px(stroke / 2.0)),
                                gpui::size(px(stroke), px(stroke)),
                            ),
                            color,
                        )
                        .corner_radii(px(stroke / 2.0)),
                    );
                }
            }
        },
    )
    .size(theme().icons.control)
    .flex_none()
    .into_any_element()
}

fn context_color(percent: f64) -> gpui::Rgba {
    if percent > 90.0 {
        theme().colors.error
    } else if percent > 70.0 {
        theme().colors.warning
    } else {
        theme().colors.text
    }
}
