use gpui::{
    Anchor, AnyElement, FontWeight, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, px, relative,
};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

use super::super::PiApp;
use crate::{
    primitives::{ButtonTone, dropdown_button},
    sessions::{UsageSummary, descendant_sessions, root_session_for_path},
    theme::{MONO_FONT_FAMILY, THEME},
};

// Dynamic labels and metrics stay inside these fixed slots so the footer never reflows.
const PROVIDER_CONTROL_WIDTH: f32 = 125.0;
const MODEL_CONTROL_WIDTH: f32 = 180.0;
const EFFORT_CONTROL_WIDTH: f32 = 90.0;
const CONTEXT_CONTROL_WIDTH: f32 = 192.0;
const USAGE_CONTROL_WIDTH: f32 = 220.0;
const SEPARATOR_WIDTH: f32 = 13.0;

impl PiApp {
    pub(super) fn render_composer_controls(&self, entity: WeakEntity<Self>) -> AnyElement {
        let identity = self.snapshot.session_identity();
        let selected_model = identity.model;
        let selected_provider = identity
            .provider
            .map(str::to_owned)
            .or_else(|| {
                self.snapshot
                    .models
                    .first()
                    .map(|model| model.provider.clone())
            })
            .unwrap_or_else(|| "Provider".into());
        let model_label = selected_model
            .map(|model| bounded_label(&model.name, 18))
            .unwrap_or_else(|| "Model".into());
        let mut providers = self
            .snapshot
            .models
            .iter()
            .map(|model| model.provider.clone())
            .collect::<Vec<_>>();
        providers.sort();
        providers.dedup();
        let provider_button_label = if providers.is_empty() {
            "Provider".into()
        } else {
            bounded_label(&selected_provider, 10)
        };
        let model_button_label = selected_model.map_or_else(|| "Model".into(), |_| model_label);
        let provider_models = self
            .snapshot
            .models
            .iter()
            .filter(|model| model.provider == selected_provider)
            .cloned()
            .collect::<Vec<_>>();
        let effort = identity.effort;
        let efforts = self.snapshot.thinking_levels.clone();
        let provider_entity = entity.clone();
        let model_entity = entity.clone();
        let effort_entity = entity;
        let usage = composer_usage(self);

        let provider = dropdown_button(
            "select-provider",
            provider_button_label,
            ButtonTone::Quiet,
            !providers.is_empty(),
        )
        .w_full()
        .font_family(MONO_FONT_FAMILY)
        .text_color(THEME.colors.text)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            let mut menu = menu
                .min_w(px(PROVIDER_CONTROL_WIDTH))
                .max_h(px(420.0))
                .label("Provider");
            for provider in &providers {
                let target = provider.clone();
                let entity = provider_entity.clone();
                menu = menu.item(
                    PopupMenuItem::new(provider.clone()).on_click(move |_, _, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.select_provider(&target, cx);
                        });
                    }),
                );
            }
            menu
        });
        let model = dropdown_button(
            "select-model",
            model_button_label,
            ButtonTone::Quiet,
            !provider_models.is_empty(),
        )
        .w_full()
        .font_family(MONO_FONT_FAMILY)
        .text_color(THEME.colors.text)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            let mut menu = menu
                .min_w(px(MODEL_CONTROL_WIDTH))
                .max_h(px(480.0))
                .label("Model");
            for model in &provider_models {
                let target = model.clone();
                let entity = model_entity.clone();
                menu = menu.item(PopupMenuItem::new(model.name.clone()).on_click(
                    move |_, _, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.select_model(&target, cx);
                        });
                    },
                ));
            }
            menu
        });

        div()
            .id("composer-footer-controls")
            .min_w_0()
            .flex_1()
            .flex()
            .items_stretch()
            .overflow_x_scroll()
            .track_scroll(&self.composer_footer_scroll)
            .child(control_cell(
                "PROVIDER",
                PROVIDER_CONTROL_WIDTH,
                provider.into_any_element(),
            ))
            .child(footer_separator())
            .child(control_cell(
                "MODEL",
                MODEL_CONTROL_WIDTH,
                model.into_any_element(),
            ))
            .child(footer_separator())
            .child(control_cell(
                "EFFORT",
                EFFORT_CONTROL_WIDTH,
                effort_selector(effort, &efforts, effort_entity),
            ))
            .child(div().min_w_0().flex_1())
            .child(footer_separator())
            .child(context_cell(&usage))
            .child(footer_separator())
            .child(usage_cell(&usage))
            .into_any_element()
    }
}

fn effort_selector(
    selected: Option<&str>,
    efforts: &[String],
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let label = selected.map_or_else(|| "Effort".into(), effort_label);
    let efforts = efforts.to_vec();

    dropdown_button(
        "select-effort",
        label,
        ButtonTone::Quiet,
        !efforts.is_empty(),
    )
    .w_full()
    .font_family(MONO_FONT_FAMILY)
    .text_color(effort_color(selected.unwrap_or("off")))
    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
        let mut menu = menu
            .min_w(px(EFFORT_CONTROL_WIDTH))
            .max_h(px(420.0))
            .label("Effort");
        for effort in &efforts {
            let target = effort.clone();
            let click_entity = entity.clone();
            menu = menu.item(
                PopupMenuItem::new(effort_label(effort)).on_click(move |_, _, cx| {
                    let _ = click_entity.update(cx, |this, cx| {
                        this.set_thinking_level(target.clone(), cx);
                    });
                }),
            );
        }
        menu
    })
    .into_any_element()
}

fn control_cell(label: &'static str, width: f32, control: AnyElement) -> AnyElement {
    fixed_footer_cell(width)
        .justify_center()
        .gap(px(2.0))
        .px(THEME.space.sm)
        .child(footer_label(label))
        .child(control)
        .into_any_element()
}

#[derive(Clone, Debug)]
struct ComposerUsage {
    context: super::run_panel::ContextSummary,
    aggregate: UsageSummary,
    cache_hit_rate: Option<f64>,
    message_count: usize,
}

fn composer_usage(app: &PiApp) -> ComposerUsage {
    let root = root_session_for_path(
        &app.all_sessions,
        app.snapshot.selected_session.as_deref(),
    );
    let descendants = root
        .map(|root| descendant_sessions(&app.all_sessions, &root.id))
        .unwrap_or_default();
    let mut aggregate = root.map(|root| root.usage).unwrap_or_default();
    for (session, _) in &descendants {
        aggregate.add(session.usage);
    }
    let message_count = root.map_or(0, |session| session.message_count)
        + descendants
            .iter()
            .map(|(session, _)| session.message_count)
            .sum::<usize>();
    ComposerUsage {
        context: super::run_panel::context_summary(
            super::run_panel::visible_context_stats(
                &app.snapshot.stats,
                app.snapshot.conversation.running,
            ),
            aggregate.cost_micros,
        ),
        aggregate,
        cache_hit_rate: app
            .snapshot
            .conversation
            .average_cache_hit_rate
            .filter(|rate| rate.is_finite())
            .map(|rate| rate.clamp(0.0, 100.0)),
        message_count,
    }
}

fn context_cell(usage: &ComposerUsage) -> AnyElement {
    let percent = usage
        .context
        .percent
        .map_or_else(|| "—".into(), |percent| format!("{percent:.0}%"));
    let used = usage.context.used.map_or_else(
        || "— used".into(),
        |value| format!("{} used", super::run_panel::compact_number(value)),
    );
    let remaining = usage.context.remaining.map_or_else(
        || "— remaining".into(),
        |value| format!("{} remaining", super::run_panel::compact_number(value)),
    );
    let color = if usage.context.warning {
        THEME.colors.warning
    } else {
        THEME.colors.text
    };
    fixed_footer_cell(CONTEXT_CONTROL_WIDTH)
        .justify_center()
        .gap(px(2.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(footer_label("CONTEXT"))
                .child(fixed_value(42.0, percent, color, gpui::TextAlign::Right)),
        )
        .child(
            div()
                .h(px(3.0))
                .w_full()
                .rounded_full()
                .overflow_hidden()
                .bg(THEME.colors.border)
                .child(
                    div()
                        .h_full()
                        .w(relative(
                            usage.context.percent.unwrap_or(0.0).clamp(0.0, 100.0) as f32
                                / 100.0,
                        ))
                        .rounded_full()
                        .bg(if usage.context.warning {
                            THEME.colors.warning
                        } else if usage.context.percent.is_some() {
                            THEME.colors.accent
                        } else {
                            THEME.colors.border
                        }),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .child(fixed_value(
                    72.0,
                    used,
                    THEME.colors.muted,
                    gpui::TextAlign::Left,
                ))
                .child(metric_separator())
                .child(fixed_value(
                    108.0,
                    remaining,
                    THEME.colors.muted,
                    gpui::TextAlign::Right,
                )),
        )
        .into_any_element()
}

fn usage_cell(usage: &ComposerUsage) -> AnyElement {
    let input = format!(
        "{} in",
        super::run_panel::compact_number(usage.aggregate.input)
    );
    let output = format!(
        "{} out",
        super::run_panel::compact_number(usage.aggregate.output)
    );
    let hit_rate = usage
        .cache_hit_rate
        .map_or_else(|| "— hit".into(), |rate| format!("{rate:.0}% hit"));
    fixed_footer_cell(USAGE_CONTROL_WIDTH)
        .justify_center()
        .gap(px(2.0))
        .child(
            div()
                .flex()
                .items_center()
                .child(fixed_label(46.0, "TOKENS"))
                .child(fixed_value(
                    70.0,
                    input,
                    THEME.colors.muted,
                    gpui::TextAlign::Right,
                ))
                .child(metric_separator())
                .child(fixed_value(
                    82.0,
                    output,
                    THEME.colors.muted,
                    gpui::TextAlign::Right,
                )),
        )
        .child(
            div()
                .flex()
                .items_center()
                .child(fixed_value(
                    58.0,
                    super::run_panel::format_cost(usage.context.cost_micros),
                    THEME.colors.muted,
                    gpui::TextAlign::Left,
                ))
                .child(metric_separator())
                .child(fixed_value(
                    70.0,
                    hit_rate,
                    THEME.colors.muted,
                    gpui::TextAlign::Right,
                ))
                .child(metric_separator())
                .child(fixed_value(
                    68.0,
                    format!("{} msg", usage.message_count),
                    THEME.colors.muted,
                    gpui::TextAlign::Right,
                )),
        )
        .child(
            div()
                .flex()
                .items_center()
                .child(fixed_label(46.0, "CACHE"))
                .child(fixed_value(
                    75.0,
                    format!(
                        "{} read",
                        super::run_panel::compact_number(usage.aggregate.cache_read)
                    ),
                    THEME.colors.subtle,
                    gpui::TextAlign::Right,
                ))
                .child(metric_separator())
                .child(fixed_value(
                    87.0,
                    format!(
                        "{} write",
                        super::run_panel::compact_number(usage.aggregate.cache_write)
                    ),
                    THEME.colors.subtle,
                    gpui::TextAlign::Right,
                )),
        )
        .into_any_element()
}

fn fixed_footer_cell(width: f32) -> gpui::Div {
    div()
        .w(px(width))
        .min_w(px(width))
        .max_w(px(width))
        .flex_none()
        .flex()
        .flex_col()
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.caption)
        .overflow_hidden()
}

fn footer_label(label: &'static str) -> AnyElement {
    div()
        .font_weight(FontWeight::MEDIUM)
        .text_color(THEME.colors.subtle)
        .child(label)
        .into_any_element()
}

fn fixed_label(width: f32, label: &'static str) -> AnyElement {
    div()
        .w(px(width))
        .min_w(px(width))
        .max_w(px(width))
        .flex_none()
        .font_weight(FontWeight::MEDIUM)
        .text_color(THEME.colors.subtle)
        .child(label)
        .into_any_element()
}

fn fixed_value(
    width: f32,
    value: String,
    color: gpui::Rgba,
    align: gpui::TextAlign,
) -> AnyElement {
    div()
        .w(px(width))
        .min_w(px(width))
        .max_w(px(width))
        .flex_none()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_align(align)
        .text_color(color)
        .child(value)
        .into_any_element()
}

fn metric_separator() -> AnyElement {
    div()
        .w(px(12.0))
        .flex_none()
        .text_align(gpui::TextAlign::Center)
        .text_color(THEME.colors.subtle)
        .child("|")
        .into_any_element()
}

pub(super) fn footer_separator() -> AnyElement {
    div()
        .w(px(SEPARATOR_WIDTH))
        .min_w(px(SEPARATOR_WIDTH))
        .max_w(px(SEPARATOR_WIDTH))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.caption)
        .text_color(THEME.colors.subtle)
        .child("|")
        .into_any_element()
}

fn effort_color(level: &str) -> gpui::Rgba {
    match level.to_ascii_lowercase().as_str() {
        "off" => THEME.colors.subtle,
        "minimal" => THEME.colors.muted,
        "low" => THEME.colors.link,
        "medium" => THEME.colors.accent,
        "high" => THEME.colors.warning,
        "xhigh" | "max" => THEME.colors.error,
        _ => THEME.colors.accent,
    }
}

fn effort_label(level: &str) -> String {
    let mut characters = level.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => "Off".into(),
    }
}

fn bounded_label(value: &str, max: usize) -> String {
    let mut label = value.chars().take(max).collect::<String>();
    if value.chars().count() > max {
        label.push('…');
    }
    label
}
