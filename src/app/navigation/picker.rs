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
    IndexPath,
    input::Backspace,
    list::{List, ListEvent, ListState as ComponentListState},
};

use super::FarcasterApp;
use crate::{
    app::ui::assets::AppIcon,
    app::ui::keybindings::application_key,
    app::ui::primitives::{ButtonTone, PickerDelegate, PickerRow, button, modal},
    app::ui::theme::THEME,
    sessions::SessionSummary,
};

pub(crate) const PICKER_KEY_CONTEXT: &str = "PiPicker";

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
    fn label(&self) -> &str {
        match self {
            Self::Actions => "Actions",
            Self::Projects(ProjectPickerIntent::MoveSession { .. }) => "Move session",
            Self::Projects(_) => "Choose project",
            Self::Sessions => "Find session",
            Self::Sandbox => "Set sandbox",
            Self::Harnesses => "Set harness",
            Self::Providers => "Choose provider",
            Self::Models(_) => "Choose model",
            Self::Efforts(_) => "Choose reasoning effort",
            Self::ArchivedSessions => "Restore session",
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
            Self::Efforts(_) => "Search reasoning efforts…",
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
    SetHarness(String),
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
    pub(in crate::app) fn picker_focus(&self, cx: &gpui::App) -> Option<gpui::FocusHandle> {
        self.picker
            .as_ref()
            .map(|picker| picker.list.read(cx).focus_handle(cx))
    }

    pub(in crate::app) fn open_picker(
        &mut self,
        scope: PickerScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if scope == PickerScope::Harnesses && self.editable_draft_harness().is_none() {
            return;
        }
        if self
            .picker
            .as_ref()
            .is_some_and(|picker| picker.scope != scope && picker.has_ancestor(&scope))
        {
            let mut page = self.picker.take().expect("picker has history");
            while page.scope != scope {
                page = page.pop_previous().expect("requested ancestor exists");
            }
            page.list.update(cx, |list, cx| list.focus(window, cx));
            self.picker = Some(page);
            cx.notify();
            return;
        }
        self.cover_native_workspace_surface(cx);
        if self.picker.is_none() {
            let sheet_open = self.overlays.sessions
                || self.overlays.run
                || self.overlays.keybindings
                || self.overlays.settings;
            self.picker_return_focus = if sheet_open {
                self.sheet_return_focus
                    .clone()
                    .or_else(|| Some(self.chat_composer_focus(cx)))
            } else {
                window.focused(cx)
            };
            if sheet_open {
                self.overlays.sessions = false;
                self.overlays.run = false;
                self.overlays.keybindings = false;
                self.overlays.settings = false;
                self.overlays.pending_setup = false;
                self.sheet_return_focus = None;
            }
        }
        let (rows, commands) = self.picker_rows(scope.clone());
        let selected = configuration::selected_row(
            &rows,
            &commands,
            &self.snapshot,
            (scope == PickerScope::Harnesses).then(|| self.active_harness()),
        )
        .map(|row| IndexPath {
            row,
            ..Default::default()
        });
        let (delegate, handles) = PickerDelegate::new(rows);
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
                list.scroll_handle()
                    .scroll_to_item(selected.row, gpui::ScrollStrategy::Center);
            }
            list.focus(window, cx);
        });
        let previous = self
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
        self.picker = Some(PickerState {
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
        let Some(picker) = self.picker.take() else {
            return;
        };
        let target = self.picker_return_focus.take();
        let focus = picker.list.read(cx).focus_handle(cx);
        self.restore_overlay_focus(target, &focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
    }

    pub(in crate::app) fn picker_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.picker.as_ref() else {
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
        if self.picker.is_none() {
            return;
        }
        if let Some(previous) = self.picker.as_mut().and_then(PickerState::pop_previous) {
            previous.list.update(cx, |list, cx| list.focus(window, cx));
            self.picker = Some(previous);
            cx.notify();
        } else if matches!(
            self.picker.as_ref().map(|picker| &picker.scope),
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
        let picker = self.picker.as_ref()?;
        let list = picker.list.clone();
        let focus = list.read(cx).focus_handle(cx);
        let back_label = match picker.previous.as_ref().map(|page| &page.scope) {
            Some(PickerScope::Models(_)) => "Back to models",
            Some(PickerScope::Providers) => "Back to providers",
            _ if picker.scope == PickerScope::Actions => "Close",
            _ => "Back to actions",
        };
        let back = entity.clone();
        let close = entity;
        Some(
            modal(
                "command-picker",
                picker.scope.label(),
                &focus,
                PICKER_KEY_CONTEXT,
                move |window, cx| {
                    let _ = close.update(cx, |this, cx| this.close_picker(window, cx));
                },
                |surface| {
                    surface
                        .w(gpui::px(640.0))
                        .max_w_full()
                        .overflow_hidden()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(div().px(THEME.space.md).py(THEME.space.sm).child(button(
                                    "picker-back",
                                    back_label,
                                    ButtonTone::Quiet,
                                    true,
                                    move |window, cx| {
                                        let _ = back.update(cx, |this, cx| {
                                            this.picker_navigate_back(window, cx)
                                        });
                                    },
                                )))
                                .child(
                                    List::new(&list)
                                        .search_placeholder(picker.scope.placeholder())
                                        .max_h(gpui::px(480.0)),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap(THEME.space.md)
                                        .border_t(THEME.border)
                                        .border_color(THEME.colors.border)
                                        .px(THEME.space.md)
                                        .py(THEME.space.sm)
                                        .text_size(THEME.type_scale.caption)
                                        .text_color(THEME.colors.subtle)
                                        .child("↑ ↓ / Tab ⇧Tab Move")
                                        .child("Enter Choose")
                                        .child("Alt+← Back")
                                        .child("Esc Close"),
                                )
                                .children((picker.scope == PickerScope::Actions).then(|| {
                                    div()
                                        .px(THEME.space.md)
                                        .pb(THEME.space.sm)
                                        .text_size(THEME.type_scale.caption)
                                        .text_color(THEME.colors.subtle)
                                        .child(format!(
                                            "Open actions: Ctrl+G then Space, or {}",
                                            if cfg!(target_os = "macos") {
                                                "Cmd+Shift+P"
                                            } else {
                                                "Ctrl+Shift+P"
                                            }
                                        ))
                                })),
                        )
                },
            )
            .into_any_element(),
        )
    }

    fn execute_picker_row(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(command) = self
            .picker
            .as_ref()
            .and_then(|picker| picker.commands.get(id))
            .cloned()
        else {
            return;
        };
        match command {
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
            PickerCommand::SetSandbox(mode) => {
                self.close_picker(window, cx);
                self.set_access_mode(mode, cx);
            }
            PickerCommand::SetRuntime { model, effort } => {
                self.close_picker(window, cx);
                self.select_model(&model, cx);
                if let Some(effort) = effort {
                    self.set_thinking_level(effort, cx);
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
            PickerCommand::OpenSettings => {
                self.close_picker(window, cx);
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
                self.composer_focus.focus(window, cx);
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
        let include_shortcuts = scope == PickerScope::Actions;
        let mut rows = match scope {
            PickerScope::Actions => vec![
                picker_row(
                    &mut commands,
                    "action:code-task",
                    PickerCommand::StartCodeTask,
                    AppIcon::Code,
                    "Start task from selected code…",
                    None,
                    Some("ctrl-g shift-n".into()),
                    "neovim editor selection background new chat",
                )
                .disabled(self.surface != crate::app::AppSurface::Editor),
                picker_row(
                    &mut commands,
                    "action:harness",
                    PickerCommand::OpenScope(PickerScope::Harnesses),
                    AppIcon::Code,
                    "Set harness…",
                    None,
                    Some(application_key("shift-h")),
                    "backend agent",
                )
                .disabled(self.editable_draft_harness().is_none()),
                picker_row(
                    &mut commands,
                    "action:sandbox",
                    PickerCommand::OpenScope(PickerScope::Sandbox),
                    AppIcon::Shield,
                    "Set sandbox…",
                    None,
                    Some(application_key("shift-s")),
                    "access permissions approval",
                ),
                picker_row(
                    &mut commands,
                    "action:runtime",
                    PickerCommand::OpenScope(PickerScope::Providers),
                    AppIcon::List,
                    "Set provider/model/effort…",
                    None,
                    Some(application_key("shift-m")),
                    "model reasoning thinking runtime",
                ),
                picker_row(
                    &mut commands,
                    "action:restore",
                    PickerCommand::OpenScope(PickerScope::ArchivedSessions),
                    AppIcon::ChatCircle,
                    "Restore session…",
                    None,
                    Some(application_key("shift-a")),
                    "unarchive archived thread",
                ),
                picker_row(
                    &mut commands,
                    "action:new-session",
                    PickerCommand::OpenProjects(ProjectPickerIntent::NewSession),
                    AppIcon::Plus,
                    "New session…",
                    None,
                    Some(application_key("n")),
                    "project thread",
                ),
                picker_row(
                    &mut commands,
                    "action:find-session",
                    PickerCommand::OpenSessions,
                    AppIcon::MagnifyingGlass,
                    "Find session",
                    None,
                    None,
                    "open resume thread",
                ),
                picker_row(
                    &mut commands,
                    "action:add-project",
                    PickerCommand::AddProject(None),
                    AppIcon::FolderPlus,
                    "Add project",
                    None,
                    Some(application_key("shift-n")),
                    "folder checkout",
                ),
                picker_row(
                    &mut commands,
                    "action:project-work",
                    PickerCommand::OpenWorkGraph,
                    AppIcon::List,
                    "Project work",
                    None,
                    Some(application_key("shift-i")),
                    "issues tasks",
                ),
                picker_row(
                    &mut commands,
                    "action:import-sessions",
                    PickerCommand::ImportSessions,
                    AppIcon::Binoculars,
                    "Import sessions…",
                    None,
                    None,
                    "import discover catalog disk harness",
                ),
                picker_row(
                    &mut commands,
                    "action:settings",
                    PickerCommand::OpenSettings,
                    AppIcon::Key,
                    "Settings",
                    None,
                    None,
                    "configuration preferences keybindings modifier",
                ),
            ],
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
                .then_some(self.project.as_path());
                let mut rows =
                    ordered_projects(&self.projects, &self.all_sessions, open_session_project)
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
                    .all_sessions
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
                        self.drafts
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
        if include_shortcuts {
            let listed = rows
                .iter()
                .filter_map(|row| row.shortcut.clone())
                .collect::<HashSet<_>>();
            let mut actions = HashSet::new();
            for shortcut in crate::app::ui::keybindings::registry()
                .into_iter()
                .filter(|shortcut| shortcut.show_in_picker)
            {
                if listed.contains(&shortcut.keystroke) {
                    continue;
                }
                let action = shortcut.binding.action().name();
                if !actions.insert(action) {
                    continue;
                }
                rows.push(picker_row(
                    &mut commands,
                    &format!("shortcut:{}", shortcut.keystroke),
                    PickerCommand::Action(action),
                    AppIcon::Key,
                    if shortcut.keystroke == application_key("w") {
                        "Close surface or draft; archive session"
                    } else {
                        shortcut.label
                    },
                    Some(shortcut.section.to_owned()),
                    Some(shortcut.keystroke),
                    shortcut.section,
                ));
            }
        }
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
    commands.insert(id.to_owned(), command);
    PickerRow::new(id, icon, label, detail, shortcut, keywords)
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
