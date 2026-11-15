use gpui::{
    App, ElementId, IntoElement as _, ParentElement as _, RenderOnce, SharedString,
    StyleRefinement, Styled, Window, div,
};
use gpui_component::{Disableable as _, button::Button, menu::PopupMenuItem};
use std::rc::Rc;

use super::{AppIconSize, ButtonTone, DropdownMenu as _, app_icon, dropdown_content_button};
use crate::app::ui::{assets::AppIcon, theme::theme};

pub(crate) struct DropdownItem {
    label: SharedString,
    icon: Option<AppIcon>,
    selected: bool,
    disabled: bool,
    on_select: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl DropdownItem {
    pub(crate) fn new(
        label: impl Into<SharedString>,
        on_select: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            icon: None,
            selected: false,
            disabled: false,
            on_select: Rc::new(on_select),
        }
    }
    pub(crate) fn icon(mut self, icon: AppIcon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

#[derive(gpui::IntoElement)]
pub(crate) struct Dropdown {
    trigger: Button,
    items: Vec<DropdownItem>,
    auto_menu_width: bool,
    anchor: gpui::Anchor,
}

impl Dropdown {
    pub(crate) fn with_content(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        content: impl gpui::IntoElement,
        items: Vec<DropdownItem>,
    ) -> Self {
        let trigger =
            dropdown_content_button(id, label, content, ButtonTone::Neutral, !items.is_empty());
        Self {
            trigger,
            items,
            auto_menu_width: false,
            anchor: gpui::Anchor::TopLeft,
        }
    }
    pub(crate) fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        items: Vec<DropdownItem>,
    ) -> Self {
        let icon = items
            .iter()
            .find(|item| item.selected)
            .and_then(|item| item.icon);
        let label = label.into();
        Self::with_content(
            id,
            label.clone(),
            div()
                .flex()
                .items_center()
                .gap(theme().space.xs)
                .when_some(icon, |content, icon| {
                    content.child(app_icon(icon, AppIconSize::Control))
                })
                .child(div().min_w_0().truncate().child(label)),
            items,
        )
    }

    pub(crate) fn tone(mut self, tone: ButtonTone) -> Self {
        self.trigger = super::button::tone_button(self.trigger, tone);
        self
    }
    pub(crate) fn caret(mut self, visible: bool) -> Self {
        self.trigger = self.trigger.dropdown_caret(visible);
        self
    }
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.trigger = self.trigger.disabled(disabled || self.items.is_empty());
        self
    }
    pub(crate) fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.trigger = self.trigger.accessibility_label(label);
        self
    }
    pub(crate) fn anchor(mut self, anchor: gpui::Anchor) -> Self {
        self.anchor = anchor;
        self
    }
    // Backend choices include installation/sign-in details that need more room.
    pub(crate) fn auto_menu_width(mut self) -> Self {
        self.auto_menu_width = true;
        self
    }
}

impl Styled for Dropdown {
    fn style(&mut self) -> &mut StyleRefinement {
        self.trigger.style()
    }
}
impl gpui::InteractiveElement for Dropdown {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.trigger.interactivity()
    }
}

impl RenderOnce for Dropdown {
    fn render(self, _: &mut Window, _: &mut App) -> impl gpui::IntoElement {
        let menu = self
            .trigger
            .dropdown_menu_with_anchor(self.anchor, move |menu, _, _| {
                self.items.iter().fold(
                    menu.check_side(gpui_component::Side::Right)
                        .max_h(theme().size(320.0))
                        .scrollable(true),
                    |menu, item| {
                        let on_select = item.on_select.clone();
                        let disabled = item.disabled;
                        menu.item(
                            PopupMenuItem::new(item.label.clone())
                                .when_some(item.icon, |item, icon| item.icon(icon))
                                .checked(item.selected)
                                .disabled(item.disabled)
                                .on_click(move |_, window, cx| {
                                    // PopupMenu can confirm a disabled first row from the keyboard.
                                    if !disabled {
                                        on_select(window, cx);
                                    }
                                }),
                        )
                    },
                )
            });
        if self.auto_menu_width {
            menu.into_any_element()
        } else {
            menu.match_trigger_width().into_any_element()
        }
    }
}

use gpui::prelude::FluentBuilder as _;

#[cfg(test)]
#[path = "dropdown_tests.rs"]
mod tests;
