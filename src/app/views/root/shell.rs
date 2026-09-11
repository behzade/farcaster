use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ObjectFit, ParentElement as _,
    Styled as _, StyledImage as _, WeakEntity, div, img,
    prelude::FluentBuilder as _,
};

use super::{super::FarcasterApp, draft};
use crate::app::{
    AppSurface,
    ui::{
        layout::{
            LayoutMode, composer_bottom_clearance, shows_left_inline,
            shows_right_inline,
        },
        theme::THEME,
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
                    .child(self.transcript_view.clone())
                    .into_any_element()
            } else {
                draft::render_body(
                    self.composer_view.clone(),
                    editable_draft_project.map(|project| {
                        draft::render_heading(project, entity.clone()).into_any_element()
                    }),
                    self.composer_focus.clone(),
                    viewport_height,
                )
                .into_any_element()
            })
            .when(has_conversation, |main| {
                main.child(
                    div()
                        .w_full()
                        .max_w(THEME.layout.conversation_width)
                        .mx_auto()
                        .px(THEME.space.md)
                        .pt(THEME.space.sm)
                        .pb(composer_bottom_clearance(viewport_height))
                        .flex_none()
                        .child(self.composer_view.clone()),
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
        let native_surface = matches!(self.surface, AppSurface::Editor | AppSurface::Terminal);
        let native_surface_covered = native_surface
            && self.native_surface_covered
            && self.native_workspace_covered_by_overlay();
        let main = if native_surface_covered {
            div()
                .size_full()
                .min_h_0()
                .when_some(self.native_surface_snapshot.clone(), |surface, snapshot| {
                    surface.child(img(snapshot).size_full().object_fit(ObjectFit::Fill))
                })
                .into_any_element()
        } else {
            match self.surface {
                AppSurface::Editor if self.editor.is_some() => self.render_editor_surface(),
                AppSurface::Terminal if self.terminal.is_some() => self.render_terminal_workspace(),
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
                native_surface && self.extension.dialog.is_some(),
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
                        .min_w(THEME.layout.session_rail_min)
                        .max_w(THEME.layout.session_rail_max)
                        .flex_none()
                        .border_r(THEME.border)
                        .border_color(THEME.colors.border)
                        .child(
                            self.session_rail_view
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
                        .min_w(THEME.layout.run_panel_min)
                        .max_w(THEME.layout.run_panel_max)
                        .flex_none()
                        .border_l(THEME.border)
                        .border_color(THEME.colors.border)
                        .child(
                            if self.workgraph_inspector_issue.is_some()
                                && self.visible_review().is_none()
                            {
                                self.workgraph_detail_view.clone().into_any_element()
                            } else {
                                self.run_panel_view
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
        .w(gpui::px(7.0))
        .cursor_col_resize()
        .group(id)
        .on_mouse_down(gpui::MouseButton::Left, move |event, _, cx| {
            cx.stop_propagation();
            on_begin(event.position.x, cx);
        })
        .child(
            div()
                .ml(gpui::px(3.0))
                .w(THEME.border)
                .h_full()
                .opacity(0.0)
                .bg(THEME.colors.muted)
                .group_hover(id, |line| line.opacity(1.0)),
        )
}
