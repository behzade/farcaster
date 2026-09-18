use std::rc::Rc;

use gpui::{AnyElement, AnyView, App, SharedString, StatefulInteractiveElement, Window};
use gpui_component::{
    ElementExt, Placement,
    tooltip::{ManagedTooltipExt as _, Tooltip},
};

const PLACEMENT: Placement = Placement::Right;

pub(crate) fn tooltip(
    label: impl Into<SharedString> + 'static,
) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let label = label.into();
    move |window, cx| Tooltip::new(label.clone()).build(window, cx)
}

pub(crate) trait AppTooltip: StatefulInteractiveElement + ElementExt + Sized {
    fn app_tooltip(self, label: impl Into<SharedString> + 'static) -> Self {
        self.managed_tooltip_with_placement(Some(PLACEMENT), tooltip(label))
    }

    fn app_tooltip_element(
        self,
        content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        let content = Rc::new(content);
        self.managed_tooltip_with_placement(Some(PLACEMENT), move |window, cx| {
            let content = Rc::clone(&content);
            Tooltip::element(move |window, cx| content(window, cx)).build(window, cx)
        })
    }
}

impl<E: StatefulInteractiveElement + ElementExt> AppTooltip for E {}
