use gpui::{AnyView, App, SharedString, StatefulInteractiveElement, Window};
use gpui_component::{
    ElementExt,
    tooltip::{ManagedTooltipExt as _, Tooltip},
};

pub(crate) fn tooltip(
    label: impl Into<SharedString> + 'static,
) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let label = label.into();
    move |window, cx| Tooltip::new(label.clone()).build(window, cx)
}

pub(crate) trait AppTooltip: StatefulInteractiveElement + ElementExt + Sized {
    fn app_tooltip(self, label: impl Into<SharedString> + 'static) -> Self {
        self.managed_tooltip(tooltip(label))
    }
}

impl<E: StatefulInteractiveElement + ElementExt> AppTooltip for E {}
