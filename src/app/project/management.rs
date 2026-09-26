use super::*;

impl FarcasterApp {
    pub(in crate::app) fn choose_project_folder(
        &mut self,
        intent: Option<ProjectPickerIntent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add project".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = selected.await else {
                return;
            };
            let Some(project) = paths.pop() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(project) = this.add_project(project, cx) else {
                    return;
                };
                match intent {
                    Some(ProjectPickerIntent::NewSession) => {
                        this.new_session(project, window, cx);
                    }
                    Some(ProjectPickerIntent::NewSessionInFolder(folder)) => {
                        this.new_session_with_folder(project, Some(folder), window, cx);
                    }
                    Some(ProjectPickerIntent::ChangeDraft) => {
                        this.change_draft_project(project, window, cx);
                        this.composer.focus.focus(window, cx);
                    }
                    Some(ProjectPickerIntent::MoveSession { path, .. }) => {
                        this.move_session(path, project, window, cx);
                    }
                    None => {}
                }
            });
        })
        .detach();
    }

    pub(in crate::app) fn add_project(
        &mut self,
        project: PathBuf,
        cx: &mut Context<Self>,
    ) -> Option<PathBuf> {
        let project = match project.canonicalize() {
            Ok(project) if project.is_dir() => project,
            Ok(project) => {
                self.sessions.error = Some(format!(
                    "Project path is not a folder: {}",
                    project.display()
                ));
                self.notify_session_rail(cx);
                cx.notify();
                return None;
            }
            Err(error) => {
                self.sessions.error = Some(format!("Open {}: {error}", project.display()));
                self.notify_session_rail(cx);
                cx.notify();
                return None;
            }
        };
        let restored = projects::restore(&mut self.project.excluded, &project);
        if projects::add_unique(&mut self.project.registered, project.clone()) || restored {
            self.save_project_registry();
        }
        self.warm_repository_observations(cx);
        self.notify_session_rail(cx);
        cx.notify();
        Some(project)
    }

    pub(in crate::app) fn remove_project_from_picker(
        &mut self,
        project: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !projects::remove(
            &mut self.project.registered,
            &mut self.project.excluded,
            project,
        ) {
            return;
        }
        self.save_project_registry();
        self.forget_repository_project(project);
        self.warm_repository_observations(cx);
        let scope = self
            .navigation
            .picker
            .as_ref()
            .map(|picker| picker.scope.clone())
            .unwrap_or(PickerScope::Projects(ProjectPickerIntent::NewSession));
        self.open_picker(scope, window, cx);
    }

    pub(in crate::app) fn select_project(&mut self, project: PathBuf, cx: &mut Context<Self>) {
        if self.project.path != project {
            self.composer.project_files.clear();
            self.composer.project_files_project = None;
            self.composer.project_files_loading = None;
        }
        self.project.path = project.clone();
        self.select_repository_project(project.clone(), cx);
        self.warm_repository_observations(cx);
        if projects::select(
            &mut self.project.registered,
            &self.project.excluded,
            project,
        ) {
            self.save_project_registry();
        }
    }

    pub(in crate::app) fn request_composer_project_files(&mut self, cx: &mut Context<Self>) {
        let project = self.project.path.clone();
        if !self.project.repository.execution_allowed {
            self.composer.project_files.clear();
            self.composer.project_files_project = Some(project);
            self.composer.project_files_loading = None;
            self.notify_composer(cx);
            return;
        }
        if self.composer.project_files_project.as_ref() == Some(&project)
            || self.composer.project_files_loading.as_ref() == Some(&project)
        {
            return;
        }
        self.composer.project_files_loading = Some(project.clone());
        let preference = self.project.repository.preference;
        let task = cx.background_spawn(async move {
            let files = file_mentions::project_files(&project, preference);
            (project, preference, files)
        });
        cx.spawn(async move |weak, cx| {
            let (project, preference, files) = task.await;
            let _ = weak.update(cx, |this, cx| {
                if this.composer.project_files_loading.as_ref() == Some(&project) {
                    this.composer.project_files_loading = None;
                }
                if this.project.path == project && this.project.repository.preference == preference
                {
                    this.composer.project_files = files;
                    this.composer.project_files_project = Some(project);
                    this.notify_composer(cx);
                }
            });
        })
        .detach();
    }

    pub(in crate::app) fn save_project_registry(&mut self) {
        if let Err(error) = project_registry::save(&projects::ProjectList {
            projects: self.project.registered.clone(),
            excluded_projects: self.project.excluded.clone(),
        }) {
            self.sessions.error = Some(error);
        }
    }
}
