use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ObjectFit, ParentElement as _,
    Styled as _, StyledImage as _, WeakEntity, div, img, prelude::FluentBuilder as _,
};

use super::{super::FarcasterApp, draft};
use crate::app::{
    AppSurface,
    ui::{
        layout::{LayoutMode, composer_bottom_clearance, shows_left_inline, shows_right_inline},
        theme::theme,
    },
};

impl FarcasterApp {
    pub(super) fn render_chat_main(
        &self,
        entity: WeakEntity<Self>,
        viewport_height: gpui::Pixels,
    ) -> AnyElement {
        let has_conversation = !self.selected_draft_is_empty_and_unsubmitted();
        let editable_draft_project = (!has_conversation)
            .then(|| self.editable_draft_project())
            .flatten();

        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(if has_conversation {
                div()
                    .id("chat-body")
                    .flex_1()
                    .min_h_0()
                    .child(self.views.transcript.clone())
                    .into_any_element()
            } else {
                draft::render_body(
                    self.views.composer.clone(),
                    editable_draft_project.map(|project| {
                        draft::render_heading(project, entity.clone()).into_any_element()
                    }),
                    self.composer.focus.clone(),
                    viewport_height,
                )
                .into_any_element()
            })
            .when(has_conversation, |main| {
                main.child(
                    div()
                        .w_full()
                        .max_w(theme().layout.conversation_width)
                        .mx_auto()
                        .px(theme().space.md)
                        .pt(theme().space.sm)
                        .pb(composer_bottom_clearance(viewport_height))
                        .flex_none()
                        .child(self.views.composer.clone()),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_workspace_main(
        &self,
        entity: WeakEntity<Self>,
        mode: LayoutMode,
        viewport_height: gpui::Pixels,
        request_focused: bool,
    ) -> AnyElement {
        let native_surface = matches!(
            self.workspace.surface,
            AppSurface::Editor | AppSurface::Terminal
        );
        let native_surface_covered = native_surface
            && self.workspace.native_surface_covered
            && self.native_workspace_covered_by_overlay();
        let main = if native_surface_covered {
            div()
                .size_full()
                .min_h_0()
                .when_some(
                    self.workspace.native_surface_snapshot.clone(),
                    |surface, snapshot| {
                        surface.child(img(snapshot).size_full().object_fit(ObjectFit::Fill))
                    },
                )
                .into_any_element()
        } else {
            match self.workspace.surface {
                AppSurface::Editor
                    if self.workspace.editor.view.is_some()
                        || self.workspace.editor.terminal_editor_view.is_some() =>
                {
                    self.render_editor_surface()
                }
                AppSurface::Terminal if self.workspace.terminal.view.is_some() => {
                    self.render_terminal_workspace()
                }
                _ => self.render_chat_main(entity.clone(), viewport_height),
            }
        };

        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(self.render_workspace_bar(entity.clone(), mode))
            .child(div().relative().flex_1().min_h_0().child(main).when(
                native_surface && self.extensions.active.dialog.is_some(),
                |center| {
                    center.child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .child(self.render_composer_request(entity, request_focused)),
                    )
                },
            ))
            .into_any_element()
    }

    pub(super) fn render_inline_shell(
        &self,
        entity: WeakEntity<Self>,
        mode: LayoutMode,
        main: AnyElement,
        session_rail_width: gpui::Pixels,
        run_panel_width: gpui::Pixels,
    ) -> AnyElement {
        div()
            .size_full()
            .flex()
            .when(shows_left_inline(mode), |shell| {
                let resize = entity.clone();
                shell.child(
                    div()
                        .relative()
                        .w(session_rail_width)
                        .min_w(theme().layout.session_rail_min)
                        .max_w(theme().layout.session_rail_max)
                        .flex_none()
                        .border_r(theme().border)
                        .border_color(theme().colors.border)
                        .child(
                            self.views
                                .session_rail
                                .clone()
                                .cached(gpui::StyleRefinement::default().size_full()),
                        )
                        .child(resize_handle("session-rail-resize", true, move |x, cx| {
                            let _ = resize.update(cx, |this, cx| {
                                this.begin_session_rail_resize(x, cx);
                            });
                        })),
                )
            })
            .child(main)
            .when(shows_right_inline(mode), |shell| {
                let resize = entity;
                shell.child(
                    div()
                        .relative()
                        .w(run_panel_width)
                        .min_w(theme().layout.run_panel_min)
                        .max_w(theme().layout.run_panel_max)
                        .flex_none()
                        .border_l(theme().border)
                        .border_color(theme().colors.border)
                        .child(
                            if self.views.workgraph_inspector_issue.is_some()
                                && self.visible_review().is_none()
                            {
                                self.views.workgraph_detail.clone().into_any_element()
                            } else {
                                self.views
                                    .run_panel
                                    .clone()
                                    .cached(gpui::StyleRefinement::default().size_full())
                                    .into_any_element()
                            },
                        )
                        .child(resize_handle("run-panel-resize", false, move |x, cx| {
                            let _ = resize.update(cx, |this, cx| {
                                this.begin_run_panel_resize(x, cx);
                            });
                        })),
                )
            })
            .into_any_element()
    }
}

fn resize_handle(
    id: &'static str,
    right: bool,
    on_begin: impl Fn(gpui::Pixels, &mut gpui::App) + 'static,
) -> impl gpui::IntoElement {
    div()
        .id(id)
        .absolute()
        .top_0()
        .bottom_0()
        .when(right, |handle| handle.right(gpui::px(-4.0)))
        .when(!right, |handle| handle.left(gpui::px(-4.0)))
        .w(theme().size(7.0))
        .cursor_col_resize()
        .group(id)
        .on_mouse_down(gpui::MouseButton::Left, move |event, _, cx| {
            cx.stop_propagation();
            on_begin(event.position.x, cx);
        })
        .child(
            div()
                .ml(theme().size(3.0))
                .w(theme().border)
                .h_full()
                .opacity(0.0)
                .bg(theme().colors.muted)
                .group_hover(id, |line| line.opacity(1.0)),
        )
}
