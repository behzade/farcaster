use crate::{
    app::{
        ui::{
            primitives::{ButtonTone, Popover, dropdown_content_button},
            theme::theme,
        },
        views::usage::{ComposerUsage, format_cost, format_tokens, has_reported_usage},
    },
    sessions::UsageSummary,
};
use gpui::{
    AnyElement, App, FontWeight, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, div, prelude::FluentBuilder as _,
};
use gpui_component::ThemeStyled as _;

pub(super) fn render(usage: ComposerUsage, content: AnyElement) -> AnyElement {
    Popover::new("composer-usage-popover")
        .anchor(gpui::Anchor::BottomLeft)
        .appearance(false)
        .trigger(
            dropdown_content_button(
                "composer-context",
                "Usage details",
                content,
                ButtonTone::Quiet,
                true,
            )
            .dropdown_caret(false)
            .h(theme().type_scale.line_body)
            .py(gpui::px(0.0))
            .px(gpui::px(0.0))
            .debug_selector(|| "composer-context-trigger".into()),
        )
        .content(move |_, _, cx| details(&usage, cx))
        .into_any_element()
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
