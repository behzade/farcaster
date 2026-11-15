use crate::app::ui::primitives::Popover;
use gpui::{
    Animation, AnimationExt as _, AnyElement, Div, FontWeight, InteractiveElement as _,
    IntoElement as _, ParentElement as _, Styled as _, Transformation, WeakEntity, div, percentage,
    prelude::FluentBuilder as _,
};

use crate::app::FarcasterApp;
use crate::{
    agents::SandboxState,
    app::ui::assets::AppIcon,
    app::ui::primitives::{
        AppIconSize, ButtonTone, Dropdown, DropdownItem, app_icon, dropdown_content_button,
    },
    app::ui::theme::theme,
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
    let effort = identity.effort.unwrap_or("default");
    let shows_effort = !app.snapshot.available_thinking_levels().is_empty()
        || identity.effort.is_some_and(|effort| effort != "off");
    let runtime_content = setting_content()
        .debug_selector(|| "composer-runtime-content".into())
        .child(div().text_color(theme().colors.muted).child(provider_label))
        .child(model_label)
        .when(shows_effort, |content| content.child(effort_label(effort)));
    let runtime_entity = entity.clone();
    let content_entity = entity.clone();
    let runtime = Popover::new("runtime-popover")
        .anchor(gpui::Anchor::BottomLeft)
        .appearance(false)
        .open(app.workspace.runtime_picker.open)
        .trigger(
            dropdown_content_button(
                "select-runtime",
                "Runtime",
                runtime_content,
                ButtonTone::Quiet,
                true,
            )
            .flex_none(),
        )
        .on_open_change(move |open, window, cx| {
            let _ =
                runtime_entity.update(cx, |app, cx| app.set_runtime_picker_open(*open, window, cx));
        })
        .content(move |_, window, cx| {
            content_entity
                .update(cx, |app, cx| app.render_runtime_picker(window, cx))
                .unwrap_or_else(|_| div().into_any_element())
        });

    div()
        .debug_selector(|| "composer-runtime-controls".into())
        .flex_none()
        .flex()
        .items_center()
        .gap(theme().space.md)
        .child(runtime)
        .when(app.snapshot.sandbox_controls_available(), |content| {
            content.child(access_selector(
                app.snapshot.access_mode,
                app.snapshot.available_access_modes(),
                app.snapshot.sandbox_state,
                entity,
            ))
        })
        .into_any_element()
}

fn effort_label(level: &str) -> String {
    let mut characters = level.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => "Off".into(),
    }
}

fn setting_content() -> Div {
    div()
        .flex()
        .items_center()
        .gap(theme().space.sm)
        .text_size(theme().type_scale.body)
        .line_height(theme().type_scale.line_body)
        .font_weight(FontWeight::NORMAL)
        .text_color(theme().colors.text)
}

fn access_selector(
    selected: HarnessAccessMode,
    supported: Vec<HarnessAccessMode>,
    state: SandboxState,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let label = sandbox_state_label(state, selected);
    let content = setting_content()
        .text_color(match state {
            SandboxState::Active(mode) => access_mode_color(mode),
            SandboxState::Pending(_) | SandboxState::Checking => access_mode_color(selected),
            _ => theme().colors.muted,
        })
        .child(app_icon(access_mode_icon(selected), AppIconSize::Control))
        .child(label.strip_prefix("Sandbox: ").unwrap_or(label))
        .when(matches!(state, SandboxState::Pending(_)), |this| {
            this.child(app_icon(AppIcon::Hourglass, AppIconSize::Control))
        })
        .when(state == SandboxState::Checking, |this| {
            this.child(
                app_icon(AppIcon::SpinnerGap, AppIconSize::Control).with_animation(
                    "sandbox-applying",
                    Animation::new(std::time::Duration::from_millis(800)).repeat(),
                    |icon, delta| icon.transform(Transformation::rotate(percentage(delta))),
                ),
            )
        });
    Dropdown::with_content(
        "harness-access",
        sandbox_state_tooltip(state, selected),
        content,
        supported
            .iter()
            .map(|&target| {
                let entity = entity.clone();
                DropdownItem::new(access_mode_label(target), move |_, cx| {
                    let _ = entity.update(cx, |this, cx| this.set_access_mode(target, cx));
                })
                .icon(access_mode_icon(target))
                .selected(target == selected)
            })
            .collect(),
    )
    .tone(ButtonTone::Quiet)
    .anchor(gpui::Anchor::BottomLeft)
    .accessibility_label(sandbox_state_tooltip(state, selected))
    .disabled(supported.len() < 2)
    .into_any_element()
}

fn sandbox_state_label(state: SandboxState, selected: HarnessAccessMode) -> &'static str {
    match state {
        SandboxState::Unmanaged => "Sandbox: Not managed",
        SandboxState::Pending(_) | SandboxState::Checking => access_mode_label(selected),
        SandboxState::Failed => "Sandbox: Unavailable",
        SandboxState::Active(mode) => access_mode_label(mode),
    }
}

fn sandbox_state_tooltip(state: SandboxState, selected: HarnessAccessMode) -> String {
    match state {
        SandboxState::Pending(current) => format!(
            "Currently {}. Waiting for the session to become idle to apply {}.",
            access_mode_label(current),
            access_mode_label(selected),
        ),
        SandboxState::Checking => {
            format!("Applying {}…", access_mode_label(selected))
        }
        _ => format!("Sandbox settings: {}", sandbox_state_label(state, selected)),
    }
}

const fn access_mode_icon(mode: HarnessAccessMode) -> AppIcon {
    match mode {
        HarnessAccessMode::Sandboxed => AppIcon::Shield,
        HarnessAccessMode::Auto => AppIcon::GearSix,
        HarnessAccessMode::Full => AppIcon::WarningCircle,
    }
}

const fn access_mode_label(mode: HarnessAccessMode) -> &'static str {
    match mode {
        HarnessAccessMode::Full => "Sandbox: Off",
        HarnessAccessMode::Sandboxed => "Sandbox: On",
        HarnessAccessMode::Auto => "Sandbox: Auto",
    }
}

fn access_mode_color(mode: HarnessAccessMode) -> gpui::Rgba {
    match mode {
        HarnessAccessMode::Sandboxed | HarnessAccessMode::Auto => theme().colors.text,
        HarnessAccessMode::Full => theme().colors.warning,
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
