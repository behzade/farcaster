use std::rc::Rc;

use gpui::{
    AnyElement, App, Div, ElementId, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    Window,
};
use gpui_component::{
    Disableable as _, IconNamed, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
};

use crate::{
    primitives::{AppIconSize, app_icon, icon_control},
    theme::THEME,
};

type ButtonPress = dyn Fn(&mut Window, &mut App);

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
    let button = Button::new(id)
        .label(label)
        .with_size(Size::Small)
        .disabled(!enabled)
        .on_click(move |_, window, cx| on_press(window, cx));
    tone_button(button, tone)
}

pub(crate) fn dropdown_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    tone: ButtonTone,
    enabled: bool,
) -> Button {
    let button = Button::new(id)
        .label(label)
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
        .tooltip(accessible_label)
        .child(content)
        .dropdown_caret(true)
        .with_size(Size::Small)
        .disabled(!enabled);
    tone_button(button, tone)
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

fn icon_button_with_size(
    id: impl Into<ElementId>,
    icon: impl IconNamed,
    label: impl Into<SharedString>,
    size: AppIconSize,
    tone: ButtonTone,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let on_press: Rc<ButtonPress> = Rc::new(on_press);
    let click = Rc::clone(&on_press);
    tone_icon_control(icon_control(id, label).child(app_icon(icon, size)), tone)
        .on_click(move |_, window, cx| click(window, cx))
        .on_key_down(move |event, window, cx| {
            if activates_button(event) {
                cx.stop_propagation();
                on_press(window, cx);
            }
        })
}

pub(crate) fn activates_button(event: &KeyDownEvent) -> bool {
    !event.is_held
        && event.keystroke.modifiers == gpui::Modifiers::default()
        && matches!(event.keystroke.key.as_str(), "enter" | "space")
}

fn tone_icon_control(control: Stateful<Div>, tone: ButtonTone) -> Stateful<Div> {
    match tone {
        ButtonTone::Accent => control
            .bg(THEME.colors.accent)
            .text_color(THEME.colors.canvas)
            .hover(|control| control.bg(THEME.colors.accent_hover))
            .active(|control| control.bg(THEME.colors.accent_active)),
        ButtonTone::Neutral => control
            .bg(THEME.colors.surface)
            .text_color(THEME.colors.text)
            .hover(|control| control.bg(THEME.colors.hover))
            .active(|control| control.bg(THEME.colors.hover)),
        ButtonTone::Quiet => control
            .text_color(THEME.colors.muted)
            .hover(|control| control.bg(THEME.colors.hover))
            .active(|control| control.bg(THEME.colors.hover)),
        ButtonTone::Danger => control
            .bg(THEME.colors.error)
            .text_color(THEME.colors.canvas),
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
mod tests {
    use super::*;

    fn key_event(key: &str, is_held: bool) -> KeyDownEvent {
        KeyDownEvent {
            keystroke: gpui::Keystroke::parse(key).expect("test keystroke"),
            is_held,
            prefer_character_input: false,
        }
    }

    #[test]
    fn icon_buttons_activate_on_unmodified_enter_or_space_once() {
        assert!(activates_button(&key_event("enter", false)));
        assert!(activates_button(&key_event("space", false)));
        assert!(!activates_button(&key_event("enter", true)));
        assert!(!activates_button(&key_event("cmd-enter", false)));
        assert!(!activates_button(&key_event("a", false)));
    }
}
