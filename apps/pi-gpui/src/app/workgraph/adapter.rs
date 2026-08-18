//! GPUI and SQLite adapters for the workgraph board.

use std::path::PathBuf;

use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Task, Window, div,
    prelude::FluentBuilder as _, px,
};
use workgraph::{
    adapter::SqliteAdapter,
    contract::{PlanningView, SearchRequest, SearchResult},
    core::WorkGraph,
};

use super::{
    contract::{BoardData, BoardFilter, BoardLoadState, IssueGroup, IssueRow},
    core::{filter_count, project_groups},
};
use crate::{
    primitives::{ButtonTone, FeedbackTone, button, feedback},
    theme::THEME,
};

pub(crate) struct WorkGraphBoardView {
    database: PathBuf,
    project: PathBuf,
    state: BoardLoadState,
    filter: BoardFilter,
    refresh: Option<Task<()>>,
}

impl WorkGraphBoardView {
    pub(crate) fn new(
        database: Result<PathBuf, String>,
        project: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        let (database, state) = match database {
            Ok(database) => (database, BoardLoadState::Loading),
            Err(error) => (PathBuf::new(), BoardLoadState::Failed(error)),
        };
        let should_refresh = matches!(state, BoardLoadState::Loading);
        let mut view = Self {
            database,
            project,
            state,
            filter: BoardFilter::Active,
            refresh: None,
        };
        if should_refresh {
            view.refresh(cx);
        }
        view
    }

    pub(crate) fn refresh_for(&mut self, project: PathBuf, cx: &mut Context<Self>) {
        self.project = project;
        self.refresh(cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.state = BoardLoadState::Loading;
        let database = self.database.clone();
        let project = self.project.clone();
        let load = cx.background_spawn(async move { load_issues(database, project) });
        self.refresh = Some(cx.spawn(async move |weak, cx| {
            let state = match load.await {
                Ok(issues) => BoardLoadState::Ready(issues),
                Err(error) => BoardLoadState::Failed(error),
            };
            let _ = weak.update(cx, |this, cx| {
                this.state = state;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn set_filter(&mut self, filter: BoardFilter, cx: &mut Context<Self>) {
        if self.filter != filter {
            self.filter = filter;
            cx.notify();
        }
    }

    fn render_filter_rail(&self, entity: Entity<Self>, data: &BoardData) -> impl IntoElement {
        div()
            .w(px(176.0))
            .min_w(px(144.0))
            .flex_none()
            .flex()
            .flex_col()
            .gap(THEME.space.xs)
            .pr(THEME.space.sm)
            .border_r(THEME.border)
            .border_color(THEME.colors.border)
            .children(BoardFilter::ALL.into_iter().map(|filter| {
                let selected = filter == self.filter;
                let count = filter_count(data, filter);
                let entity = entity.clone();
                button(
                    format!("workgraph-filter-{filter:?}"),
                    format!("{}  {count}", filter.label()),
                    if selected {
                        ButtonTone::Neutral
                    } else {
                        ButtonTone::Quiet
                    },
                    true,
                    move |_, cx| {
                        entity.update(cx, |this, cx| this.set_filter(filter, cx));
                    },
                )
            }))
    }

    fn render_groups(&self, groups: Vec<IssueGroup>) -> impl IntoElement {
        div()
            .id("workgraph-issue-list")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(THEME.space.md)
            .children(groups.into_iter().map(render_group))
    }
}

impl Render for WorkGraphBoardView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        div()
            .size_full()
            .min_h_0()
            .p(THEME.space.md)
            .bg(THEME.colors.panel)
            .child(match &self.state {
                BoardLoadState::Loading => feedback(
                    "workgraph-loading",
                    "Loading work graph…",
                    FeedbackTone::Info,
                )
                .into_any_element(),
                BoardLoadState::Failed(error) => {
                    let retry = entity.clone();
                    div()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.sm)
                        .child(feedback(
                            "workgraph-error",
                            error.clone(),
                            FeedbackTone::Error,
                        ))
                        .child(button(
                            "workgraph-retry",
                            "Try again",
                            ButtonTone::Neutral,
                            true,
                            move |_, cx| {
                                retry.update(cx, |this, cx| this.refresh(cx));
                            },
                        ))
                        .into_any_element()
                }
                BoardLoadState::Ready(data) => {
                    let groups = project_groups(data, self.filter);
                    div()
                        .size_full()
                        .min_h_0()
                        .flex()
                        .gap(THEME.space.md)
                        .child(self.render_filter_rail(entity, data))
                        .child(if groups.is_empty() {
                            feedback(
                                "workgraph-empty",
                                self.filter.empty_message(),
                                FeedbackTone::Info,
                            )
                            .into_any_element()
                        } else {
                            self.render_groups(groups).into_any_element()
                        })
                        .into_any_element()
                }
            })
    }
}

fn load_issues(database: PathBuf, project: PathBuf) -> Result<BoardData, String> {
    let project = project
        .canonicalize()
        .map_err(|error| format!("resolve work graph project: {error}"))?
        .into_os_string()
        .into_string()
        .map_err(|_| "work graph project path is not valid UTF-8".to_owned())?;
    let adapter = SqliteAdapter::open(database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    let issues = match graph
        .search(&SearchRequest::Status {
            project: project.clone(),
            status: None,
        })
        .map_err(|error| error.to_string())?
    {
        SearchResult::Status(issues) => issues,
        _ => return Err("work graph returned an unexpected status result".into()),
    };
    let ready = planning_numbers(&mut graph, &project, PlanningView::Ready)?;
    let blocked = planning_numbers(&mut graph, &project, PlanningView::Blocked)?;
    let next = planning_numbers(&mut graph, &project, PlanningView::Next)?
        .into_iter()
        .next();
    Ok(BoardData {
        issues,
        ready: ready.into_iter().collect(),
        blocked: blocked.into_iter().collect(),
        next,
    })
}

fn planning_numbers(
    graph: &mut WorkGraph<SqliteAdapter>,
    project: &str,
    planning: PlanningView,
) -> Result<Vec<u64>, String> {
    match graph
        .search(&SearchRequest::Planning {
            project: project.to_owned(),
            planning,
        })
        .map_err(|error| error.to_string())?
    {
        SearchResult::Planning(issues) => {
            Ok(issues.into_iter().map(|issue| issue.number).collect())
        }
        _ => Err("work graph returned an unexpected planning result".into()),
    }
}

fn render_group(group: IssueGroup) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child(format!("{}  {}", group.group.label(), group.rows.len())),
        )
        .children(group.rows.into_iter().map(render_issue_row))
}

fn render_issue_row(row: IssueRow) -> impl IntoElement {
    let status_color = if row.status_label.starts_with("Blocked") {
        THEME.colors.warning
    } else {
        match row.issue.status {
            workgraph::contract::IssueStatus::Blocked => THEME.colors.warning,
            workgraph::contract::IssueStatus::Done => THEME.colors.success,
            workgraph::contract::IssueStatus::Cancelled => THEME.colors.subtle,
            workgraph::contract::IssueStatus::InProgress => THEME.colors.accent,
            workgraph::contract::IssueStatus::Open => THEME.colors.link,
        }
    };
    div()
        .rounded(THEME.radius)
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.surface)
        .p(THEME.space.sm)
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(THEME.space.sm)
                .child(
                    div()
                        .min_w_0()
                        .text_color(THEME.colors.text)
                        .child(format!("#{}  {}", row.issue.number, row.issue.title)),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.muted)
                        .child(row.priority_label),
                ),
        )
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(status_color)
                .child(row.status_label),
        )
        .when(!row.issue.body.trim().is_empty(), |item| {
            item.child(
                div()
                    .text_size(THEME.type_scale.body_small)
                    .text_color(THEME.colors.muted)
                    .child(row.issue.body.lines().next().unwrap_or_default().to_owned()),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use workgraph::contract::{EditAction, EditRequest, EditResult};

    #[test]
    fn board_loader_reads_issues_from_the_shared_gui_database() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let database = directory.path().join("gui-state.sqlite3");
        let project = directory.path().join("project");
        std::fs::create_dir(&project).expect("project directory");
        let _state = crate::state::StateStore::open_at(&database).expect("GUI state");
        let adapter = SqliteAdapter::open(&database).expect("workgraph adapter");
        let mut graph = WorkGraph::new(adapter);
        let project_key = project
            .canonicalize()
            .expect("canonical project")
            .into_os_string()
            .into_string()
            .expect("UTF-8 project");
        let prerequisite = graph
            .edit(&EditRequest {
                project: project_key.clone(),
                idempotency_key: "prerequisite".into(),
                action: EditAction::Create {
                    title: "Prerequisite".into(),
                    body: String::new(),
                    priority: 2,
                },
            })
            .expect("create prerequisite");
        let EditResult::Issue(prerequisite) = prerequisite else {
            panic!("issue result");
        };
        let blocked = graph
            .edit(&EditRequest {
                project: project_key.clone(),
                idempotency_key: "blocked".into(),
                action: EditAction::Create {
                    title: "Blocked issue".into(),
                    body: String::new(),
                    priority: 0,
                },
            })
            .expect("create blocked issue");
        let EditResult::Issue(blocked) = blocked else {
            panic!("issue result");
        };
        graph
            .edit(&EditRequest {
                project: project_key,
                idempotency_key: "dependency".into(),
                action: EditAction::AddDependency {
                    number: blocked.number,
                    depends_on: prerequisite.number,
                    expected_version: Some(blocked.version),
                },
            })
            .expect("add dependency");

        let loaded = load_issues(database, project).expect("board load");
        assert_eq!(loaded.issues.len(), 2);
        assert!(loaded.ready.contains(&prerequisite.number));
        assert!(loaded.blocked.contains(&blocked.number));
        assert_eq!(loaded.next, Some(prerequisite.number));
    }
}
