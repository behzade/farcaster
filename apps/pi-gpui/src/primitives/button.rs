use gpui::{App, ElementId, SharedString, Window};
use gpui_component::{
    Disableable as _, Icon, IconNamed, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
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

pub(crate) fn icon_button(
    id: impl Into<ElementId>,
    icon: impl IconNamed,
    label: impl Into<SharedString>,
    tone: ButtonTone,
    enabled: bool,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    let label = label.into();
    let button = Button::new(id)
        .icon(Icon::new(icon))
        .tooltip(label)
        .with_size(Size::Small)
        .disabled(!enabled)
        .on_click(move |_, window, cx| on_press(window, cx));
    tone_button(button, tone)
}

fn tone_button(button: Button, tone: ButtonTone) -> Button {
    match tone {
        ButtonTone::Accent => button.primary(),
        ButtonTone::Neutral => button.secondary(),
        ButtonTone::Quiet => button.ghost(),
        ButtonTone::Danger => button.danger(),
    }
}
