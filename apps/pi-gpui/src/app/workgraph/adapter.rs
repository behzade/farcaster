//! GPUI and SQLite adapters for the workgraph board.

mod detail;

use std::path::PathBuf;

use super::{
    components::{render_create, render_filter_rail, render_graph, render_groups},
    contract::{BoardFilter, BoardLoadState, BoardMode},
    core::{adjacent_issue_number, filter_count, matching_project_groups},
    layout::{BoardLayoutMode, DETAIL_WIDTH, board_toolbar_stacks, surface_board_layout},
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
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    Task, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::input::{Input, InputEvent, InputState, TextareaState};

pub(crate) const WORKGRAPH_KEY_CONTEXT: &str = "PiWorkGraph";
pub(crate) const WORKGRAPH_NAV_KEY_CONTEXT: &str = "PiWorkGraph && !Input";

pub(crate) struct WorkGraphBoardView {
    database: PathBuf,
    project: PathBuf,
    state: BoardLoadState,
    focus: FocusHandle,
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
            focus: cx.focus_handle(),
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

    pub(super) fn set_filter(&mut self, filter: BoardFilter, cx: &mut Context<Self>) {
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

    pub(crate) fn select_issue(&mut self, number: u64, cx: &mut Context<Self>) {
        if replace_work_state(
            &mut self.selected,
            &mut self.creating,
            &mut self.editing,
            Some(number),
            false,
            None,
        ) {
            cx.notify();
        }
    }

    pub(crate) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
    }

    pub(crate) fn focus_search(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = &self.search {
            search.read(cx).focus_handle(cx).focus(window, cx);
        } else {
            self.focus.focus(window, cx);
        }
    }

    pub(crate) fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let BoardLoadState::Ready(data) = &self.state else {
            return;
        };
        let search = self
            .search
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();
        let numbers = matching_project_groups(data, self.filter, &search)
            .into_iter()
            .flat_map(|group| group.rows.into_iter().map(|row| row.issue.number))
            .collect::<Vec<_>>();
        if let Some(number) = adjacent_issue_number(&numbers, self.selected, delta) {
            self.select_issue(number, cx);
        }
    }

    pub(crate) fn dismiss_work_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.take().is_some() || std::mem::take(&mut self.creating) {
            cx.notify();
            return;
        }
        if let Some(search) = &self.search
            && !search.read(cx).value().is_empty()
        {
            search.update(cx, |input, cx| {
                input.set_value(String::new(), window, cx);
            });
            return;
        }
        self.clear_selection(cx);
    }

    pub(super) fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if replace_work_state(
            &mut self.selected,
            &mut self.creating,
            &mut self.editing,
            None,
            false,
            None,
        ) {
            cx.notify();
        }
    }

    pub(crate) fn start_create(&mut self, cx: &mut Context<Self>) {
        if replace_work_state(
            &mut self.selected,
            &mut self.creating,
            &mut self.editing,
            None,
            true,
            None,
        ) {
            cx.notify();
        }
    }

    pub(super) fn set_editing(&mut self, number: Option<u64>, cx: &mut Context<Self>) {
        if self.editing != number {
            self.editing = number;
            cx.notify();
        }
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
}

fn replace_work_state(
    selected: &mut Option<u64>,
    creating: &mut bool,
    editing: &mut Option<u64>,
    next_selected: Option<u64>,
    next_creating: bool,
    next_editing: Option<u64>,
) -> bool {
    if (*selected, *creating, *editing) == (next_selected, next_creating, next_editing) {
        return false;
    }
    *selected = next_selected;
    *creating = next_creating;
    *editing = next_editing;
    true
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
        let viewport_width = window.viewport_size().width;
        let shell_layout = crate::layout::layout_mode(viewport_width);
        let mut board_width = if crate::layout::shows_left_inline(shell_layout) {
            viewport_width - THEME.layout.session_rail
        } else {
            viewport_width
        };
        if matches!(shell_layout, crate::layout::LayoutMode::Wide) {
            board_width -= px(DETAIL_WIDTH);
        }
        let external_detail = matches!(shell_layout, crate::layout::LayoutMode::Wide);
        let layout = surface_board_layout(board_width, external_detail);
        div()
            .size_full()
            .track_focus(&self.focus)
            .key_context(WORKGRAPH_KEY_CONTEXT)
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
                    let refresh = entity.clone();
                    let active_count = filter_count(data, BoardFilter::Active);
                    let blocked_count = filter_count(data, BoardFilter::Blocked);
                    let compact_toolbar = board_toolbar_stacks(layout);
                    div()
                        .size_full()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .h(px(if compact_toolbar { 92.0 } else { 52.0 }))
                                .flex_none()
                                .px(THEME.space.md)
                                .flex()
                                .when(compact_toolbar, |toolbar| {
                                    toolbar
                                        .flex_col()
                                        .items_stretch()
                                        .justify_center()
                                        .gap(THEME.space.xs)
                                })
                                .when(!compact_toolbar, |toolbar| {
                                    toolbar.items_center().justify_between()
                                })
                                .border_b(THEME.border)
                                .border_color(THEME.colors.border)
                                .bg(THEME.colors.panel)
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap(THEME.space.xs)
                                        .child(
                                            div()
                                                .text_size(THEME.type_scale.body)
                                                .font_weight(FontWeight::SEMIBOLD)
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
                                        .min_w_0()
                                        .when(compact_toolbar, |controls| controls.w_full())
                                        .flex()
                                        .items_center()
                                        .gap(THEME.space.xs)
                                        .child(
                                            Input::new(
                                                self.search
                                                    .as_ref()
                                                    .expect("workgraph search initialized"),
                                            )
                                            .when(compact_toolbar, |search| {
                                                search.flex_1().min_w_0()
                                            })
                                            .when(!compact_toolbar, |search| search.w(px(220.0))),
                                        )
                                        .child(button(
                                            "workgraph-refresh",
                                            "Refresh",
                                            ButtonTone::Quiet,
                                            true,
                                            move |_, cx| {
                                                refresh.update(cx, |this, cx| this.refresh(cx));
                                            },
                                        ))
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
                                    board.child(render_filter_rail(
                                        self.filter,
                                        entity.clone(),
                                        data,
                                    ))
                                })
                                .when(
                                    !self.creating
                                        && (external_detail
                                            || layout != BoardLayoutMode::Narrow
                                            || self.selected.is_none()),
                                    |board| {
                                        board.child(if groups.is_empty() {
                                            div()
                                                .id("workgraph-empty")
                                                .flex_1()
                                                .min_w_0()
                                                .h_full()
                                                .flex()
                                                .flex_col()
                                                .items_center()
                                                .justify_center()
                                                .gap(THEME.space.xs)
                                                .px(THEME.space.md)
                                                .child(
                                                    div()
                                                        .text_size(THEME.type_scale.body)
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(THEME.colors.muted)
                                                        .child(self.filter.empty_message()),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(THEME.type_scale.caption)
                                                        .text_color(THEME.colors.subtle)
                                                        .child("Choose another work state or create an issue."),
                                                )
                                                .into_any_element()
                                        } else if self.mode == BoardMode::Graph {
                                            render_graph(
                                                self.selected,
                                                entity.clone(),
                                                data,
                                            )
                                            .into_any_element()
                                        } else {
                                            render_groups(
                                                self.selected,
                                                self.active_session
                                                    .as_ref()
                                                    .map(|(session_id, _)| session_id.as_str()),
                                                entity.clone(),
                                                groups,
                                                data,
                                            )
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
                                        && !external_detail
                                        && (layout != BoardLayoutMode::Narrow
                                            || self.selected.is_some()),
                                    |board| {
                                        board.child(self.render_detail(
                                            entity, data, layout, false,
                                        ))
                                    },
                                ),
                        )
                        .into_any_element()
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::replace_work_state;

    #[test]
    fn identical_work_state_does_not_request_a_rerender() {
        let mut selected = Some(7);
        let mut creating = false;
        let mut editing = None;

        assert!(!replace_work_state(
            &mut selected,
            &mut creating,
            &mut editing,
            Some(7),
            false,
            None,
        ));
        assert!(replace_work_state(
            &mut selected,
            &mut creating,
            &mut editing,
            None,
            true,
            None,
        ));
        assert!(!replace_work_state(
            &mut selected,
            &mut creating,
            &mut editing,
            None,
            true,
            None,
        ));
    }
}
