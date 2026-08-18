use gpui::{
    AnyElement, App, Div, ElementId, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_component::{
    Disableable as _, IconNamed, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
};

use crate::{
    primitives::{AppIconSize, app_icon, icon_control},
    theme::THEME,
};

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

pub(crate) fn button_with_icon(
    id: impl Into<ElementId>,
    icon: impl IconNamed,
    label: impl Into<SharedString>,
    tone: ButtonTone,
    enabled: bool,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    let button = Button::new(id)
        .icon(app_icon(icon, AppIconSize::Inline))
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

pub(crate) fn dropdown_icon_button(
    id: impl Into<ElementId>,
    icon: impl IconNamed,
    label: impl Into<SharedString>,
    tone: ButtonTone,
    enabled: bool,
) -> Button {
    let button = Button::new(id)
        .icon(app_icon(icon, AppIconSize::Control))
        .tooltip(label)
        .dropdown_caret(false)
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
    tone_icon_control(
        icon_control(id, label).child(app_icon(icon, AppIconSize::Control)),
        tone,
    )
    .on_click(move |_, window, cx| on_press(window, cx))
    .into_any_element()
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
