use std::path::PathBuf;

use gpui::{
    AppContext as _, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Task, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};

use super::{
    contract::{BoardData, BoardFilter, BoardLoadState},
    core::{filter_count, status_label},
    persistence::load_issues,
};
use crate::{
    app::PiApp,
    primitives::{ButtonTone, FeedbackTone, button, feedback, section_heading},
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
    next: Option<workgraph::contract::Issue>,
    active_count: usize,
    blocked_count: usize,
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
        self.state = BoardLoadState::Loading;
        let database = self.database.clone();
        let project = self.project.clone();
        let load = cx.background_spawn(async move { load_issues(database, project) });
        self.refresh = Some(cx.spawn(async move |weak, cx| {
            let state = match load.await {
                Ok(data) => BoardLoadState::Ready(data),
                Err(error) => BoardLoadState::Failed(error),
            };
            let _ = weak.update(cx, |this, cx| {
                this.state = state;
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Render for WorkGraphSidebarView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(section_heading("Work graph"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(THEME.space.xs)
                    .child(button(
                        "refresh-workgraph-sidebar",
                        "Refresh",
                        ButtonTone::Quiet,
                        true,
                        {
                            let entity = cx.entity().downgrade();
                            move |_, cx| {
                                if let Some(entity) = entity.upgrade() {
                                    entity.update(cx, |this, cx| this.refresh(cx));
                                }
                            }
                        },
                    ))
                    .child(button(
                        "open-workgraph-from-sidebar",
                        "Open",
                        ButtonTone::Quiet,
                        true,
                        {
                            let app = self.app.clone();
                            move |window, cx| {
                                let _ =
                                    app.update(cx, |app, cx| app.open_workgraph_sheet(window, cx));
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
                            .child("Work graph unavailable"),
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
        next: data
            .next
            .and_then(|number| data.issues.iter().find(|issue| issue.number == number))
            .cloned(),
        active_count: filter_count(data, BoardFilter::Active),
        blocked_count: filter_count(data, BoardFilter::Blocked),
    }
}

fn render_summary(summary: SidebarSummary, app: WeakEntity<PiApp>) -> impl IntoElement {
    let linked = summary.linked.clone();
    let featured = linked.clone().or_else(|| summary.next.clone());
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child(format!(
                    "{} active · {} need attention",
                    summary.active_count, summary.blocked_count
                )),
        )
        .when_some(featured, |card, issue| {
            let number = issue.number;
            let label = if linked.is_some() {
                format!("#{} · {}", issue.number, status_label(issue.status))
            } else {
                format!("Next · #{}", issue.number)
            };
            card.child(
                div()
                    .id(("workgraph-sidebar-issue", number))
                    .px(THEME.space.sm)
                    .py(THEME.space.xs)
                    .border_l(px(2.0))
                    .border_color(if linked.is_some() {
                        THEME.colors.accent
                    } else {
                        THEME.colors.border
                    })
                    .bg(THEME.colors.canvas)
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .cursor_pointer()
                    .hover(|card| card.bg(THEME.colors.hover))
                    .on_click(move |_, window, cx| {
                        let _ = app.update(cx, |app, cx| {
                            app.open_workgraph_issue(number, window, cx);
                        });
                    })
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if linked.is_some() {
                                THEME.colors.accent
                            } else {
                                THEME.colors.subtle
                            })
                            .child(label),
                    )
                    .child(
                        div()
                            .line_clamp(2)
                            .text_color(THEME.colors.text)
                            .child(issue.title),
                    ),
            )
        })
        .when(linked.is_none() && summary.active_count > 0, |card| {
            card.child(
                div()
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.muted)
                    .child("No issue linked to this session"),
            )
        })
        .when(summary.active_count == 0, |card| {
            card.child(
                div()
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.muted)
                    .child("No active project work"),
            )
        })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

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
    fn linked_session_issue_takes_precedence_over_project_next() {
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
            ready: HashSet::from([1]),
            next: Some(1),
            ..BoardData::default()
        };

        let summary = sidebar_summary(&data, Some("session-1"));

        assert_eq!(summary.linked.map(|issue| issue.number), Some(2));
        assert_eq!(summary.next.map(|issue| issue.number), Some(1));
        assert_eq!(summary.active_count, 2);
    }

    #[test]
    fn unlinked_session_still_sees_project_work_summary() {
        let data = BoardData {
            issues: vec![issue(4, IssueStatus::Open), issue(5, IssueStatus::Blocked)],
            ready: HashSet::from([4]),
            blocked: HashSet::from([5]),
            next: Some(4),
            ..BoardData::default()
        };

        let summary = sidebar_summary(&data, Some("unlinked"));

        assert_eq!(summary.linked, None);
        assert_eq!(summary.next.map(|issue| issue.number), Some(4));
        assert_eq!(summary.active_count, 2);
        assert_eq!(summary.blocked_count, 1);
    }
}
