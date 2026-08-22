//! Pi command palette scopes and command dispatch.

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

use super::PiApp;
use crate::{
    assets::AppIcon,
    primitives::{PickerDelegate, PickerRow, modal},
    protocol::{ExtensionUiRequest, ExtensionUiResponse},
    runtime::RuntimeCommand,
    sessions::SessionSummary,
    theme::THEME,
};

pub(crate) const PICKER_KEY_CONTEXT: &str = "PiPicker";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProjectPickerIntent {
    NewSession,
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
    ProviderLogin {
        id: String,
        title: String,
        options: Vec<String>,
    },
}

impl PickerScope {
    fn label(&self) -> &str {
        match self {
            Self::Actions => "Actions",
            Self::Projects(ProjectPickerIntent::MoveSession { .. }) => "Move session",
            Self::Projects(_) => "Choose project",
            Self::Sessions => "Find session",
            Self::ProviderLogin { title, .. } => title,
        }
    }

    fn placeholder(&self) -> &'static str {
        match self {
            Self::Actions => "Search actions…",
            Self::Projects(_) => "Search projects…",
            Self::Sessions => "Search sessions…",
            Self::ProviderLogin { title, .. } if title == "Add provider" => "Search providers…",
            Self::ProviderLogin { .. } => "Search methods…",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PickerCommand {
    OpenProjects(ProjectPickerIntent),
    OpenSessions,
    AddProject(Option<ProjectPickerIntent>),
    OpenWorkGraph,
    NewSession(PathBuf),
    ChangeDraftProject(PathBuf),
    MoveSession { path: PathBuf, project: PathBuf },
    SelectSession { path: PathBuf, project: PathBuf },
    ResumeDraft { id: String, project: PathBuf },
    ProviderLogin { id: String, value: String },
}

pub(super) struct PickerState {
    scope: PickerScope,
    list: Entity<ComponentListState<PickerDelegate>>,
    commands: HashMap<String, PickerCommand>,
    query: Rc<RefCell<String>>,
    _subscription: Subscription,
}

impl PiApp {
    pub(super) fn open_picker(
        &mut self,
        scope: PickerScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.picker.is_none() {
            let sheet_open = self.sessions_sheet || self.run_sheet || self.keybindings_help;
            self.picker_return_focus = if sheet_open {
                Some(self.composer_focus.clone())
            } else {
                window.focused(cx)
            };
            if sheet_open {
                self.sessions_sheet = false;
                self.run_sheet = false;
                self.keybindings_help = false;
                self.pending_sheet_setup = false;
                self.sheet_return_focus = None;
            }
        }
        let (rows, commands) = self.picker_rows(scope.clone());
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
            list.set_selected_index(Some(IndexPath::default()), window, cx);
            list.focus(window, cx);
        });
        self.picker = Some(PickerState {
            scope,
            list,
            commands,
            query,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(super) fn close_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_picker(None, window, cx);
    }

    fn finish_picker(
        &mut self,
        response: Option<ExtensionUiResponse>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = self.picker.take() else {
            return;
        };
        if let PickerScope::ProviderLogin { id, .. } = picker.scope {
            self.send(RuntimeCommand::ExtensionResponse(response.unwrap_or(
                ExtensionUiResponse::Cancelled {
                    id,
                    cancelled: true,
                },
            )));
        }
        self.picker_return_focus
            .take()
            .unwrap_or_else(|| self.composer_focus.clone())
            .focus(window, cx);
        cx.notify();
    }

    pub(super) fn picker_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        if !picker.query.borrow().is_empty() {
            window.dispatch_action(Box::new(Backspace), cx);
            cx.stop_propagation();
            return;
        }
        if matches!(
            picker.scope,
            PickerScope::Actions | PickerScope::ProviderLogin { .. }
        ) {
            self.close_picker(window, cx);
        } else {
            self.open_picker(PickerScope::Actions, window, cx);
        }
        cx.stop_propagation();
    }

    pub(super) fn render_picker(
        &self,
        entity: WeakEntity<Self>,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let picker = self.picker.as_ref()?;
        let list = picker.list.clone();
        let focus = list.read(cx).focus_handle(cx);
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
                                .child(
                                    List::new(&list)
                                        .search_placeholder(picker.scope.placeholder())
                                        .max_h(gpui::px(480.0)),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap(THEME.space.md)
                                        .border_t(THEME.border)
                                        .border_color(THEME.colors.border)
                                        .px(THEME.space.md)
                                        .py(THEME.space.sm)
                                        .text_size(THEME.type_scale.caption)
                                        .text_color(THEME.colors.subtle)
                                        .child("Up/Down Navigate")
                                        .child("Enter Select")
                                        .child("Backspace Back")
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
            .picker
            .as_ref()
            .and_then(|picker| picker.commands.get(id))
            .cloned()
        else {
            return;
        };
        match command {
            PickerCommand::OpenProjects(intent) => {
                self.open_picker(PickerScope::Projects(intent), window, cx);
            }
            PickerCommand::OpenSessions => {
                self.open_picker(PickerScope::Sessions, window, cx);
            }
            PickerCommand::AddProject(intent) => {
                self.close_picker(window, cx);
                self.choose_project_folder(intent, window, cx);
            }
            PickerCommand::OpenWorkGraph => {
                self.close_picker(window, cx);
                self.open_workgraph_surface(window, cx);
            }
            PickerCommand::NewSession(project) => {
                self.close_picker(window, cx);
                self.new_session(project, window, cx);
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
                self.select_session(path, project, window, cx);
            }
            PickerCommand::ResumeDraft { id, project } => {
                self.close_picker(window, cx);
                self.resume_draft(id, project, window, cx);
            }
            PickerCommand::ProviderLogin { id, value } => {
                self.finish_picker(Some(ExtensionUiResponse::Value { id, value }), window, cx);
            }
        }
    }

    fn picker_rows(&self, scope: PickerScope) -> (Vec<PickerRow>, HashMap<String, PickerCommand>) {
        let mut commands = HashMap::new();
        let rows = match scope {
            PickerScope::Actions => vec![
                picker_row(
                    &mut commands,
                    "action:new-session",
                    PickerCommand::OpenProjects(ProjectPickerIntent::NewSession),
                    AppIcon::Plus,
                    "New session…",
                    None,
                    Some("cmd-n"),
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
                    Some("cmd-shift-n"),
                    "folder checkout",
                ),
                picker_row(
                    &mut commands,
                    "action:work-graph",
                    PickerCommand::OpenWorkGraph,
                    AppIcon::List,
                    "Work graph",
                    None,
                    None,
                    "issues tasks",
                ),
            ],
            PickerScope::Projects(intent) => {
                let mut rows = ordered_projects(&self.projects, &self.all_sessions)
                    .into_iter()
                    .filter(|project| project_is_available_for_intent(&intent, project))
                    .enumerate()
                    .map(|(index, project)| {
                        let command = match &intent {
                            ProjectPickerIntent::NewSession => {
                                PickerCommand::NewSession(project.clone())
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
                        let (command, keywords) = if let Some((path, search)) = session {
                            (
                                PickerCommand::SelectSession {
                                    path,
                                    project: project.clone(),
                                },
                                search,
                            )
                        } else {
                            (
                                PickerCommand::ResumeDraft {
                                    id: draft_id.expect("draft entry has an id"),
                                    project: project.clone(),
                                },
                                "draft new session".into(),
                            )
                        };
                        picker_row(
                            &mut commands,
                            &format!("session:{index}"),
                            command,
                            AppIcon::List,
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
            PickerScope::ProviderLogin {
                id,
                title: _,
                options,
            } => options
                .into_iter()
                .enumerate()
                .map(|(index, option)| {
                    picker_row(
                        &mut commands,
                        &format!("provider-login:{index}"),
                        PickerCommand::ProviderLogin {
                            id: id.clone(),
                            value: option.clone(),
                        },
                        AppIcon::Question,
                        &option,
                        None,
                        None,
                        &option,
                    )
                })
                .collect(),
        };
        (rows, commands)
    }
}

pub(super) fn provider_login_scope(request: &ExtensionUiRequest) -> Option<PickerScope> {
    let ExtensionUiRequest::Select {
        id, title, options, ..
    } = request
    else {
        return None;
    };
    matches!(
        title.as_str(),
        "Add provider" | "Select authentication method"
    )
    .then(|| PickerScope::ProviderLogin {
        id: id.clone(),
        title: title.clone(),
        options: options.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
fn picker_row(
    commands: &mut HashMap<String, PickerCommand>,
    id: &str,
    command: PickerCommand,
    icon: AppIcon,
    label: &str,
    detail: Option<String>,
    shortcut: Option<&'static str>,
    keywords: &str,
) -> PickerRow {
    commands.insert(id.to_owned(), command);
    PickerRow::new(id, icon, label, detail, shortcut, keywords)
}

fn ordered_projects(projects: &[PathBuf], sessions: &[SessionSummary]) -> Vec<PathBuf> {
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
    sort_projects_by_recency(projects, &recency)
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
mod tests {
    use super::*;

    #[test]
    fn provider_login_selects_use_the_native_picker() {
        let request = ExtensionUiRequest::Select {
            id: "login".into(),
            title: "Add provider".into(),
            options: vec!["xAI — Account".into(), "xAI — API key".into()],
            timeout: None,
        };

        assert!(matches!(
            provider_login_scope(&request),
            Some(PickerScope::ProviderLogin { id, title, options })
                if id == "login" && title == "Add provider" && options.len() == 2
        ));
        let mut ordinary = request;
        if let ExtensionUiRequest::Select { title, .. } = &mut ordinary {
            *title = "Choose a branch".into();
        }
        assert_eq!(provider_login_scope(&ordinary), None);
    }

    #[test]
    fn move_project_choices_exclude_the_source_project() {
        let source = PathBuf::from("/work/source");
        let target = PathBuf::from("/work/target");
        let intent = ProjectPickerIntent::MoveSession {
            path: PathBuf::from("/sessions/session.jsonl"),
            source_project: source.clone(),
        };

        assert!(!project_is_available_for_intent(&intent, &source));
        assert!(project_is_available_for_intent(&intent, &target));
    }

    #[test]
    fn projects_with_recent_sessions_lead_then_registry_order_breaks_ties() {
        let alpha = PathBuf::from("/work/alpha");
        let beta = PathBuf::from("/work/beta");
        let gamma = PathBuf::from("/work/gamma");
        let recency = HashMap::from([
            (alpha.clone(), Duration::from_secs(10)),
            (beta.clone(), Duration::from_secs(20)),
        ]);

        assert_eq!(
            sort_projects_by_recency(&[alpha.clone(), gamma.clone(), beta.clone()], &recency,),
            vec![beta, alpha, gamma]
        );
    }
}
