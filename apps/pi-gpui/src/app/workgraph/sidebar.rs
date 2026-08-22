use std::path::PathBuf;

use gpui::{
    AppContext as _, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Task, WeakEntity, div,
    prelude::FluentBuilder as _,
};

use super::{
    components::{detail_card, detail_label, status_pill},
    contract::{BoardData, BoardLoadState},
    persistence::load_issues,
};
use crate::{
    app::PiApp,
    assets::AppIcon,
    primitives::{ButtonTone, FeedbackTone, button, feedback, icon_button, section_heading},
    theme::THEME,
};

pub(crate) struct WorkGraphSidebarView {
    app: WeakEntity<PiApp>,
    database: PathBuf,
    project: PathBuf,
    session_id: Option<String>,
    state: BoardLoadState,
    refresh: Option<Task<()>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SidebarSummary {
    linked: Option<workgraph::contract::Issue>,
}

impl WorkGraphSidebarView {
    pub(crate) fn new(
        app: WeakEntity<PiApp>,
        database: Result<PathBuf, String>,
        project: PathBuf,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let (database, state) = match database {
            Ok(database) => (database, BoardLoadState::Loading),
            Err(error) => (PathBuf::new(), BoardLoadState::Failed(error)),
        };
        let should_refresh = matches!(state, BoardLoadState::Loading);
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
        self.project = project;
        self.session_id = session_id;
        self.refresh(cx);
    }

    pub(crate) fn refresh(&mut self, cx: &mut gpui::Context<Self>) {
        let notify_loading = prepare_refresh(&mut self.state);
        let database = self.database.clone();
        let project = self.project.clone();
        let load = cx.background_spawn(async move { load_issues(database, project) });
        self.refresh = Some(cx.spawn(async move |weak, cx| {
            let state = match load.await {
                Ok(data) => BoardLoadState::Ready(data),
                Err(error) => BoardLoadState::Failed(error),
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

fn prepare_refresh(state: &mut BoardLoadState) -> bool {
    if matches!(state, BoardLoadState::Ready(_) | BoardLoadState::Loading) {
        return false;
    }
    *state = BoardLoadState::Loading;
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
            .child(section_heading("Current work"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(THEME.space.xs)
                    .child(icon_button(
                        "refresh-workgraph-sidebar",
                        AppIcon::ArrowCounterClockwise,
                        "Refresh work graph",
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
                        AppIcon::ArrowSquareOut,
                        "View all project work",
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
        div()
            .flex()
            .flex_col()
            .gap(THEME.space.xs)
            .child(header)
            .child(match &self.state {
                BoardLoadState::Loading => feedback(
                    "workgraph-sidebar-loading",
                    "Loading project work…",
                    FeedbackTone::Info,
                )
                .into_any_element(),
                BoardLoadState::Failed(_) => div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(THEME.space.sm)
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child("Project work unavailable"),
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
                BoardLoadState::Ready(data) => render_summary(
                    sidebar_summary(data, self.session_id.as_deref()),
                    self.app.clone(),
                )
                .into_any_element(),
            })
    }
}

fn sidebar_summary(data: &BoardData, session_id: Option<&str>) -> SidebarSummary {
    let linked_number = session_id.and_then(|session_id| {
        data.sessions
            .iter()
            .find(|link| link.session_id == session_id)
            .map(|link| link.issue_number)
    });
    SidebarSummary {
        linked: linked_number
            .and_then(|number| data.issues.iter().find(|issue| issue.number == number))
            .cloned(),
    }
}

fn render_summary(summary: SidebarSummary, app: WeakEntity<PiApp>) -> impl IntoElement {
    let has_linked = summary.linked.is_some();
    let choose_issue = app.clone();
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .when_some(summary.linked, |card, issue| {
            let number = issue.number;
            card.child(
                div()
                    .id(("workgraph-sidebar-issue", number))
                    .p(THEME.space.sm)
                    .flex()
                    .flex_col()
                    .gap(THEME.space.sm)
                    .rounded(THEME.radius)
                    .border(THEME.border)
                    .border_color(THEME.colors.border)
                    .bg(THEME.colors.canvas)
                    .cursor_pointer()
                    .hover(|card| card.bg(THEME.colors.surface))
                    .on_click(move |_, window, cx| {
                        let _ = app.update(cx, |app, cx| {
                            app.inspect_workgraph_issue(number, window, cx);
                        });
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(THEME.type_scale.caption)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(THEME.colors.muted)
                                    .child(format!("Issue #{}", issue.number)),
                            )
                            .child(status_pill(issue.status)),
                    )
                    .child(
                        div()
                            .line_clamp(2)
                            .text_size(THEME.type_scale.body_small)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(THEME.colors.text)
                            .child(issue.title),
                    )
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.link)
                            .child("Open issue details"),
                    ),
            )
        })
        .when(!has_linked, |card| {
            card.child(
                detail_card()
                    .child(detail_label("No linked issue"))
                    .child(
                        div()
                            .text_size(THEME.type_scale.body_small)
                            .text_color(THEME.colors.muted)
                            .child("Attach this session to project work."),
                    )
                    .child(button(
                        "choose-current-work",
                        "Choose issue",
                        ButtonTone::Quiet,
                        true,
                        move |window, cx| {
                            let _ = choose_issue.update(cx, |app, cx| {
                                app.open_workgraph_surface(window, cx);
                            });
                        },
                    )),
            )
        })
}

#[cfg(test)]
mod tests {
    use workgraph::contract::{Issue, IssueStatus, SessionLink};

    use super::*;

    fn issue(number: u64, status: IssueStatus) -> Issue {
        Issue {
            project: "/project".into(),
            number,
            title: format!("Issue {number}"),
            body: String::new(),
            status,
            priority: 0,
            version: 1,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn ready_content_remains_visible_while_refreshing() {
        let data = BoardData {
            issues: vec![issue(1, IssueStatus::Open)],
            ..BoardData::default()
        };
        let mut state = BoardLoadState::Ready(data.clone());

        assert!(!prepare_refresh(&mut state));
        assert_eq!(state, BoardLoadState::Ready(data));

        let mut failed = BoardLoadState::Failed("unavailable".into());
        assert!(prepare_refresh(&mut failed));
        assert_eq!(failed, BoardLoadState::Loading);
    }

    #[test]
    fn summary_contains_only_the_current_sessions_linked_issue() {
        let data = BoardData {
            issues: vec![
                issue(1, IssueStatus::Open),
                issue(2, IssueStatus::InProgress),
            ],
            sessions: vec![SessionLink {
                session_id: "session-1".into(),
                session_path: "/sessions/1.jsonl".into(),
                issue_number: 2,
                linked_at: 0,
            }],
            next: Some(1),
            ..BoardData::default()
        };

        let summary = sidebar_summary(&data, Some("session-1"));

        assert_eq!(summary.linked.as_ref().map(|issue| issue.number), Some(2));
        assert_eq!(
            summary.linked.as_ref().map(|issue| issue.status),
            Some(IssueStatus::InProgress)
        );
        assert_eq!(
            summary.linked.as_ref().map(|issue| issue.title.as_str()),
            Some("Issue 2")
        );
        assert_eq!(sidebar_summary(&data, Some("unlinked")).linked, None);
    }
}
