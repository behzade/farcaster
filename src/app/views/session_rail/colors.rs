use gpui::{
    Context, IntoElement, ParentElement as _, Rgba, SharedString, Styled as _, WeakEntity, div,
};
use gpui_component::menu::{PopupMenu, PopupMenuItem};

use crate::{
    app::{FarcasterApp, ui::theme::theme},
    sessions::FOLDER_COLOR_COUNT,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum ColorTarget {
    Folder(u64),
    Session(i64),
}

pub(super) fn palette_color(index: u8) -> Rgba {
    let colors = theme().colors;
    let palette = [
        colors.code,
        colors.skill,
        colors.file,
        colors.warning,
        colors.error,
        colors.success,
        colors.accent,
        colors.muted,
    ];
    palette[usize::from(index) % palette.len()]
}

pub(super) fn color_box(color: Rgba) -> impl IntoElement {
    div()
        .w(theme().size(12.0))
        .h(theme().size(12.0))
        .flex_none()
        .border(theme().border)
        .border_color(theme().colors.border)
        .bg(color)
}

pub(super) fn color_menu(
    menu: PopupMenu,
    current: Option<u8>,
    entity: WeakEntity<FarcasterApp>,
    target: ColorTarget,
) -> PopupMenu {
    let mut menu = menu.label("Colour");
    for index in 0..FOLDER_COLOR_COUNT {
        menu = menu.item(color_swatch(
            u8::try_from(index).unwrap_or(0),
            current,
            entity.clone(),
            target,
        ));
    }
    match target {
        ColorTarget::Folder(_) => menu,
        ColorTarget::Session(_) => menu
            .separator()
            .item(PopupMenuItem::new("No colour").on_click(move |_, _, cx| {
                let _ = entity.update(cx, |app, cx| app.set_rail_color(target, None, cx));
            })),
    }
}

fn color_swatch(
    index: u8,
    current: Option<u8>,
    entity: WeakEntity<FarcasterApp>,
    target: ColorTarget,
) -> PopupMenuItem {
    let color = palette_color(index);
    let label: SharedString = format!("Colour {}", index + 1).into();
    PopupMenuItem::element(move |_, _| {
        div()
            .flex()
            .items_center()
            .gap(theme().space.sm)
            .child(color_box(color))
            .child(label.clone())
    })
    .checked(current == Some(index))
    .disabled(current == Some(index))
    .on_click(move |_, _, cx| {
        let _ = entity.update(cx, |app, cx| app.set_rail_color(target, Some(index), cx));
    })
}

impl FarcasterApp {
    fn set_rail_color(&mut self, target: ColorTarget, color: Option<u8>, cx: &mut Context<Self>) {
        let mut next = self.sessions.folders.clone();
        let changed = match target {
            ColorTarget::Folder(id) => color.is_some_and(|color| next.set_color(id, color)),
            ColorTarget::Session(session) => next.set_session_color(session, color),
        };
        if !changed {
            return;
        }
        self.save_session_folders(next, cx);
    }
}
