use std::path::PathBuf;

use gpui::{
    AppContext as _, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Role, StatefulInteractiveElement as _, Styled as _, Task, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};

use super::{
    contract::{PlanLoadState, PlanRow},
    core::plan_rows,
    persistence::load_plan,
};
use crate::{
    app::FarcasterApp,
    assets::AppIcon,
    primitives::{
        AppIconSize, ButtonTone, FeedbackTone, app_icon, button, feedback, icon_button,
        section_heading,
    },
    theme::THEME,
};

pub(crate) struct WorkGraphSidebarView {
    app: WeakEntity<FarcasterApp>,
    database: PathBuf,
    project: PathBuf,
    session_id: Option<String>,
    state: PlanLoadState,
    refresh: Option<Task<()>>,
}

impl WorkGraphSidebarView {
    pub(crate) fn new(
        app: WeakEntity<FarcasterApp>,
        database: Result<PathBuf, String>,
        project: PathBuf,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let (database, state) = match database {
            Ok(database) => (database, PlanLoadState::Loading),
            Err(error) => (PathBuf::new(), PlanLoadState::Failed(error)),
        };
        let should_refresh = matches!(state, PlanLoadState::Loading);
        let mut view = Self {
            app,
            database,
            project,
            session_id: None,
            state,
            refresh: None,
        };
        if should_refresh {
            view.refresh(cx);
        }
        view
    }

    pub(crate) fn refresh_for(
        &mut self,
        project: PathBuf,
        session_id: Option<String>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.project != project || self.session_id != session_id {
            self.state = PlanLoadState::Ready(Box::default());
        }
        self.project = project;
        self.session_id = session_id;
        self.refresh(cx);
    }

    pub(crate) fn refresh(&mut self, cx: &mut gpui::Context<Self>) {
        let notify_loading = prepare_refresh(&mut self.state);
        let database = self.database.clone();
        let project = self.project.clone();
        let session_id = self.session_id.clone();
        let load =
            cx.background_spawn(async move { load_plan(database, project, session_id.as_deref()) });
        self.refresh = Some(cx.spawn(async move |weak, cx| {
            let state = match load.await {
                Ok(data) => PlanLoadState::Ready(Box::new(data)),
                Err(error) => PlanLoadState::Failed(error),
            };
            let _ = weak.update(cx, |this, cx| {
                if this.state != state {
                    this.state = state;
                    cx.notify();
                }
            });
        }));
        if notify_loading {
            cx.notify();
        }
    }
}

fn prepare_refresh(state: &mut PlanLoadState) -> bool {
    if matches!(state, PlanLoadState::Ready(_) | PlanLoadState::Loading) {
        return false;
    }
    *state = PlanLoadState::Loading;
    true
}

impl Render for WorkGraphSidebarView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let _timing = crate::performance::Timing::new("render.workgraph_sidebar");
        let entity = cx.entity().downgrade();
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(section_heading("Current plan"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(THEME.space.xs)
                    .child(icon_button(
                        "refresh-workgraph-sidebar",
                        AppIcon::ArrowsClockwise,
                        "Refresh plan",
                        ButtonTone::Quiet,
                        {
                            let entity = cx.entity().downgrade();
                            move |_, cx| {
                                if let Some(entity) = entity.upgrade() {
                                    entity.update(cx, |this, cx| this.refresh(cx));
                                }
                            }
                        },
                    ))
                    .child(icon_button(
                        "open-workgraph-from-sidebar",
                        AppIcon::ArrowsOut,
                        "View project plan",
                        ButtonTone::Quiet,
                        {
                            let app = self.app.clone();
                            move |window, cx| {
                                let _ = app.update(cx, |app, cx| {
                                    app.open_workgraph_surface(window, cx);
                                });
                            }
                        },
                    )),
            );
        let visible = sidebar_visible(&self.state, self.session_id.is_some());
        div()
            .when(!visible, |sidebar| sidebar.hidden())
            .when(visible, |sidebar| {
                sidebar
                    .flex()
                    .flex_col()
                    .gap(px(11.0))
                    .child(header)
                    .child(match &self.state {
                        PlanLoadState::Loading => feedback(
                            "workgraph-sidebar-loading",
                            "Loading plan…",
                            FeedbackTone::Info,
                        )
                        .into_any_element(),
                        PlanLoadState::Failed(_) => div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(THEME.space.sm)
                            .child(
                                div()
                                    .text_size(THEME.type_scale.caption)
                                    .text_color(THEME.colors.subtle)
                                    .child("Plan unavailable"),
                            )
                            .child(button(
                                "workgraph-sidebar-retry",
                                "Retry",
                                ButtonTone::Quiet,
                                true,
                                move |_, cx| {
                                    if let Some(entity) = entity.upgrade() {
                                        entity.update(cx, |this, cx| this.refresh(cx));
                                    }
                                },
                            ))
                            .into_any_element(),
                        PlanLoadState::Ready(data) => div()
                            .flex()
                            .flex_col()
                            .children(
                                data.snapshot
                                    .as_ref()
                                    .map(|snapshot| plan_rows(snapshot, ""))
                                    .unwrap_or_default()
                                    .into_iter()
                                    .map(|row| render_sidebar_row(row, self.app.clone())),
                            )
                            .into_any_element(),
                    })
            })
    }
}

fn sidebar_visible(state: &PlanLoadState, has_session: bool) -> bool {
    has_session && !matches!(state, PlanLoadState::Ready(data) if data.session_link.is_none())
}

fn render_sidebar_row(row: PlanRow, app: WeakEntity<FarcasterApp>) -> impl IntoElement {
    let number = row.node.number;
    let title_color = if row.reached {
        THEME.colors.subtle
    } else {
        THEME.colors.text
    };
    div()
        .id(("workgraph-sidebar-node", number))
        .role(Role::Button)
        .aria_label(format!("Open plan node {}", row.node.title))
        .tab_index(0)
        .cursor_pointer()
        .px(px(2.0))
        .py(px(3.0))
        .flex()
        .items_start()
        .gap(px(7.0))
        .hover(|row| row.bg(THEME.colors.surface))
        .on_click(move |_, window, cx| {
            let _ = app.update(cx, |app, cx| {
                app.open_workgraph_node(number, window, cx);
            });
        })
        .child(
            div()
                .w(px(16.0))
                .h(px(20.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .text_color(if row.reached {
                    THEME.colors.success
                } else if row.current {
                    THEME.colors.accent
                } else {
                    THEME.colors.subtle
                })
                .when(row.reached, |marker| {
                    marker.child(app_icon(AppIcon::CheckCircle, AppIconSize::Inline))
                })
                .when(!row.reached, |marker| {
                    marker.child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{number}")),
                    )
                }),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .line_clamp(2)
                        .text_size(THEME.type_scale.body_small)
                        .font_weight(if row.current {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(title_color)
                        .when(row.reached, |title| title.line_through())
                        .child(row.node.title),
                )
                .when(!row.node.files.is_empty(), |content| {
                    content.child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(format!("{} path(s)", row.node.files.len())),
                    )
                }),
        )
}

#[cfg(test)]
mod tests {
    use workgraph::contract::SessionLink;

    use super::*;
    use crate::app::workgraph::contract::PlanData;

    fn state(attached: bool) -> PlanLoadState {
        PlanLoadState::Ready(Box::new(PlanData {
            session_link: attached.then_some(SessionLink {
                session_id: "session-1".into(),
                session_path: "/sessions/1.jsonl".into(),
                plan_number: 1,
                walk_number: 1,
                linked_at: 0,
            }),
            ..PlanData::default()
        }))
    }

    #[test]
    fn sidebar_requires_a_real_session_link() {
        assert!(sidebar_visible(&state(true), true));
        assert!(!sidebar_visible(&state(false), true));
        assert!(!sidebar_visible(&state(true), false));
    }
}
