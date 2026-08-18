//! GPUI and SQLite adapters for the workgraph board.

use std::path::PathBuf;

use super::{
    components::{
        dependency_issue_section, detail_section, related_issue_section, render_create,
        render_edit_fields, render_graph_row, render_group, status_color,
    },
    contract::{BoardData, BoardFilter, BoardLoadState, BoardMode, IssueGroup},
    core::{filter_count, matching_project_groups},
    layout::{BoardLayoutMode, board_layout_mode, issue_detail_shell},
    persistence::{
        add_dependency, add_issue_note, create_issue, link_session, load_issues, remove_dependency,
        update_issue_fields, update_issue_status,
    },
};
use crate::{
    primitives::{ButtonTone, FeedbackTone, button, feedback},
    theme::THEME,
};
use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};

pub(crate) struct WorkGraphBoardView {
    database: PathBuf,
    project: PathBuf,
    state: BoardLoadState,
    filter: BoardFilter,
    mode: BoardMode,
    selected: Option<u64>,
    creating: bool,
    editing: Option<u64>,
    active_session: Option<(String, String)>,
    search: Option<Entity<InputState>>,
    create_title: Option<Entity<InputState>>,
    create_body: Option<Entity<TextareaState>>,
    edit_title: Option<Entity<InputState>>,
    edit_body: Option<Entity<TextareaState>>,
    edit_priority: Option<Entity<InputState>>,
    dependency: Option<Entity<InputState>>,
    note: Option<Entity<TextareaState>>,
    note_issue: Option<u64>,
    refresh: Option<Task<()>>,
    subscriptions: Vec<Subscription>,
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
            creating: false,
            editing: None,
            active_session: None,
            search: None,
            create_title: None,
            create_body: None,
            edit_title: None,
            edit_body: None,
            edit_priority: None,
            dependency: None,
            note: None,
            note_issue: None,
            refresh: None,
            subscriptions: Vec::new(),
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

    pub(super) fn select_issue(&mut self, number: u64, cx: &mut Context<Self>) {
        self.creating = false;
        self.editing = None;
        self.selected = Some(number);
        cx.notify();
    }

    pub(super) fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.creating = false;
        self.editing = None;
        self.selected = None;
        cx.notify();
    }

    fn start_create(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        self.editing = None;
        self.creating = true;
        cx.notify();
    }

    pub(super) fn set_editing(&mut self, number: Option<u64>, cx: &mut Context<Self>) {
        self.editing = number;
        cx.notify();
    }

    pub(super) fn update_issue_fields(
        &mut self,
        number: u64,
        title: String,
        body: String,
        priority: u64,
        expected_version: u64,
        cx: &mut Context<Self>,
    ) {
        let database = self.database.clone();
        let project = self.project.clone();
        let edit = cx.background_spawn(async move {
            update_issue_fields(
                database,
                project,
                number,
                title,
                body,
                priority,
                expected_version,
            )
        });
        self.state = BoardLoadState::Loading;
        self.refresh = Some(cx.spawn(async move |weak, cx| {
            let state = match edit.await {
                Ok(data) => BoardLoadState::Ready(data),
                Err(error) => BoardLoadState::Failed(error),
            };
            let _ = weak.update(cx, |this, cx| {
                this.state = state;
                this.editing = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn create_issue(&mut self, title: String, body: String, cx: &mut Context<Self>) {
        let database = self.database.clone();
        let project = self.project.clone();
        let edit = cx.background_spawn(async move { create_issue(database, project, title, body) });
        self.state = BoardLoadState::Loading;
        self.refresh = Some(cx.spawn(async move |weak, cx| {
            let result = edit.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok((data, number)) => {
                        this.state = BoardLoadState::Ready(data);
                        this.selected = Some(number);
                        this.creating = false;
                    }
                    Err(error) => this.state = BoardLoadState::Failed(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn set_issue_status(
        &mut self,
        number: u64,
        status: workgraph::contract::IssueStatus,
        expected_version: u64,
        cx: &mut Context<Self>,
    ) {
        let database = self.database.clone();
        let project = self.project.clone();
        let edit = cx.background_spawn(async move {
            update_issue_status(database, project, number, status, expected_version)
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

    pub(super) fn change_dependency(
        &mut self,
        number: u64,
        depends_on: u64,
        expected_version: u64,
        add: bool,
        cx: &mut Context<Self>,
    ) {
        let database = self.database.clone();
        let project = self.project.clone();
        let edit = cx.background_spawn(async move {
            if add {
                add_dependency(database, project, number, depends_on, expected_version)
            } else {
                remove_dependency(database, project, number, depends_on, expected_version)
            }
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

    fn add_note(
        &mut self,
        number: u64,
        expected_version: u64,
        body: String,
        cx: &mut Context<Self>,
    ) {
        let database = self.database.clone();
        let project = self.project.clone();
        let edit = cx.background_spawn(async move {
            add_issue_note(database, project, number, expected_version, body)
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
        let current_issue =
            self.active_session
                .as_ref()
                .and_then(|(session_id, _)| match &self.state {
                    BoardLoadState::Ready(data) => data
                        .sessions
                        .iter()
                        .find(|link| link.session_id == *session_id)
                        .map(|link| link.issue_number),
                    BoardLoadState::Loading | BoardLoadState::Failed(_) => None,
                });
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
                    .map(|group| render_group(group, selected, current_issue, entity.clone())),
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

    fn render_detail(
        &self,
        entity: Entity<Self>,
        data: &BoardData,
        layout: BoardLayoutMode,
    ) -> impl IntoElement {
        let issue = self
            .selected
            .and_then(|number| data.issues.iter().find(|issue| issue.number == number));
        let narrow = issue_detail_shell(layout).shows_sheet(false);
        div()
            .id("workgraph-issue-detail")
            .w(px(400.0))
            .min_w(px(360.0))
            .when(narrow, |detail| detail.w_full().min_w_0())
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .p(THEME.space.md)
            .bg(THEME.colors.panel)
            .border_l(THEME.border)
            .border_color(THEME.colors.border)
            .child(match issue {
                Some(issue) if self.editing == Some(issue.number) => render_edit_fields(
                    issue.clone(),
                    self.edit_title.as_ref().expect("edit title initialized"),
                    self.edit_body.as_ref().expect("edit body initialized"),
                    self.edit_priority
                        .as_ref()
                        .expect("edit priority initialized"),
                    entity,
                )
                .into_any_element(),
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
                        .cloned()
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
                        .cloned()
                        .collect::<Vec<_>>();
                    let sessions = data
                        .sessions
                        .iter()
                        .filter(|link| link.issue_number == issue.number)
                        .collect::<Vec<_>>();
                    let notes = data
                        .notes
                        .iter()
                        .filter(|note| note.issue_number == issue.number)
                        .collect::<Vec<_>>();
                    let active_link = self.active_session.as_ref().and_then(|(id, _)| {
                        data.sessions.iter().find(|link| link.session_id == *id)
                    });
                    let dependency_action = self.dependency.as_ref().map(|dependency| {
                        let dependency_input = dependency.clone();
                        let dependency_submit = dependency.clone();
                        let entity = entity.clone();
                        let number = issue.number;
                        let version = issue.version;
                        div()
                            .flex()
                            .gap(THEME.space.xs)
                            .child(Input::new(&dependency_input).w(px(160.0)))
                            .child(button(
                                format!("workgraph-add-dependency-{number}"),
                                "Add dependency",
                                ButtonTone::Quiet,
                                true,
                                move |window, cx| {
                                    let value =
                                        dependency_submit.read(cx).value().trim().to_owned();
                                    let Ok(depends_on) =
                                        value.strip_prefix('#').unwrap_or(&value).parse::<u64>()
                                    else {
                                        return;
                                    };
                                    dependency_submit.update(cx, |input, cx| {
                                        input.set_value(String::new(), window, cx);
                                    });
                                    entity.update(cx, |this, cx| {
                                        this.change_dependency(
                                            number, depends_on, version, true, cx,
                                        );
                                    });
                                },
                            ))
                    });
                    let note_action = self.note.as_ref().map(|note| {
                        let note_input = note.clone();
                        let note_submit = note.clone();
                        let entity = entity.clone();
                        let number = issue.number;
                        let version = issue.version;
                        div()
                            .flex()
                            .flex_col()
                            .gap(THEME.space.xs)
                            .child(Textarea::new(&note_input).w_full().appearance(true))
                            .child(button(
                                format!("workgraph-add-note-{number}"),
                                "Add note",
                                ButtonTone::Neutral,
                                true,
                                move |window, cx| {
                                    let body = note_submit.read(cx).value().trim().to_owned();
                                    if body.is_empty() {
                                        return;
                                    }
                                    note_submit.update(cx, |input, cx| {
                                        input.set_value(String::new(), window, cx);
                                    });
                                    entity.update(cx, |this, cx| {
                                        this.add_note(number, version, body, cx);
                                    });
                                },
                            ))
                    });
                    let edit_title = self
                        .edit_title
                        .as_ref()
                        .expect("edit title initialized")
                        .clone();
                    let edit_body = self
                        .edit_body
                        .as_ref()
                        .expect("edit body initialized")
                        .clone();
                    let edit_priority = self
                        .edit_priority
                        .as_ref()
                        .expect("edit priority initialized")
                        .clone();
                    let edit_entity = entity.clone();
                    let edit_number = issue.number;
                    let current_title = issue.title.clone();
                    let current_body = issue.body.clone();
                    let current_priority = issue.priority.to_string();
                    let edit_action = button(
                        format!("workgraph-edit-{edit_number}"),
                        "Edit issue",
                        ButtonTone::Quiet,
                        true,
                        move |window, cx| {
                            edit_title.update(cx, |input, cx| {
                                input.set_value(current_title.clone(), window, cx);
                            });
                            edit_body.update(cx, |input, cx| {
                                input.set_value(current_body.clone(), window, cx);
                            });
                            edit_priority.update(cx, |input, cx| {
                                input.set_value(current_priority.clone(), window, cx);
                            });
                            edit_entity.update(cx, |this, cx| {
                                this.set_editing(Some(edit_number), cx);
                            });
                        },
                    );
                    let status_actions = [
                        workgraph::contract::IssueStatus::Open,
                        workgraph::contract::IssueStatus::InProgress,
                        workgraph::contract::IssueStatus::Blocked,
                        workgraph::contract::IssueStatus::Done,
                        workgraph::contract::IssueStatus::Cancelled,
                    ]
                    .into_iter()
                    .filter(|status| *status != issue.status)
                    .map(|status| {
                        let entity = entity.clone();
                        let number = issue.number;
                        let version = issue.version;
                        button(
                            format!("workgraph-status-{number}-{status:?}"),
                            super::core::status_label(status),
                            ButtonTone::Quiet,
                            true,
                            move |_, cx| {
                                entity.update(cx, |this, cx| {
                                    this.set_issue_status(number, status, version, cx);
                                });
                            },
                        )
                    })
                    .collect::<Vec<_>>();
                    let session_action = self.active_session.as_ref().map(|_| {
                        let number = issue.number;
                        let entity = entity.clone();
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
                    let back = entity.clone();
                    div()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.md)
                        .when(narrow, |detail| {
                            detail.child(button(
                                "workgraph-detail-back",
                                "Back to issues",
                                ButtonTone::Quiet,
                                true,
                                move |_, cx| {
                                    back.update(cx, |this, cx| this.clear_selection(cx));
                                },
                            ))
                        })
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
                                )
                                .child(edit_action),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(THEME.space.xs)
                                .child(
                                    div()
                                        .text_size(THEME.type_scale.caption)
                                        .text_color(THEME.colors.subtle)
                                        .child("CHANGE STATUS"),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap(THEME.space.xs)
                                        .children(status_actions),
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
                        .child(dependency_issue_section(
                            issue.number,
                            issue.version,
                            dependencies,
                            entity.clone(),
                        ))
                        .children(dependency_action)
                        .child(related_issue_section(
                            "UNBLOCKS",
                            "No dependent issues.",
                            dependents,
                            entity.clone(),
                        ))
                        .child(detail_section(
                            "NOTES",
                            if notes.is_empty() {
                                "No progress notes yet.".into()
                            } else {
                                notes
                                    .iter()
                                    .map(|note| note.body.as_str())
                                    .collect::<Vec<_>>()
                                    .join("\n\n")
                            },
                        ))
                        .children(note_action)
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.search.is_none() {
            let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search issues"));
            self.subscriptions.push(cx.subscribe_in(
                &search,
                window,
                |_, _, _: &InputEvent, _, cx| cx.notify(),
            ));
            self.search = Some(search);
        }
        if self.create_title.is_none() {
            self.create_title = Some(
                cx.new(|cx| InputState::new(window, cx).placeholder("What needs to be done?")),
            );
        }
        if self.create_body.is_none() {
            self.create_body = Some(cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(4, 10)
                    .submit_on_enter(false)
                    .placeholder("Context, expected result, and useful constraints")
            }));
        }
        if self.edit_title.is_none() {
            self.edit_title =
                Some(cx.new(|cx| InputState::new(window, cx).placeholder("Issue title")));
        }
        if self.edit_body.is_none() {
            self.edit_body = Some(cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(4, 10)
                    .submit_on_enter(false)
                    .placeholder("Issue description")
            }));
        }
        if self.edit_priority.is_none() {
            self.edit_priority =
                Some(cx.new(|cx| InputState::new(window, cx).placeholder("Priority")));
        }
        if self.dependency.is_none() {
            self.dependency =
                Some(cx.new(|cx| InputState::new(window, cx).placeholder("Issue number")));
        }
        if self.note.is_none() {
            self.note = Some(cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(2, 5)
                    .submit_on_enter(false)
                    .placeholder("Add a progress or handoff note")
            }));
        }
        if self.note_issue != self.selected {
            if let Some(note) = &self.note {
                note.update(cx, |input, cx| {
                    input.set_value(String::new(), window, cx);
                });
            }
            self.note_issue = self.selected;
        }
        let entity = cx.entity();
        let layout = board_layout_mode(window.viewport_size().width);
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
                    let search = self
                        .search
                        .as_ref()
                        .map(|input| input.read(cx).value().to_string())
                        .unwrap_or_default();
                    let groups = matching_project_groups(data, self.filter, &search);
                    let matching_count = groups.iter().map(|group| group.rows.len()).sum::<usize>();
                    let mode = self.mode;
                    let kanban = entity.clone();
                    let graph = entity.clone();
                    let create = entity.clone();
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
                                                    "{matching_count} shown  ·  {active_count} active  ·  {blocked_count} need attention  ·  {} total",
                                                    data.issues.len()
                                                )),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(THEME.space.xs)
                                        .child(
                                            Input::new(
                                                self.search
                                                    .as_ref()
                                                    .expect("workgraph search initialized"),
                                            )
                                            .w(px(220.0)),
                                        )
                                        .child(button(
                                            "workgraph-create-open",
                                            "New issue",
                                            ButtonTone::Neutral,
                                            true,
                                            move |_, cx| {
                                                create.update(cx, |this, cx| this.start_create(cx));
                                            },
                                        ))
                                        .children([BoardMode::Kanban, BoardMode::Graph].map(|item| {
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
                                        })),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .flex()
                                .when(layout != BoardLayoutMode::Narrow, |board| {
                                    board.child(self.render_filter_rail(entity.clone(), data))
                                })
                                .when(
                                    !self.creating
                                        && (layout != BoardLayoutMode::Narrow
                                            || self.selected.is_none()),
                                    |board| {
                                        board.child(if groups.is_empty() {
                                            feedback(
                                                "workgraph-empty",
                                                self.filter.empty_message(),
                                                FeedbackTone::Info,
                                            )
                                            .into_any_element()
                                        } else if self.mode == BoardMode::Graph {
                                            self.render_graph(entity.clone(), data)
                                                .into_any_element()
                                        } else {
                                            self.render_groups(entity.clone(), groups)
                                                .into_any_element()
                                        })
                                    },
                                )
                                .when(self.creating, |board| {
                                    board.child(render_create(
                                        self.create_title.as_ref().expect("create title initialized"),
                                        self.create_body.as_ref().expect("create body initialized"),
                                        entity.clone(),
                                        layout,
                                    ))
                                })
                                .when(
                                    !self.creating
                                        && (layout != BoardLayoutMode::Narrow
                                            || self.selected.is_some()),
                                    |board| {
                                        board.child(self.render_detail(entity, data, layout))
                                    },
                                ),
                        )
                        .into_any_element()
                }
            })
    }
}
