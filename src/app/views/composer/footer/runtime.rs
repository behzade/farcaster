use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, CursorStyle, ElementId, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Role, StatefulInteractiveElement as _, Styled as _, WeakEntity, Window,
    div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    tooltip::Tooltip,
};

use super::separator;
use crate::app::FarcasterApp;
use crate::{
    app::ui::assets::AppIcon,
    app::ui::primitives::{
        AppIconSize, ButtonTone, activates_button, app_icon, dropdown_content_button,
    },
    app::ui::theme::{MONO_FONT_FAMILY, THEME},
    protocol::{AgentMode, Model},
    runtime::{ConfigurationStatus, HarnessAccessMode},
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let identity = app.snapshot.session_identity();
    let selected_model = identity.model;
    let selected_provider = identity.provider.map(str::to_owned).or_else(|| {
        app.snapshot
            .models
            .first()
            .map(|model| model.provider.clone())
    });
    let catalog_loading = selected_provider.is_none()
        && !app.snapshot.connected
        && app.snapshot.configuration_status == ConfigurationStatus::Loading;
    let provider_label = selected_provider.unwrap_or_else(|| "Provider".into());
    let model_label = selected_model.map_or_else(
        || {
            if catalog_loading {
                "Loading models…".into()
            } else {
                "Model".into()
            }
        },
        |model| model.name.clone(),
    );
    let effort = identity.effort.unwrap_or("off");
    let shows_effort = !app.snapshot.available_thinking_levels().is_empty() || effort != "off";
    let runtime_content = div()
        .flex()
        .items_center()
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.body)
        .child(div().text_color(THEME.colors.muted).child(provider_label))
        .child(runtime_slash())
        .child(div().text_color(THEME.colors.text).child(model_label))
        .when(shows_effort, |content| {
            content.child(runtime_slash()).child(
                div()
                    .text_color(effort_color(effort))
                    .child(effort_label(effort)),
            )
        });
    let runtime_entity = entity.clone();
    let runtime = dropdown_content_button(
        "select-runtime",
        "Runtime",
        runtime_content,
        ButtonTone::Neutral,
        true,
    )
    .flex_none()
    .dropdown_menu_with_anchor(gpui::Anchor::BottomLeft, move |menu, window, cx| {
        build_runtime_menu(menu, &runtime_entity, window, cx)
    });

    div()
        .flex_none()
        .flex()
        .items_center()
        .child(runtime)
        .child(separator())
        .child(access_selector(
            app.snapshot.access_mode,
            &app.snapshot.harness,
            entity,
        ))
        .into_any_element()
}

fn build_runtime_menu(
    menu: PopupMenu,
    entity: &WeakEntity<FarcasterApp>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let Some(app) = entity.upgrade() else {
        return menu;
    };
    let data = {
        let app = app.read(cx);
        let identity = app.snapshot.session_identity();
        RuntimeMenuData {
            models: app.snapshot.models.clone(),
            catalog_levels: app.snapshot.thinking_levels.clone(),
            selected_model: identity.model.cloned(),
            selected_effort: identity.effort.map(str::to_owned),
            modes: app.snapshot.modes.clone(),
            selected_mode: app.snapshot.selected_mode.clone(),
            feedback: catalog_feedback(app),
        }
    };
    if let Some(message) = data.feedback {
        return menu
            .min_w(px(220.0))
            .item(PopupMenuItem::new(message).disabled(true));
    }

    let groups = runtime_menu_groups(&data.models, &data.catalog_levels);
    let reveal = runtime_reveal_path(
        &groups,
        data.selected_model.as_ref(),
        data.selected_effort.as_deref(),
    );
    let mut menu = menu.min_w(px(160.0));
    for (provider_ix, group) in groups.into_iter().enumerate() {
        let entity = entity.clone();
        let selected_model = data.selected_model.clone();
        let selected_effort = data.selected_effort.clone();
        let reveal = reveal.filter(|reveal| reveal.provider == provider_ix);
        menu = menu.submenu(group.provider, window, cx, move |menu, window, cx| {
            build_model_menu(
                menu,
                &entity,
                &group.models,
                selected_model.as_ref(),
                selected_effort.as_deref(),
                reveal,
                window,
                cx,
            )
        });
    }
    if !data.modes.is_empty() {
        let entity = entity.clone();
        let modes = data.modes;
        let selected_mode = data.selected_mode;
        menu = menu
            .separator()
            .submenu("Mode", window, cx, move |menu, _, _| {
                let mut menu = menu.min_w(px(140.0));
                for mode in &modes {
                    let entity = entity.clone();
                    let target = mode.id.clone();
                    let checked = selected_mode.as_deref() == Some(mode.id.as_str());
                    menu = menu.item(
                        PopupMenuItem::new(mode.name.clone())
                            .checked(checked)
                            .on_click(move |_, _, cx| {
                                let _ = entity.update(cx, |this, cx| {
                                    this.set_agent_mode(target.clone(), cx);
                                });
                            }),
                    );
                }
                menu
            });
    }
    if let Some(reveal) = reveal {
        menu = menu.with_selected_index(reveal.provider);
    }
    menu
}

#[allow(clippy::too_many_arguments)]
fn build_model_menu(
    menu: PopupMenu,
    entity: &WeakEntity<FarcasterApp>,
    models: &[RuntimeMenuModel],
    selected_model: Option<&Model>,
    selected_effort: Option<&str>,
    reveal: Option<RuntimeReveal>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let mut menu = menu.min_w(px(200.0));
    for (model_ix, entry) in models.iter().enumerate() {
        let selected = selected_model.is_some_and(|selected| {
            selected.id == entry.model.id && selected.provider == entry.model.provider
        });
        if entry.efforts.is_empty() {
            let model = entry.model.clone();
            let entity = entity.clone();
            menu = menu.item(
                PopupMenuItem::new(entry.model.name.clone())
                    .checked(selected)
                    .on_click(move |_, _, cx| apply_runtime(&entity, &model, None, cx)),
            );
            continue;
        }

        let model = entry.model.clone();
        let entity = entity.clone();
        let efforts = entry.efforts.clone();
        let selected_effort = selected_effort.map(str::to_owned);
        let effort_ix = reveal
            .filter(|reveal| reveal.model == model_ix)
            .and_then(|reveal| reveal.effort);
        menu = menu.submenu(entry.model.name.clone(), window, cx, move |menu, _, _| {
            let mut menu = menu.min_w(px(120.0));
            for effort in &efforts {
                let entity = entity.clone();
                let model = model.clone();
                let target = effort.clone();
                let checked = selected && selected_effort.as_deref() == Some(effort.as_str());
                menu = menu.item(
                    PopupMenuItem::new(effort_label(effort))
                        .checked(checked)
                        .on_click(move |_, _, cx| {
                            apply_runtime(&entity, &model, Some(target.clone()), cx);
                        }),
                );
            }
            if let Some(effort_ix) = effort_ix {
                menu = menu.with_selected_index(effort_ix);
            }
            menu
        });
    }
    if let Some(reveal) = reveal {
        menu = menu.with_selected_index(reveal.model);
    }
    menu
}

fn apply_runtime(
    entity: &WeakEntity<FarcasterApp>,
    model: &Model,
    effort: Option<String>,
    cx: &mut App,
) {
    let _ = entity.update(cx, |this, cx| {
        let same_model = this
            .snapshot
            .session_identity()
            .model
            .is_some_and(|selected| selected.id == model.id && selected.provider == model.provider);
        if !same_model {
            this.select_model(model, cx);
        }
        if let Some(effort) = effort {
            this.set_thinking_level(effort, cx);
        }
    });
}

fn catalog_feedback(app: &FarcasterApp) -> Option<String> {
    if !app.snapshot.models.is_empty() {
        return None;
    }
    Some(match &app.snapshot.configuration_status {
        ConfigurationStatus::Loading if !app.snapshot.connected => "Refreshing models…".to_owned(),
        ConfigurationStatus::Failed(error) => format!("Models unavailable: {error}"),
        ConfigurationStatus::Loading | ConfigurationStatus::Loaded => {
            "No models were advertised by this harness.".to_owned()
        }
    })
}

struct RuntimeMenuData {
    models: Vec<Model>,
    catalog_levels: Vec<String>,
    selected_model: Option<Model>,
    selected_effort: Option<String>,
    modes: Vec<AgentMode>,
    selected_mode: Option<String>,
    feedback: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RuntimeReveal {
    provider: usize,
    model: usize,
    effort: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RuntimeMenuGroup {
    provider: String,
    models: Vec<RuntimeMenuModel>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RuntimeMenuModel {
    model: Model,
    efforts: Vec<String>,
}

fn runtime_menu_groups(models: &[Model], catalog_levels: &[String]) -> Vec<RuntimeMenuGroup> {
    let mut providers = models
        .iter()
        .map(|model| model.provider.clone())
        .collect::<Vec<_>>();
    providers.sort();
    providers.dedup();
    providers
        .into_iter()
        .map(|provider| RuntimeMenuGroup {
            models: models
                .iter()
                .filter(|model| model.provider == provider)
                .map(|model| RuntimeMenuModel {
                    efforts: efforts_for_model(model, catalog_levels).to_vec(),
                    model: model.clone(),
                })
                .collect(),
            provider,
        })
        .collect()
}

fn runtime_reveal_path(
    groups: &[RuntimeMenuGroup],
    selected_model: Option<&Model>,
    selected_effort: Option<&str>,
) -> Option<RuntimeReveal> {
    let selected = selected_model?;
    let provider = groups
        .iter()
        .position(|group| group.provider == selected.provider)?;
    let model = groups[provider]
        .models
        .iter()
        .position(|entry| entry.model.id == selected.id)?;
    let effort = selected_effort.and_then(|effort| {
        groups[provider].models[model]
            .efforts
            .iter()
            .position(|candidate| candidate == effort)
    });
    Some(RuntimeReveal {
        provider,
        model,
        effort,
    })
}

fn efforts_for_model<'a>(model: &'a Model, catalog_levels: &'a [String]) -> &'a [String] {
    if !model.reasoning {
        return &[];
    }
    match model.efforts.as_deref() {
        Some(efforts) => efforts,
        None => catalog_levels,
    }
}

fn runtime_slash() -> AnyElement {
    div()
        .px(px(6.0))
        .text_color(THEME.colors.subtle)
        .child("/")
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

fn access_selector(
    selected: HarnessAccessMode,
    harness: &str,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let supported = crate::agents::supported_access_modes(harness);
    let selected = crate::agents::normalize_access_mode(harness, selected);
    let next = next_access_mode(selected, supported);
    let label = cycle_label(
        "Access",
        access_mode_label(selected),
        access_mode_label(next),
    );

    div()
        .id("harness-access")
        .flex_none()
        .child(access_cycle_button(
            "cycle-harness-access",
            AppIcon::Folder,
            access_mode_color(selected),
            label,
            move |cx| {
                let _ = entity.update(cx, |this, cx| this.set_access_mode(next, cx));
            },
        ))
        .into_any_element()
}

fn access_cycle_button(
    id: impl Into<ElementId>,
    resource: AppIcon,
    color: gpui::Rgba,
    label: String,
    on_press: impl Fn(&mut App) + 'static,
) -> AnyElement {
    let on_press = Rc::new(on_press);
    let click = Rc::clone(&on_press);
    let tooltip = label.clone();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .tab_index(0)
        .flex()
        .items_center()
        .gap(px(5.0))
        .px(px(1.0))
        .py(px(2.0))
        .rounded(px(3.0))
        .cursor(CursorStyle::PointingHand)
        .text_color(THEME.colors.muted)
        .hover(|button| {
            button
                .bg(THEME.colors.surface)
                .text_color(THEME.colors.text)
        })
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            click(cx);
        })
        .on_key_down(move |event, _, cx| {
            if activates_button(event) {
                cx.stop_propagation();
                on_press(cx);
            }
        })
        .child(app_icon(resource, AppIconSize::Inline))
        .child(div().size(px(7.0)).rounded_full().bg(color))
        .into_any_element()
}

fn cycle_label(capability: &str, current: &str, next: &str) -> String {
    format!("{capability}: {current}. Click to change to {next}")
}

fn next_access_mode(
    selected: HarnessAccessMode,
    supported: &[HarnessAccessMode],
) -> HarnessAccessMode {
    let Some(index) = supported.iter().position(|mode| *mode == selected) else {
        return supported
            .first()
            .copied()
            .unwrap_or(HarnessAccessMode::Full);
    };
    supported
        .get((index + 1) % supported.len().max(1))
        .copied()
        .unwrap_or(HarnessAccessMode::Full)
}

const fn access_mode_label(mode: HarnessAccessMode) -> &'static str {
    match mode {
        HarnessAccessMode::Full => "Full access",
        HarnessAccessMode::Sandboxed => "Sandboxed",
        HarnessAccessMode::Auto => "Auto",
    }
}

fn access_mode_color(mode: HarnessAccessMode) -> gpui::Rgba {
    match mode {
        HarnessAccessMode::Sandboxed => THEME.colors.warning,
        HarnessAccessMode::Auto => THEME.colors.accent,
        HarnessAccessMode::Full => THEME.colors.error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(
        provider: &str,
        id: &str,
        name: &str,
        reasoning: bool,
        efforts: Option<&[&str]>,
    ) -> Model {
        Model {
            id: id.into(),
            name: name.into(),
            provider: provider.into(),
            context_window: 0,
            reasoning,
            efforts: efforts.map(|efforts| efforts.iter().map(|effort| (*effort).into()).collect()),
        }
    }

    #[test]
    fn runtime_menu_nests_effort_under_models_that_reason() {
        let groups = runtime_menu_groups(
            &[
                model(
                    "openai-codex",
                    "sol",
                    "GPT-5.6 Sol",
                    true,
                    Some(&["low", "medium"]),
                ),
                model("openai-codex", "mini", "GPT-5.4 mini", false, None),
                model("anthropic", "opus", "Opus", true, None),
            ],
            &["off".into(), "high".into()],
        );

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].provider, "anthropic");
        assert_eq!(groups[0].models[0].efforts, ["off", "high"]);
        assert_eq!(groups[1].provider, "openai-codex");
        assert_eq!(groups[1].models[0].efforts, ["low", "medium"]);
        assert!(groups[1].models[1].efforts.is_empty());
    }

    #[test]
    fn runtime_menu_reveals_the_selected_provider_model_and_effort() {
        let groups = runtime_menu_groups(
            &[
                model("anthropic", "opus", "Opus", true, None),
                model(
                    "openai-codex",
                    "sol",
                    "GPT-5.6 Sol",
                    true,
                    Some(&["low", "medium", "high"]),
                ),
                model("openai-codex", "mini", "GPT-5.4 mini", false, None),
            ],
            &[],
        );
        let selected = model(
            "openai-codex",
            "sol",
            "GPT-5.6 Sol",
            true,
            Some(&["low", "medium", "high"]),
        );

        assert_eq!(
            runtime_reveal_path(&groups, Some(&selected), Some("medium")),
            Some(RuntimeReveal {
                provider: 1,
                model: 0,
                effort: Some(1),
            })
        );
        assert_eq!(
            runtime_reveal_path(&groups, Some(&selected), Some("missing")),
            Some(RuntimeReveal {
                provider: 1,
                model: 0,
                effort: None,
            })
        );
        assert_eq!(runtime_reveal_path(&groups, None, Some("medium")), None);
    }

    #[test]
    fn access_controls_cycle_only_supported_modes() {
        use HarnessAccessMode::{Auto, Full, Sandboxed};
        assert_eq!(next_access_mode(Sandboxed, &[Sandboxed, Full]), Full);
        assert_eq!(next_access_mode(Full, &[Sandboxed, Full]), Sandboxed);
        assert_eq!(next_access_mode(Sandboxed, &[Sandboxed, Auto, Full]), Auto);
        assert_eq!(next_access_mode(Sandboxed, &[Auto, Full]), Auto);
    }
}
