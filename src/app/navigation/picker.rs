use crate::agents::Backend;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    time::{Duration, UNIX_EPOCH},
};

use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement as _, ParentElement as _,
    Styled as _, Subscription, WeakEntity, Window, div,
};
use gpui_component::{
    input::Backspace,
    list::{List, ListEvent, ListState as ComponentListState},
};

use super::FarcasterApp;
use crate::{
    app::ui::assets::AppIcon,
    app::ui::keybindings::application_key,
    app::ui::primitives::{ButtonTone, PickerDelegate, PickerRow, button, icon_button, modal},
    app::ui::theme::theme,
    runtime::RuntimeCommand,
    sessions::SessionSummary,
};

pub(crate) const PICKER_KEY_CONTEXT: &str = "PiPicker";

mod actions;
mod configuration;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProjectPickerIntent {
    NewSession,
    NewSessionInFolder(u64),
    ChangeDraft,
    MoveSession {
        path: PathBuf,
        source_project: PathBuf,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PickerScope {
    Actions,
    Projects(ProjectPickerIntent),
    Sessions,
    Sandbox,
    Harnesses,
    Providers,
    Models(String),
    Efforts(crate::protocol::Model),
    ArchivedSessions,
}

impl PickerScope {
    fn title(&self) -> String {
        match self {
            Self::Actions => "Actions".into(),
            Self::Projects(ProjectPickerIntent::MoveSession { .. }) => "Move session".into(),
            Self::Projects(_) => "Choose project".into(),
            Self::Sessions => "Find session".into(),
            Self::Sandbox => "Sandbox".into(),
            Self::Harnesses => "Harnesses".into(),
            Self::Providers => "Providers".into(),
            Self::Models(provider) => format!("Models · {provider}"),
            Self::Efforts(model) => format!("Effort · {}", model.name),
            Self::ArchivedSessions => "Restore session".into(),
        }
    }

    fn placeholder(&self) -> &'static str {
        match self {
            Self::Actions => "Search actions…",
            Self::Projects(_) => "Search projects…",
            Self::Sessions => "Search sessions…",
            Self::Sandbox => "Search sandbox modes…",
            Self::Harnesses => "Search harnesses…",
            Self::Providers => "Search providers…",
            Self::Models(_) => "Search models…",
            Self::Efforts(_) => "Search model presets…",
            Self::ArchivedSessions => "Search archived sessions…",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PickerCommand {
    Action(&'static str),
    OpenProjects(ProjectPickerIntent),
    OpenSessions,
    StartCodeTask,
    AddProject(Option<ProjectPickerIntent>),
    OpenWorkGraph,
    OpenSettings,
    OpenThemes,
    ImportSessions,
    NewSession {
        project: PathBuf,
        folder: Option<u64>,
    },
    ChangeDraftProject(PathBuf),
    MoveSession {
        path: PathBuf,
        project: PathBuf,
    },
    SelectSession {
        path: PathBuf,
        project: PathBuf,
    },
    ResumeDraft {
        id: String,
        project: PathBuf,
    },
    OpenScope(PickerScope),
    SetSandbox(crate::runtime::HarnessAccessMode),
    SetHarness(Backend),
    SetHarnessProfile(Backend, String),
    RetryConfiguration,
    SetRuntime {
        model: crate::protocol::Model,
        effort: Option<String>,
    },
    RestoreSession(PathBuf),
}

pub(in crate::app) struct PickerState {
    pub(in crate::app) scope: PickerScope,
    list: Entity<ComponentListState<PickerDelegate>>,
    commands: HashMap<String, PickerCommand>,
    query: Rc<RefCell<String>>,
    _subscription: Subscription,
    previous: Option<Box<PickerState>>,
}

impl PickerState {
    fn pop_previous(&mut self) -> Option<Self> {
        self.previous.take().map(|previous| *previous)
    }

    fn has_ancestor(&self, scope: &PickerScope) -> bool {
        std::iter::successors(self.previous.as_deref(), |page| page.previous.as_deref())
            .any(|page| &page.scope == scope)
    }
}

impl FarcasterApp {
    pub(in crate::app) fn refresh_configuration_picker(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self
            .navigation
            .picker
            .as_ref()
            .map(|picker| picker.scope.clone())
        else {
            return;
        };
        if !matches!(
            scope,
            PickerScope::Providers | PickerScope::Models(_) | PickerScope::Efforts(_)
        ) {
            return;
        }
        let (rows, commands) = self.picker_rows(scope);
        let picker = self.navigation.picker.as_mut().expect("picker is open");
        picker.commands = commands;
        let list = picker.list.clone();
        let selected = list.update(cx, |list, cx| {
            let selected = list.delegate_mut().replace_rows(rows);
            cx.notify();
            selected
        });
        cx.defer(move |cx| {
            if let Some(window_handle) = cx.active_window() {
                let _ = cx.update_window(window_handle, |_, window, cx| {
                    list.update(cx, |list, cx| list.set_selected_index(selected, window, cx));
                });
            }
        });
    }

    pub(in crate::app) fn picker_focus(&self, cx: &gpui::App) -> Option<gpui::FocusHandle> {
        self.navigation
            .picker
            .as_ref()
            .map(|picker| picker.list.read(cx).focus_handle(cx))
    }

    pub(in crate::app) fn open_picker(
        &mut self,
        scope: PickerScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.runtime_picker.open && self.workspace.runtime_picker.worker.is_some() {
            self.set_runtime_picker_open(false, window, cx);
        }
        if scope == PickerScope::Harnesses && self.editable_draft_harness().is_none() {
            return;
        }
        if self
            .navigation
            .picker
            .as_ref()
            .is_some_and(|picker| picker.scope != scope && picker.has_ancestor(&scope))
        {
            let mut page = self.navigation.picker.take().expect("picker has history");
            while page.scope != scope {
                page = page.pop_previous().expect("requested ancestor exists");
            }
            page.list.update(cx, |list, cx| list.focus(window, cx));
            self.navigation.picker = Some(page);
            cx.notify();
            return;
        }
        self.cover_native_workspace_surface(cx);
        if self.navigation.picker.is_none() {
            let sheet_open = self.overlays.view.sessions
                || self.overlays.view.run
                || self.overlays.view.keybindings
                || self.overlays.view.settings;
            self.navigation.picker_return_focus = if sheet_open {
                self.overlays
                    .sheet_return_focus
                    .clone()
                    .or_else(|| Some(self.chat_composer_focus(cx)))
            } else {
                window.focused(cx)
            };
            if sheet_open {
                self.overlays.view.sessions = false;
                self.overlays.view.run = false;
                self.overlays.view.keybindings = false;
                self.overlays.view.settings = false;
                self.overlays.view.pending_setup = false;
                self.overlays.sheet_return_focus = None;
            }
        }
        let (rows, commands) = self.picker_rows(scope.clone());
        let active_profile_id = (scope == PickerScope::Harnesses)
            .then(|| self.active_profile_id())
            .flatten();
        let selected = configuration::selected_row(
            &rows,
            &commands,
            &self.snapshot,
            (scope == PickerScope::Harnesses)
                .then(|| self.active_harness())
                .flatten(),
            active_profile_id.as_deref(),
        );
        let (delegate, handles) = PickerDelegate::new(rows);
        let selected = delegate.preferred_index(selected);
        let confirmed_id = handles.confirmed_id;
        let query = handles.query;
        let list = cx.new(|cx| ComponentListState::new(delegate, window, cx).searchable(true));
        let subscription =
            cx.subscribe_in(
                &list,
                window,
                move |_this, _, event, window, cx| match event {
                    ListEvent::Confirm(_) => {
                        if let Some(id) = confirmed_id.borrow_mut().take() {
                            cx.defer_in(window, move |this, window, cx| {
                                this.execute_picker_row(&id, window, cx);
                            });
                        }
                        cx.stop_propagation();
                    }
                    ListEvent::Cancel => {
                        cx.defer_in(window, |this, window, cx| {
                            this.close_picker(window, cx);
                        });
                        cx.stop_propagation();
                    }
                    ListEvent::Select(_) => {}
                },
            );
        list.update(cx, |list, cx| {
            list.set_selected_index(selected, window, cx);
            if let Some(selected) = selected {
                list.scroll_to_item(selected, gpui::ScrollStrategy::Center, window, cx);
            }
            list.focus(window, cx);
        });
        let previous = self
            .navigation
            .picker
            .take()
            .filter(|_| scope != PickerScope::Actions)
            .and_then(|page| {
                if page.scope == scope {
                    page.previous
                } else {
                    Some(Box::new(page))
                }
            });
        self.navigation.picker = Some(PickerState {
            scope,
            list,
            commands,
            query,
            _subscription: subscription,
            previous,
        });
        cx.notify();
    }

    pub(in crate::app) fn close_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.navigation.picker.take() else {
            return;
        };
        let target = self.navigation.picker_return_focus.take();
        let focus = picker.list.read(cx).focus_handle(cx);
        self.restore_overlay_focus(target, &focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
    }

    pub(in crate::app) fn picker_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.navigation.picker.as_ref() else {
            return;
        };
        if !picker.query.borrow().is_empty() {
            window.dispatch_action(Box::new(Backspace), cx);
            cx.stop_propagation();
            return;
        }
        self.picker_navigate_back(window, cx);
    }

    pub(in crate::app) fn picker_navigate_back(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.navigation.picker.is_none() {
            return;
        }
        if let Some(previous) = self
            .navigation
            .picker
            .as_mut()
            .and_then(PickerState::pop_previous)
        {
            previous.list.update(cx, |list, cx| list.focus(window, cx));
            self.navigation.picker = Some(previous);
            cx.notify();
        } else if matches!(
            self.navigation.picker.as_ref().map(|picker| &picker.scope),
            Some(PickerScope::Actions)
        ) {
            self.close_picker(window, cx);
        } else {
            self.open_picker(PickerScope::Actions, window, cx);
        }
        cx.stop_propagation();
    }

    pub(in crate::app) fn render_picker(
        &self,
        entity: WeakEntity<Self>,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let picker = self.navigation.picker.as_ref()?;
        let list = picker.list.clone();
        let focus = list.read(cx).focus_handle(cx);
        let back_label = picker
            .previous
            .as_ref()
            .map(|page| format!("Back to {}", page.scope.title()))
            .unwrap_or_else(|| "Back to Actions".into());
        let is_root = picker.scope == PickerScope::Actions;
        let back = entity.clone();
        let close_button = entity.clone();
        let close = entity;
        Some(
            modal(
                "command-picker",
                picker.scope.title(),
                &focus,
                PICKER_KEY_CONTEXT,
                move |window, cx| {
                    let _ = close.update(cx, |this, cx| this.close_picker(window, cx));
                },
                |surface| {
                    surface
                        .w(theme().size(640.0))
                        .max_w_full()
                        .overflow_hidden()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .px(theme().space.md)
                                        .py(theme().space.sm)
                                        .flex()
                                        .items_center()
                                        .gap(theme().space.sm)
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .text_ellipsis()
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .child(picker.scope.title()),
                                        )
                                        .children((!is_root).then(|| {
                                            button(
                                                "picker-back",
                                                back_label,
                                                ButtonTone::Quiet,
                                                true,
                                                move |window, cx| {
                                                    let _ = back.update(cx, |this, cx| {
                                                        this.picker_navigate_back(window, cx)
                                                    });
                                                },
                                            )
                                        }))
                                        .child(icon_button(
                                            "picker-close",
                                            AppIcon::X,
                                            "Close picker",
                                            ButtonTone::Quiet,
                                            move |window, cx| {
                                                let _ = close_button.update(cx, |this, cx| {
                                                    this.close_picker(window, cx)
                                                });
                                            },
                                        )),
                                )
                                .child(
                                    List::new(&list)
                                        .search_placeholder(picker.scope.placeholder())
                                        .h(theme().size(480.0))
                                        .max_h(theme().size(480.0)),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap(theme().space.md)
                                        .border_t(theme().border)
                                        .border_color(theme().colors.border)
                                        .px(theme().space.md)
                                        .py(theme().space.sm)
                                        .text_size(theme().type_scale.caption)
                                        .text_color(theme().colors.subtle)
                                        .child("↑ ↓ / Tab ⇧Tab Move")
                                        .child("Enter Choose")
                                        .children((!is_root).then_some("Alt+← Back"))
                                        .child("Esc Close"),
                                ),
                        )
                },
            )
            .into_any_element(),
        )
    }

    fn execute_picker_row(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(command) = self
            .navigation
            .picker
            .as_ref()
            .and_then(|picker| picker.commands.get(id))
            .cloned()
        else {
            return;
        };
        match command {
            PickerCommand::RetryConfiguration => {
                if let Some(harness) = self.snapshot.harness {
                    self.send(
                        RuntimeCommand::LoadConfiguration {
                            harness,
                            project: self.snapshot.project.clone(),
                        },
                        cx,
                    );
                }
            }
            PickerCommand::StartCodeTask => {
                self.close_picker(window, cx);
                self.start_task_from_code(window, cx);
            }
            PickerCommand::Action(name) => {
                if let Some(shortcut) = crate::app::ui::keybindings::registry()
                    .into_iter()
                    .find(|shortcut| shortcut.binding.action().name() == name)
                {
                    self.close_picker(window, cx);
                    window.dispatch_action(shortcut.binding.action().boxed_clone(), cx);
                }
            }
            PickerCommand::OpenScope(PickerScope::Sandbox) => {
                window.dispatch_action(Box::new(crate::app::SetSandbox), cx);
            }
            PickerCommand::OpenScope(PickerScope::Providers) => {
                window.dispatch_action(Box::new(crate::app::SetRuntime), cx);
            }
            PickerCommand::OpenScope(PickerScope::ArchivedSessions) => {
                window.dispatch_action(Box::new(crate::app::RestoreSession), cx);
            }
            PickerCommand::OpenScope(scope) => self.open_picker(scope, window, cx),
            PickerCommand::SetHarness(harness) => {
                if self.editable_draft_harness().is_none()
                    || !crate::agents::backend_statuses()
                        .iter()
                        .any(|backend| backend.id == harness && backend.available)
                {
                    return;
                }
                self.close_picker(window, cx);
                self.change_draft_harness(harness, window, cx);
            }
            PickerCommand::SetHarnessProfile(harness, profile_id) => {
                if self.editable_draft_harness().is_none() {
                    return;
                }
                self.close_picker(window, cx);
                self.change_draft_harness_profile(harness, profile_id, window, cx);
            }
            PickerCommand::SetSandbox(mode) => {
                self.close_picker(window, cx);
                self.set_access_mode(mode, cx);
            }
            PickerCommand::SetRuntime { model, effort } => {
                if self.select_model_with_effort(&model, effort, true, window, cx) {
                    self.close_picker(window, cx);
                }
            }
            PickerCommand::RestoreSession(path) => {
                self.close_picker(window, cx);
                self.set_session_archived(path, false, cx);
            }
            PickerCommand::OpenProjects(intent) => {
                self.open_picker(PickerScope::Projects(intent), window, cx);
            }
            PickerCommand::OpenSessions => {
                self.open_picker(PickerScope::Sessions, window, cx);
            }
            PickerCommand::AddProject(None) => {
                window.dispatch_action(Box::new(crate::app::AddProject), cx);
            }
            PickerCommand::AddProject(intent) => {
                self.close_picker(window, cx);
                self.choose_project_folder(intent, window, cx);
            }
            PickerCommand::OpenWorkGraph => {
                self.close_picker(window, cx);
                self.open_workgraph_surface(window, cx);
            }
            PickerCommand::OpenSettings | PickerCommand::OpenThemes => {
                self.close_picker(window, cx);
                if command == PickerCommand::OpenThemes {
                    self.settings.tab = crate::app::workspace::SettingsTab::Appearance;
                    self.settings.themes.editing = false;
                }
                self.open_settings(window, cx);
            }
            PickerCommand::ImportSessions => {
                self.close_picker(window, cx);
                self.open_session_import(window, cx);
            }
            PickerCommand::NewSession { project, folder } => {
                self.close_picker(window, cx);
                self.new_session_with_folder(project, folder, window, cx);
            }
            PickerCommand::ChangeDraftProject(project) => {
                self.close_picker(window, cx);
                self.change_draft_project(project, window, cx);
                self.composer.focus.focus(window, cx);
            }
            PickerCommand::MoveSession { path, project } => {
                self.close_picker(window, cx);
                self.move_session(path, project, window, cx);
            }
            PickerCommand::SelectSession { path, project } => {
                self.close_picker(window, cx);
                self.select_session_and_focus(path, project, window, cx);
            }
            PickerCommand::ResumeDraft { id, project } => {
                self.close_picker(window, cx);
                self.resume_draft_and_focus(id, project, window, cx);
            }
        }
    }

    fn picker_rows(&self, scope: PickerScope) -> (Vec<PickerRow>, HashMap<String, PickerCommand>) {
        let mut commands = HashMap::new();
        let rows = match scope {
            PickerScope::Actions => return self.action_picker_rows(),
            PickerScope::Harnesses
            | PickerScope::Sandbox
            | PickerScope::Providers
            | PickerScope::Models(_)
            | PickerScope::Efforts(_)
            | PickerScope::ArchivedSessions => self.configuration_picker_rows(scope, &mut commands),
            PickerScope::Projects(intent) => {
                let open_session_project = (matches!(
                    intent,
                    ProjectPickerIntent::NewSession | ProjectPickerIntent::NewSessionInFolder(_)
                ) && self.snapshot.selected_session.is_some())
                .then_some(self.project.path.as_path());
                let mut rows = ordered_projects(
                    &self.project.registered,
                    &self.sessions.all,
                    open_session_project,
                )
                .into_iter()
                .filter(|project| project_is_available_for_intent(&intent, project))
                .enumerate()
                .map(|(index, project)| {
                    let command = match &intent {
                        ProjectPickerIntent::NewSession => PickerCommand::NewSession {
                            project: project.clone(),
                            folder: None,
                        },
                        ProjectPickerIntent::NewSessionInFolder(folder) => {
                            PickerCommand::NewSession {
                                project: project.clone(),
                                folder: Some(*folder),
                            }
                        }
                        ProjectPickerIntent::ChangeDraft => {
                            PickerCommand::ChangeDraftProject(project.clone())
                        }
                        ProjectPickerIntent::MoveSession { path, .. } => {
                            PickerCommand::MoveSession {
                                path: path.clone(),
                                project: project.clone(),
                            }
                        }
                    };
                    picker_row(
                        &mut commands,
                        &format!("project:{index}"),
                        command,
                        AppIcon::Folder,
                        &project_label(&project),
                        Some(project.display().to_string()),
                        None,
                        "project folder checkout",
                    )
                    .removable_project(project)
                })
                .collect::<Vec<_>>();
                rows.push(picker_row(
                    &mut commands,
                    "project:new",
                    PickerCommand::AddProject(Some(intent)),
                    AppIcon::FolderPlus,
                    "New project",
                    None,
                    None,
                    "add choose folder checkout",
                ));
                rows
            }
            PickerScope::Sessions => {
                let mut entries = self
                    .sessions
                    .all
                    .iter()
                    .filter(|session| session.parent_session.is_none())
                    .map(|session| {
                        (
                            session.modified,
                            session.title.clone(),
                            session.project.clone(),
                            Some((session.path.clone(), session.search_text().to_owned())),
                            None,
                        )
                    })
                    .chain(
                        self.sessions
                            .drafts
                            .iter()
                            .filter(|draft| draft.session_path.is_none())
                            .map(|draft| {
                                (
                                    UNIX_EPOCH + Duration::from_millis(draft.created_ms),
                                    draft.title.clone().unwrap_or_else(|| "New session".into()),
                                    draft.project.clone(),
                                    None,
                                    Some(draft.id.clone()),
                                )
                            }),
                    )
                    .collect::<Vec<_>>();
                entries.sort_by_key(|entry| std::cmp::Reverse(entry.0));
                entries
                    .into_iter()
                    .enumerate()
                    .map(|(index, (_, title, project, session, draft_id))| {
                        let (command, keywords, icon) = if let Some((path, search)) = session {
                            (
                                PickerCommand::SelectSession {
                                    path,
                                    project: project.clone(),
                                },
                                search,
                                AppIcon::ChatCircle,
                            )
                        } else {
                            (
                                PickerCommand::ResumeDraft {
                                    id: draft_id.expect("draft entry has an id"),
                                    project: project.clone(),
                                },
                                "draft new session".into(),
                                AppIcon::ChatCircleDots,
                            )
                        };
                        picker_row(
                            &mut commands,
                            &format!("session:{index}"),
                            command,
                            icon,
                            &title,
                            Some(format!(
                                "{} · {}",
                                project_label(&project),
                                project.display()
                            )),
                            None,
                            &keywords,
                        )
                    })
                    .collect()
            }
        };
        (rows, commands)
    }
}

#[allow(clippy::too_many_arguments)]
fn picker_row(
    commands: &mut HashMap<String, PickerCommand>,
    id: &str,
    command: PickerCommand,
    icon: AppIcon,
    label: &str,
    detail: Option<String>,
    shortcut: Option<String>,
    keywords: &str,
) -> PickerRow {
    let opens_page = matches!(
        command,
        PickerCommand::OpenScope(_) | PickerCommand::OpenProjects(_) | PickerCommand::OpenSessions
    );
    commands.insert(id.to_owned(), command);
    PickerRow::new(id, icon, label, detail, shortcut, keywords).opens_page(opens_page)
}

fn ordered_projects(
    projects: &[PathBuf],
    sessions: &[SessionSummary],
    open_session_project: Option<&std::path::Path>,
) -> Vec<PathBuf> {
    let mut recency = HashMap::<PathBuf, Duration>::new();
    for session in sessions {
        let modified = session
            .modified
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);
        recency
            .entry(session.project.clone())
            .and_modify(|current| *current = (*current).max(modified))
            .or_insert(modified);
    }
    let mut ordered = sort_projects_by_recency(projects, &recency);
    if let Some(project) = open_session_project
        && let Some(index) = ordered.iter().position(|candidate| candidate == project)
    {
        ordered[..=index].rotate_right(1);
    }
    ordered
}

fn sort_projects_by_recency(
    projects: &[PathBuf],
    recency: &HashMap<PathBuf, Duration>,
) -> Vec<PathBuf> {
    let original_order = projects
        .iter()
        .enumerate()
        .map(|(index, project)| (project.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let mut ordered = projects
        .iter()
        .filter(|project| seen.insert((*project).clone()))
        .cloned()
        .collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        recency
            .get(right)
            .cmp(&recency.get(left))
            .then_with(|| original_order[left].cmp(&original_order[right]))
    });
    ordered
}

fn project_is_available_for_intent(intent: &ProjectPickerIntent, project: &PathBuf) -> bool {
    !matches!(
        intent,
        ProjectPickerIntent::MoveSession { source_project, .. } if source_project == project
    )
}

fn project_label(project: &std::path::Path) -> String {
    project
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| project.display().to_string(), str::to_owned)
}

#[cfg(test)]
#[path = "picker_tests.rs"]
mod tests;
