use crate::{
    app::{
        ui::{
            assets::AppIcon,
            primitives::{AppIconSize, ButtonTone, Popover, app_icon, dropdown_content_button},
            theme::theme,
        },
        views::usage::{ComposerUsage, format_cost, format_tokens, has_reported_usage},
    },
    sessions::UsageSummary,
};
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Styled as _, Transformation, div, percentage,
    prelude::FluentBuilder as _,
};
use gpui_component::ThemeStyled as _;

const COMPACTING_LABEL: &str = "Compacting context…";

#[derive(IntoElement)]
pub(super) struct UsageControl {
    pub(super) usage: ComposerUsage,
    pub(super) compacting: bool,
}

impl gpui::RenderOnce for UsageControl {
    fn render(self, window: &mut gpui::Window, cx: &mut App) -> impl IntoElement {
        let Self { usage, compacting } = self;
        let (label, content) = if compacting {
            let spinner = app_icon(AppIcon::SpinnerGap, AppIconSize::Control)
                .text_color(theme().colors.indicator)
                .with_animation(
                    "context-compacting",
                    Animation::new(std::time::Duration::from_millis(1400)).repeat(),
                    |icon, delta| icon.transform(Transformation::rotate(percentage(delta))),
                );
            (COMPACTING_LABEL.to_owned(), spinner.into_any_element())
        } else if let Some(value) = super::context_value(&usage) {
            (
                format!("Context · {value}"),
                super::context_meter(usage.context_percent),
            )
        } else {
            return gpui::Empty.into_any_element();
        };
        let focus = window
            .use_keyed_state("composer-usage-focus", cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let popover = Popover::new("composer-usage-popover")
            .anchor(gpui::Anchor::BottomLeft)
            .appearance(false)
            .trigger(
                dropdown_content_button(
                    "composer-context",
                    label,
                    content,
                    ButtonTone::Quiet,
                    true,
                )
                .track_focus(&focus)
                .dropdown_caret(false)
                .size(theme().controls.icon_button)
                .p(gpui::px(0.0))
                .flex_none()
                .debug_selector(|| "composer-context-trigger".into()),
            )
            .content(move |_, _, cx| details(&usage, cx));
        div()
            .relative()
            .flex_none()
            .child(popover)
            .when(compacting && focus.is_focused(window), |slot| {
                slot.child(
                    gpui::deferred(
                        div()
                            .absolute()
                            .debug_selector(|| "composer-compaction-focus-hint".into())
                            .bottom(theme().controls.icon_button + theme().space.xs)
                            .whitespace_nowrap()
                            .child(
                                gpui_component::tooltip::Tooltip::new(COMPACTING_LABEL)
                                    .build(window, cx),
                            ),
                    )
                    .with_priority(200),
                )
            })
            .into_any_element()
    }
}

fn details(usage: &ComposerUsage, cx: &App) -> AnyElement {
    let scopes = if usage.worker_count > 0 {
        vec![
            ("Session".to_owned(), usage.session),
            (format!("Workers ({})", usage.worker_count), usage.workers),
            ("Total".into(), usage.aggregate),
        ]
    } else {
        vec![("Session".to_owned(), usage.session)]
    };
    let mut tokens = div().flex().flex_col().gap(theme().space.sm).child(
        div()
            .flex()
            .child(div().flex_1())
            .children(scopes.iter().map(|(label, _)| {
                div()
                    .w(theme().size(76.0))
                    .text_right()
                    .font_weight(FontWeight::MEDIUM)
                    .child(label.clone())
            })),
    );
    for (label, metric) in [
        ("Input", UsageMetric::Input),
        ("Output", UsageMetric::Output),
        ("Cache read", UsageMetric::CacheRead),
        ("Cache write", UsageMetric::CacheWrite),
        ("Cost", UsageMetric::Cost),
    ] {
        if matches!(
            metric,
            UsageMetric::CacheRead | UsageMetric::CacheWrite | UsageMetric::Cost
        ) && scopes.iter().all(|(_, scope)| metric.value(scope) == 0)
        {
            continue;
        }
        tokens = tokens.child(
            div()
                .flex()
                .items_center()
                .child(div().flex_1().text_color(theme().colors.muted).child(label))
                .children(scopes.iter().map(|(_, scope)| {
                    let value = if has_reported_usage(scope) {
                        metric.format(scope)
                    } else {
                        "—".into()
                    };
                    div().w(theme().size(76.0)).text_right().child(value)
                })),
        );
    }
    div()
        .popover_style(cx)
        .debug_selector(|| "composer-usage-details".into())
        .w(theme().size(360.0))
        .p(theme().space.md)
        .flex()
        .flex_col()
        .gap(theme().space.md)
        .child(div().font_weight(FontWeight::SEMIBOLD).child("Usage"))
        .child(div().text_size(theme().type_scale.caption).child(tokens))
        .when_some(usage.cache_hit_rate, |body, rate| {
            body.child(metric("Session cache hit", format!("{rate:.0}%")))
        })
        .into_any_element()
}

#[derive(Clone, Copy)]
enum UsageMetric {
    Input,
    Output,
    CacheRead,
    CacheWrite,
    Cost,
}

impl UsageMetric {
    fn value(self, usage: &UsageSummary) -> u64 {
        match self {
            Self::Input => usage.input,
            Self::Output => usage.output,
            Self::CacheRead => usage.cache_read,
            Self::CacheWrite => usage.cache_write,
            Self::Cost => usage.cost_micros,
        }
    }
    fn format(self, usage: &UsageSummary) -> String {
        if matches!(self, Self::Cost) {
            format_cost(self.value(usage))
        } else {
            format_tokens(self.value(usage))
        }
    }
}

fn metric(label: &str, value: String) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(theme().space.md)
        .child(
            div()
                .flex_1()
                .text_color(theme().colors.muted)
                .child(label.to_owned()),
        )
        .child(value)
        .into_any_element()
}
