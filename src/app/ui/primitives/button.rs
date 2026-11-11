use crate::app::ui::primitives::AppTooltip as _;
use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, App, Div, ElementId, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    Window, div,
};
use gpui_component::{
    Disableable as _, IconNamed, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
};

use crate::{
    app::ui::primitives::{AppIconSize, app_icon, icon_control},
    app::ui::theme::theme,
};

type ButtonPress = dyn Fn(&mut Window, &mut App);

// Component buttons already have stable ids and retain this interactivity
// when rendering. Adapt that same node, without adding a layout/AX wrapper.
pub(super) struct Accessible<T>(pub T);

impl<T: gpui::InteractiveElement> gpui::InteractiveElement for Accessible<T> {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.0.interactivity()
    }
}
impl<T: gpui::InteractiveElement> gpui::StatefulInteractiveElement for Accessible<T> {}

pub(super) fn on_accessible_click<T: gpui::InteractiveElement>(
    control: T,
    press: impl Fn(&mut Window, &mut App) + 'static,
) -> T {
    // Read the rendered node, so a later `.disabled(...)` or loading state
    // also blocks an AX press. No action is allowed before its first paint.
    let enabled = Rc::new(Cell::new(false));
    let rendered = enabled.clone();
    Accessible(control)
        .a11y_synthetic_children(move |tree| {
            rendered.set(!tree.parent_node().is_disabled());
        })
        .on_a11y_action(gpui::AccessibleAction::Click, move |_, window, cx| {
            if enabled.get() {
                activate_accessible_window(window, cx);
                press(window, cx);
                crate::app::ui::accessibility::refresh_hidden_accessibility(window, cx);
            }
        })
        .0
}

pub(crate) trait ButtonPressExt {
    fn on_press(self, enabled: bool, on_press: impl Fn(&mut Window, &mut App) + 'static) -> Button;
}

impl ButtonPressExt for Button {
    fn on_press(self, enabled: bool, on_press: impl Fn(&mut Window, &mut App) + 'static) -> Button {
        on_press_button(self, enabled, on_press)
    }
}

pub(crate) fn on_press_button(
    button: Button,
    enabled: bool,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    let on_press: Rc<ButtonPress> = Rc::new(on_press);
    let accessible_press = on_press.clone();
    on_accessible_click(button, move |window, cx| {
        if enabled {
            accessible_press(window, cx);
        }
    })
    .on_click(move |_, window, cx| {
        if enabled {
            on_press(window, cx);
        }
    })
}

pub(super) fn activate_accessible_window(window: &mut Window, cx: &mut App) {
    // macOS queues window activation. Re-activating an already-key owner can
    // steal focus back from the panel this action is about to open.
    if !window.is_window_active() {
        cx.activate(true);
        window.activate_window();
    }
}

pub(crate) fn preserve_pointer_focus(_: &gpui::MouseDownEvent, window: &mut Window, cx: &mut App) {
    window.prevent_default();
    gpui_base::GlobalState::suppress_text_selection(cx);
}

#[derive(Clone, Copy)]
pub(crate) enum ButtonTone {
    Accent,
    Neutral,
    Quiet,
    Danger,
}

pub(crate) fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    tone: ButtonTone,
    enabled: bool,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    let label = label.into();
    let button = on_press_button(
        Button::new(id)
            .accessibility_label(label.clone())
            .child(button_content(label))
            .with_size(Size::Small)
            .disabled(!enabled),
        enabled,
        on_press,
    );
    tone_button(button, tone)
}

pub(crate) fn dropdown_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    tone: ButtonTone,
    enabled: bool,
) -> Button {
    let label = label.into();
    let button = Button::new(id)
        .accessibility_label(label.clone())
        .child(button_content(label))
        .dropdown_caret(true)
        .with_size(Size::Small)
        .disabled(!enabled);
    tone_button(button, tone)
}

pub(crate) fn dropdown_content_button(
    id: impl Into<ElementId>,
    accessible_label: impl Into<SharedString>,
    content: impl IntoElement,
    tone: ButtonTone,
    enabled: bool,
) -> Button {
    let accessible_label = accessible_label.into();
    let button = Button::new(id)
        .accessibility_label(accessible_label.clone())
        .app_tooltip(accessible_label)
        .child(button_content(content))
        .dropdown_caret(true)
        .with_size(Size::Small)
        .disabled(!enabled);
    tone_button(button, tone)
}

fn button_content(content: impl IntoElement) -> Div {
    div()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(theme().type_scale.body)
        .child(content)
}

pub(crate) fn icon_button(
    id: impl Into<ElementId>,
    icon: impl IconNamed,
    label: impl Into<SharedString>,
    tone: ButtonTone,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    icon_button_with_size(id, icon, label, AppIconSize::Control, tone, on_press).into_any_element()
}

pub(crate) fn prominent_icon_button(
    id: impl Into<ElementId>,
    icon: impl IconNamed,
    label: impl Into<SharedString>,
    tone: ButtonTone,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    icon_button_with_size(id, icon, label, AppIconSize::Prominent, tone, on_press)
}

pub(crate) fn icon_button_with_size(
    id: impl Into<ElementId>,
    icon: impl IconNamed,
    label: impl Into<SharedString>,
    size: AppIconSize,
    tone: ButtonTone,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let on_press: Rc<ButtonPress> = Rc::new(on_press);
    let click = Rc::clone(&on_press);
    on_press_control(
        tone_icon_control(icon_control(id, label).child(app_icon(icon, size)), tone),
        move |window, cx| click(window, cx),
    )
    .on_key_down(move |event, window, cx| {
        if activates_button(event) {
            cx.stop_propagation();
            on_press(window, cx);
        }
    })
}

/// Native accessibility presses must not depend on pointer position or hit testing.
pub(crate) fn on_press_control(
    control: Stateful<Div>,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let on_press: Rc<ButtonPress> = Rc::new(on_press);
    let accessible_press = Rc::clone(&on_press);
    control
        .on_a11y_action(gpui::AccessibleAction::Click, move |_, window, cx| {
            // AX presses skip AppKit's mouse activation. Native modal panels
            // need their owner active before the requested action opens them.
            activate_accessible_window(window, cx);
            accessible_press(window, cx);
            crate::app::ui::accessibility::refresh_hidden_accessibility(window, cx);
        })
        .on_click(move |_, window, cx| on_press(window, cx))
}

pub(crate) fn activates_button(event: &KeyDownEvent) -> bool {
    !event.is_held
        && event.keystroke.modifiers == gpui::Modifiers::default()
        && matches!(event.keystroke.key.as_str(), "enter" | "space")
}

fn tone_icon_control(control: Stateful<Div>, tone: ButtonTone) -> Stateful<Div> {
    match tone {
        ButtonTone::Accent => control
            .bg(theme().colors.accent)
            .text_color(theme().colors.canvas)
            .hover(|control| control.bg(theme().colors.accent_hover))
            .active(|control| control.bg(theme().colors.accent_active)),
        ButtonTone::Neutral => control
            .bg(theme().colors.surface)
            .text_color(theme().colors.text)
            .hover(|control| control.bg(theme().colors.highlight))
            .active(|control| control.bg(theme().colors.highlight)),
        ButtonTone::Quiet => control
            .text_color(theme().colors.muted)
            .hover(|control| control.bg(theme().colors.highlight))
            .active(|control| control.bg(theme().colors.highlight)),
        ButtonTone::Danger => control
            .bg(theme().colors.danger)
            .text_color(theme().colors.canvas),
    }
}

fn tone_button(button: Button, tone: ButtonTone) -> Button {
    match tone {
        ButtonTone::Accent => button.primary(),
        ButtonTone::Neutral => button.secondary(),
        ButtonTone::Quiet => button.ghost(),
        ButtonTone::Danger => button.danger(),
    }
}

#[cfg(test)]
#[path = "button_tests.rs"]
mod tests;
