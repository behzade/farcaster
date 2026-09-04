use gpui::{
    InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, div,
};
use gpui_base::Button as BaseButton;

use super::super::{FarcasterApp, session_rail};
use crate::app::ui::primitives::{ButtonTone, button};
use crate::app::ui::theme::THEME;
use crate::app::{PickerScope, ProjectPickerIntent};

pub(super) fn render_heading(
    project: std::path::PathBuf,
    harness: String,
    entity: gpui::WeakEntity<FarcasterApp>,
) -> impl IntoElement {
    let label = session_rail::project_label(&project);
    let project_entity = entity.clone();
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(THEME.space.sm)
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .text_size(THEME.type_scale.display)
                .text_color(THEME.colors.text)
                .child("What needs doing in ")
                .child(
                    BaseButton::new("draft-project")
                        .accessibility_label(label.clone())
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .text_size(THEME.type_scale.display)
                        .text_color(THEME.colors.accent)
                        .hover(|button| button.text_color(THEME.colors.accent_hover))
                        .active(|button| button.text_color(THEME.colors.accent_active))
                        .focus(|button| button.text_decoration_1())
                        .on_click(move |_, window, cx| {
                            let _ = project_entity.update(cx, |this, cx| {
                                this.open_picker(
                                    PickerScope::Projects(ProjectPickerIntent::ChangeDraft),
                                    window,
                                    cx,
                                );
                            });
                        })
                        .child(label),
                )
                .child("?"),
        )
        .child(
            div().flex().items_center().gap(THEME.space.xs).children(
                crate::agents::backend_statuses()
                    .into_iter()
                    .filter(|backend| backend.available)
                    .map(|backend| {
                        let selected = backend.id == harness;
                        let target = backend.id.clone();
                        let entity = entity.clone();
                        button(
                            format!("draft-harness-{target}"),
                            backend.name,
                            if selected {
                                ButtonTone::Accent
                            } else {
                                ButtonTone::Quiet
                            },
                            !selected,
                            move |window, cx| {
                                let _ = entity.update(cx, |this, cx| {
                                    this.change_draft_harness(target.clone(), window, cx);
                                });
                            },
                        )
                    }),
            ),
        )
}
