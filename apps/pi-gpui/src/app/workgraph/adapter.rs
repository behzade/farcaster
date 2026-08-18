//! GPUI and SQLite adapters for the workgraph board.

use std::path::PathBuf;

use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Task, Window, div,
    prelude::FluentBuilder as _, px,
};
use workgraph::{
    adapter::SqliteAdapter,
    contract::{EditAction, EditRequest, SearchRequest, SearchResult},
    core::WorkGraph,
};

use super::{
    contract::{BoardData, BoardFilter, BoardLoadState, BoardMode, IssueGroup, IssueRow},
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
    mode: BoardMode,
    selected: Option<u64>,
    active_session: Option<(String, String)>,
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
            mode: BoardMode::Kanban,
            selected: None,
            active_session: None,
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
        active_session: Option<(String, String)>,
        cx: &mut Context<Self>,
    ) {
        self.project = project;
        self.active_session = active_session;
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

    fn set_mode(&mut self, mode: BoardMode, cx: &mut Context<Self>) {
        if self.mode != mode {
            self.mode = mode;
            cx.notify();
        }
    }

    fn select_issue(&mut self, number: u64, cx: &mut Context<Self>) {
        self.selected = Some(number);
        cx.notify();
    }

    fn link_active_session(&mut self, number: u64, cx: &mut Context<Self>) {
        let Some((session_id, session_path)) = self.active_session.clone() else {
            return;
        };
        let database = self.database.clone();
        let project = self.project.clone();
        let edit = cx.background_spawn(async move {
            link_session(database, project, number, session_id, session_path)
        });
        self.state = BoardLoadState::Loading;
        self.refresh = Some(cx.spawn(async move |weak, cx| {
            let state = match edit.await {
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

    fn render_filter_rail(&self, entity: Entity<Self>, data: &BoardData) -> impl IntoElement {
        div()
            .w(px(216.0))
            .min_w(px(216.0))
            .flex_none()
            .flex()
            .flex_col()
            .gap(THEME.space.xs)
            .px(THEME.space.sm)
            .py(THEME.space.md)
            .bg(THEME.colors.canvas)
            .border_r(THEME.border)
            .border_color(THEME.colors.border)
            .child(
                div()
                    .px(THEME.space.sm)
                    .pb(THEME.space.sm)
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.subtle)
                    .child("WORK STATES"),
            )
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

    fn render_groups(&self, entity: Entity<Self>, groups: Vec<IssueGroup>) -> impl IntoElement {
        let selected = self.selected;
        div()
            .id("workgraph-issue-list")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(THEME.space.md)
            .p(THEME.space.md)
            .children(
                groups
                    .into_iter()
                    .map(|group| render_group(group, selected, entity.clone())),
            )
    }

    fn render_graph(&self, entity: Entity<Self>, data: &BoardData) -> impl IntoElement {
        let issues = data.issues.clone();
        let dependencies = data.dependencies.clone();
        div()
            .id("workgraph-dependency-list")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(THEME.space.xs)
            .children(issues.into_iter().map(move |issue| {
                let dependency_titles = dependencies
                    .iter()
                    .filter(|edge| edge.issue_number == issue.number)
                    .filter_map(|edge| {
                        data.issues
                            .iter()
                            .find(|candidate| candidate.number == edge.depends_on)
                            .map(|candidate| format!("#{} {}", candidate.number, candidate.title))
                    })
                    .collect::<Vec<_>>();
                render_graph_row(issue, dependency_titles, self.selected, entity.clone())
            }))
    }

    fn render_detail(&self, entity: Entity<Self>, data: &BoardData) -> impl IntoElement {
        let issue = self
            .selected
            .and_then(|number| data.issues.iter().find(|issue| issue.number == number));
        div()
            .id("workgraph-issue-detail")
            .w(px(400.0))
            .min_w(px(360.0))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .p(THEME.space.md)
            .bg(THEME.colors.panel)
            .border_l(THEME.border)
            .border_color(THEME.colors.border)
            .child(match issue {
                Some(issue) => {
                    let dependencies = data
                        .dependencies
                        .iter()
                        .filter(|edge| edge.issue_number == issue.number)
                        .filter_map(|edge| {
                            data.issues
                                .iter()
                                .find(|item| item.number == edge.depends_on)
                        })
                        .map(|item| format!("#{}  {}", item.number, item.title))
                        .collect::<Vec<_>>();
                    let dependents = data
                        .dependencies
                        .iter()
                        .filter(|edge| edge.depends_on == issue.number)
                        .filter_map(|edge| {
                            data.issues
                                .iter()
                                .find(|item| item.number == edge.issue_number)
                        })
                        .map(|item| format!("#{}  {}", item.number, item.title))
                        .collect::<Vec<_>>();
                    let sessions = data
                        .sessions
                        .iter()
                        .filter(|link| link.issue_number == issue.number)
                        .collect::<Vec<_>>();
                    let active_link = self.active_session.as_ref().and_then(|(id, _)| {
                        data.sessions.iter().find(|link| link.session_id == *id)
                    });
                    let session_action = self.active_session.as_ref().map(|_| {
                        let number = issue.number;
                        let linked_here =
                            active_link.is_some_and(|link| link.issue_number == issue.number);
                        button(
                            format!("workgraph-link-session-{number}"),
                            if linked_here {
                                "Current session linked"
                            } else if active_link.is_some() {
                                "Move current session here"
                            } else {
                                "Link current session"
                            },
                            if linked_here {
                                ButtonTone::Quiet
                            } else {
                                ButtonTone::Neutral
                            },
                            !linked_here,
                            move |_, cx| {
                                entity.update(cx, |this, cx| {
                                    this.link_active_session(number, cx);
                                });
                            },
                        )
                    });
                    div()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.md)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(THEME.space.xs)
                                .pb(THEME.space.md)
                                .border_b(THEME.border)
                                .border_color(THEME.colors.border)
                                .child(
                                    div()
                                        .text_size(THEME.type_scale.caption)
                                        .text_color(THEME.colors.subtle)
                                        .child(format!("ISSUE #{}", issue.number)),
                                )
                                .child(
                                    div()
                                        .text_size(THEME.type_scale.display)
                                        .text_color(THEME.colors.text)
                                        .child(issue.title.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(THEME.type_scale.caption)
                                        .text_color(status_color(issue.status))
                                        .child(format!(
                                            "{}  ·  Priority {}  ·  Version {}",
                                            super::core::status_label(issue.status),
                                            issue.priority,
                                            issue.version
                                        )),
                                ),
                        )
                        .child(detail_section(
                            "DESCRIPTION",
                            if issue.body.trim().is_empty() {
                                "No description recorded.".into()
                            } else {
                                issue.body.clone()
                            },
                        ))
                        .child(detail_section(
                            "DEPENDS ON",
                            if dependencies.is_empty() {
                                "Nothing — this issue can move independently.".into()
                            } else {
                                dependencies.join("\n")
                            },
                        ))
                        .child(detail_section(
                            "UNBLOCKS",
                            if dependents.is_empty() {
                                "No dependent issues.".into()
                            } else {
                                dependents.join("\n")
                            },
                        ))
                        .child(detail_section(
                            "LINKED SESSIONS",
                            if sessions.is_empty() {
                                "No sessions linked.".into()
                            } else {
                                sessions
                                    .iter()
                                    .map(|link| link.session_id.as_str())
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            },
                        ))
                        .children(session_action)
                        .into_any_element()
                }
                None => feedback(
                    "workgraph-detail-empty",
                    "Select an issue to inspect its dependencies.",
                    FeedbackTone::Info,
                )
                .into_any_element(),
            })
    }
}

impl Render for WorkGraphBoardView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        div()
            .size_full()
            .min_h_0()
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
                    let mode = self.mode;
                    let kanban = entity.clone();
                    let graph = entity.clone();
                    let active_count = filter_count(data, BoardFilter::Active);
                    let blocked_count = filter_count(data, BoardFilter::Blocked);
                    div()
                        .size_full()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .h(px(64.0))
                                .flex_none()
                                .px(THEME.space.md)
                                .flex()
                                .items_center()
                                .justify_between()
                                .border_b(THEME.border)
                                .border_color(THEME.colors.border)
                                .bg(THEME.colors.canvas)
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(THEME.space.xs)
                                        .child(
                                            div()
                                                .text_size(THEME.type_scale.display)
                                                .text_color(THEME.colors.text)
                                                .child("Project work"),
                                        )
                                        .child(
                                            div()
                                                .text_size(THEME.type_scale.caption)
                                                .text_color(THEME.colors.subtle)
                                                .child(format!(
                                                    "{active_count} active  ·  {blocked_count} need attention  ·  {} total",
                                                    data.issues.len()
                                                )),
                                        ),
                                )
                                .child(
                                    div().flex().gap(THEME.space.xs).children(
                                        [BoardMode::Kanban, BoardMode::Graph].map(|item| {
                                            let target = if item == BoardMode::Kanban {
                                                kanban.clone()
                                            } else {
                                                graph.clone()
                                            };
                                            button(
                                                format!("workgraph-mode-{item:?}"),
                                                item.label(),
                                                if item == mode {
                                                    ButtonTone::Neutral
                                                } else {
                                                    ButtonTone::Quiet
                                                },
                                                true,
                                                move |_, cx| {
                                                    target.update(cx, |this, cx| {
                                                        this.set_mode(item, cx)
                                                    })
                                                },
                                            )
                                        }),
                                    ),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .flex()
                                .child(self.render_filter_rail(entity.clone(), data))
                                .child(if groups.is_empty() {
                                    feedback(
                                        "workgraph-empty",
                                        self.filter.empty_message(),
                                        FeedbackTone::Info,
                                    )
                                    .into_any_element()
                                } else if self.mode == BoardMode::Graph {
                                    self.render_graph(entity.clone(), data).into_any_element()
                                } else {
                                    self.render_groups(entity.clone(), groups)
                                        .into_any_element()
                                })
                                .child(self.render_detail(entity, data)),
                        )
                        .into_any_element()
                }
            })
    }
}

fn link_session(
    database: PathBuf,
    project: PathBuf,
    number: u64,
    session_id: String,
    session_path: String,
) -> Result<BoardData, String> {
    let project_key = canonical_project(&project)?;
    let adapter = SqliteAdapter::open(&database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    let operation = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "system clock is unavailable".to_owned())?
        .as_nanos();
    graph
        .edit(&EditRequest {
            project: project_key,
            idempotency_key: format!("gui-link-{number}-{operation}"),
            action: EditAction::LinkSession {
                number,
                session_id,
                session_path,
                expected_version: None,
            },
        })
        .map_err(|error| error.to_string())?;
    load_issues(database, project)
}

fn canonical_project(project: &std::path::Path) -> Result<String, String> {
    project
        .canonicalize()
        .map_err(|error| format!("resolve work graph project: {error}"))?
        .into_os_string()
        .into_string()
        .map_err(|_| "work graph project path is not valid UTF-8".to_owned())
}

pub(super) fn load_issues(database: PathBuf, project: PathBuf) -> Result<BoardData, String> {
    let project = canonical_project(&project)?;
    let adapter = SqliteAdapter::open(database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    match graph
        .search(&SearchRequest::Graph { project })
        .map_err(|error| error.to_string())?
    {
        SearchResult::Graph(graph) => Ok(BoardData {
            issues: graph.issues,
            dependencies: graph.dependencies,
            sessions: graph.sessions,
            ready: graph.ready.into_iter().collect(),
            blocked: graph.blocked.into_iter().collect(),
            next: graph.next,
        }),
        _ => Err("work graph returned an unexpected graph result".into()),
    }
}

fn detail_section(label: &'static str, body: String) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child(label),
        )
        .child(
            div()
                .text_size(THEME.type_scale.body_small)
                .text_color(THEME.colors.muted)
                .line_height(THEME.type_scale.line_body)
                .child(body),
        )
}

fn status_color(status: workgraph::contract::IssueStatus) -> gpui::Rgba {
    match status {
        workgraph::contract::IssueStatus::Blocked => THEME.colors.warning,
        workgraph::contract::IssueStatus::Done => THEME.colors.success,
        workgraph::contract::IssueStatus::Cancelled => THEME.colors.subtle,
        workgraph::contract::IssueStatus::InProgress => THEME.colors.accent,
        workgraph::contract::IssueStatus::Open => THEME.colors.link,
    }
}

fn render_group(
    group: IssueGroup,
    selected: Option<u64>,
    entity: Entity<WorkGraphBoardView>,
) -> impl IntoElement {
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
        .children(
            group
                .rows
                .into_iter()
                .map(|row| render_issue_row(row, selected, entity.clone())),
        )
}

fn render_issue_row(
    row: IssueRow,
    selected: Option<u64>,
    entity: Entity<WorkGraphBoardView>,
) -> impl IntoElement {
    let row_status_color = if row.status_label.starts_with("Blocked") {
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
    let number = row.issue.number;
    let is_selected = selected == Some(number);
    div()
        .id(format!("workgraph-issue-{number}"))
        .cursor_pointer()
        .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.select_issue(number, cx)))
        .rounded(THEME.radius)
        .border(THEME.border)
        .border_color(if is_selected {
            THEME.colors.accent
        } else {
            THEME.colors.border
        })
        .bg(if is_selected {
            THEME.colors.selection
        } else {
            THEME.colors.surface
        })
        .hover(|style| style.bg(THEME.colors.hover))
        .px(THEME.space.sm)
        .py(THEME.space.sm)
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
                .text_color(row_status_color)
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

fn render_graph_row(
    issue: workgraph::contract::Issue,
    dependencies: Vec<String>,
    selected: Option<u64>,
    entity: Entity<WorkGraphBoardView>,
) -> impl IntoElement {
    let number = issue.number;
    let is_selected = selected == Some(number);
    div()
        .id(format!("workgraph-graph-{number}"))
        .cursor_pointer()
        .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.select_issue(number, cx)))
        .px(THEME.space.md)
        .py(THEME.space.sm)
        .border_b(THEME.border)
        .border_color(THEME.colors.border)
        .bg(if is_selected {
            THEME.colors.selection
        } else {
            THEME.colors.panel
        })
        .hover(|style| style.bg(THEME.colors.hover))
        .flex()
        .items_center()
        .gap(THEME.space.md)
        .child(
            div()
                .w(px(220.0))
                .text_color(THEME.colors.text)
                .child(format!("#{number}  {}", issue.title)),
        )
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child(if dependencies.is_empty() {
                    "Ready root".into()
                } else {
                    format!("← {}", dependencies.join(", "))
                }),
        )
}
