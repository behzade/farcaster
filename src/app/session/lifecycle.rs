use super::*;

pub(in crate::app) const USER_SESSION_SWITCH_RESTORES_CENTER: bool = true;

pub(in crate::app) fn current_close_target(
    selected_draft: Option<&str>,
    selected_session: Option<&std::path::Path>,
) -> CurrentCloseTarget {
    if let Some(id) = selected_draft {
        CurrentCloseTarget::Draft(id.to_owned())
    } else if let Some(path) = selected_session {
        CurrentCloseTarget::Session(path.to_owned())
    } else {
        CurrentCloseTarget::None
    }
}

impl FarcasterApp {
    pub(in crate::app) fn close_current_target(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.native_workspace_covered_by_overlay() {
            self.dismiss_surface(window, cx);
            return;
        }
        if self.workspace.surface == AppSurface::Work {
            self.show_chat_surface(window, cx);
            return;
        }
        if self.workspace.surface == AppSurface::Editor {
            self.close_editor(cx);
            return;
        }
        if self.workspace.surface == AppSurface::Terminal {
            self.close_terminal(window, cx);
            return;
        }
        match current_close_target(
            self.sessions.selected_draft.as_deref(),
            self.snapshot.selected_session.as_deref(),
        ) {
            CurrentCloseTarget::Draft(id) => self.discard_draft(&id, window, cx),
            CurrentCloseTarget::Session(path) => {
                self.archive_selected_session_and_advance(path, window, cx);
            }
            CurrentCloseTarget::None => {}
        }
    }

    pub(in crate::app) fn backend_target_for_path(
        &mut self,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> Option<SessionTarget> {
        let path = crate::sessions::normalize_session_path(path);
        let target = self
            .sessions
            .all
            .iter()
            .find(|session| session.path == path)
            .map(SessionSummary::target)
            .or_else(|| self.runtime.session_targets.get(&path).cloned())
            .or_else(|| {
                self.snapshot
                    .session_target()
                    .filter(|target| target.path == path)
            });
        if target.is_none() {
            self.sessions.error = Some(
                "The session's harness identity is unavailable; refresh sessions and try again"
                    .into(),
            );
            self.notify_session_rail(cx);
        }
        target
    }

    pub(in crate::app) fn select_session(
        &mut self,
        path: PathBuf,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_session_restoring_center(
            path,
            project,
            USER_SESSION_SWITCH_RESTORES_CENTER,
            window,
            cx,
        );
    }

    pub(in crate::app) fn select_session_and_focus(
        &mut self,
        path: PathBuf,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_session(path, project, window, cx);
        self.recover_keyboard_focus(window, cx);
    }

    pub(in crate::app) fn select_session_restoring_center(
        &mut self,
        path: PathBuf,
        project: PathBuf,
        restore_center: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.pending_trust_command.is_some() {
            return;
        }
        let _timing =
            crate::app::infrastructure::performance::Timing::new("switch.session_request");
        if self.snapshot.selected_session.as_deref() == Some(path.as_path())
            && self.sessions.selected_draft.is_none()
            && self
                .lifecycle
                .pending_session_switch
                .as_ref()
                .is_none_or(|(pending, _)| pending == &path)
        {
            self.close_sessions_sheet_after_selection(window, cx);
            return;
        }
        let previous_root = root_session_for_path(
            &self.sessions.visible,
            self.snapshot.selected_session.as_deref(),
        )
        .map(|session| session.id.clone());
        let Some(target) = self.backend_target_for_path(&path, cx) else {
            return;
        };
        let next_root = root_session_for_path(&self.sessions.visible, Some(&path))
            .map(|session| session.id.clone());
        self.switch_composer_target(session_target(&path), window, cx);
        self.sessions.selected_draft = None;
        self.select_project(project.clone(), cx);
        if restore_center {
            self.restore_center_surface(project.clone(), window, cx);
        }
        if let Some((_, timing)) = self.lifecycle.pending_session_switch.take() {
            timing.cancel();
        }
        self.lifecycle.pending_session_switch = Some((
            path.clone(),
            crate::app::infrastructure::performance::Timing::new("switch.session_total"),
        ));
        self.send_project_command(
            &project,
            RuntimeCommand::SelectSession {
                path,
                harness: target.harness,
                session_id: target.id,
                project: project.clone(),
            },
            window,
            cx,
        );
        self.close_sessions_sheet_after_selection(window, cx);
        if previous_root != next_root {
            self.reset_run_panel_scroll(cx);
            self.notify_session_rail(cx);
        }
        self.notify_transcript(cx);
        self.notify_composer(cx);
        self.notify_run_panel(cx);
        cx.notify();
    }

    pub(in crate::app) fn fork_session(
        &mut self,
        path: PathBuf,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.pending_trust_command.is_some() || self.workspace_switch_blocked() {
            return;
        }
        let Some(target) = self.backend_target_for_path(&path, cx) else {
            return;
        };
        if !crate::agents::supports_session_fork(target.harness) {
            self.sessions.error = Some(format!(
                "Forking {} sessions is not supported",
                target.harness
            ));
            self.notify_session_rail(cx);
            return;
        }
        self.reset_run_panel_scroll(cx);
        self.sessions.selected_draft = None;
        self.select_project(project.clone(), cx);
        self.restore_center_surface(project.clone(), window, cx);
        self.send_project_command(
            &project,
            RuntimeCommand::ForkSession {
                path,
                harness: target.harness,
                session_id: target.id,
                project: project.clone(),
            },
            window,
            cx,
        );
        self.close_sessions_sheet_after_selection(window, cx);
        self.notify_session_rail(cx);
        self.notify_transcript(cx);
        self.notify_composer(cx);
        self.notify_run_panel(cx);
        cx.notify();
    }

    pub(in crate::app) fn new_session(
        &mut self,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_session_with_folder(project, None, window, cx);
    }

    pub(in crate::app) fn new_session_with_folder(
        &mut self,
        project: PathBuf,
        folder: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if folder.is_some_and(|id| {
            !self
                .sessions
                .folders
                .folders
                .iter()
                .any(|folder| folder.id == id)
        }) {
            self.sessions.error = Some("This folder no longer exists".into());
            self.notify_session_rail(cx);
            return;
        }
        if self.project.pending_trust_command.is_some() {
            return;
        }
        self.reset_run_panel_scroll(cx);
        let draft =
            match project_registry::new_draft(project.clone(), self.sessions.preferred_harness) {
                Ok(draft) => draft,
                Err(error) => {
                    self.sessions.error = Some(error);
                    self.notify_session_rail(cx);
                    cx.notify();
                    return;
                }
            };
        let draft_key = draft_target(&draft.id);
        self.switch_composer_target(draft_key.clone(), window, cx);
        self.sessions.selected_draft = Some(draft.id.clone());
        self.sessions
            .draft_session_ids
            .insert(draft.id.clone(), draft.app_session_id);
        self.sessions.drafts.push(draft.clone());
        if let Some(folder) = folder {
            self.assign_session_folder(draft.app_session_id, Some(folder), cx);
        }
        self.save_project_registry();
        self.send_project_command(
            &project,
            RuntimeCommand::NewSession {
                id: draft.id,
                harness: draft.harness,
                project: project.clone(),
            },
            window,
            cx,
        );
        self.select_project(project.clone(), cx);
        self.restore_center_surface(project, window, cx);
        self.navigation
            .search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.close_sessions_sheet_after_selection(window, cx);
        self.show_chat_surface(window, cx);
        self.composer.focus.focus(window, cx);
        self.reveal_active_session_row(draft_key, cx);
        self.notify_session_rail(cx);
        self.notify_transcript(cx);
        self.notify_composer(cx);
        self.notify_run_panel(cx);
        cx.notify();
    }

    pub(in crate::app) fn resume_draft(
        &mut self,
        id: String,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resume_draft_restoring_center(
            id,
            project,
            USER_SESSION_SWITCH_RESTORES_CENTER,
            window,
            cx,
        );
    }

    pub(in crate::app) fn resume_draft_and_focus(
        &mut self,
        id: String,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resume_draft(id, project, window, cx);
        self.recover_keyboard_focus(window, cx);
    }

    pub(in crate::app) fn resume_draft_restoring_center(
        &mut self,
        id: String,
        project: PathBuf,
        restore_center: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.pending_trust_command.is_some() {
            return;
        }
        if self.sessions.selected_draft.as_deref() == Some(id.as_str())
            && !self.snapshot.history_preview
        {
            self.close_sessions_sheet_after_selection(window, cx);
            return;
        }
        let command = if let Some(Some(path)) = self.sessions.submitted_drafts.get(&id).cloned() {
            let Some(target) = self.backend_target_for_path(&path, cx) else {
                return;
            };
            RuntimeCommand::SelectSession {
                path,
                harness: target.harness,
                session_id: target.id,
                project: project.clone(),
            }
        } else {
            let Some(draft_harness) = self
                .sessions
                .drafts
                .iter()
                .find(|draft| draft.id == id)
                .map(|draft| draft.harness)
            else {
                self.sessions.error = Some("The draft's harness identity is unavailable".into());
                self.notify_session_rail(cx);
                return;
            };
            RuntimeCommand::ResumeDraft {
                id: id.clone(),
                harness: draft_harness,
                project: project.clone(),
            }
        };
        self.reset_run_panel_scroll(cx);
        self.switch_composer_target(draft_target(&id), window, cx);
        self.sessions.selected_draft = Some(id);
        self.select_project(project.clone(), cx);
        if restore_center {
            self.restore_center_surface(project.clone(), window, cx);
        }
        self.send_project_command(&project, command, window, cx);
        self.close_sessions_sheet_after_selection(window, cx);
        self.notify_session_rail(cx);
        self.notify_transcript(cx);
        self.notify_composer(cx);
        self.notify_run_panel(cx);
        cx.notify();
    }

    pub(in crate::app) fn discard_draft(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.pending_trust_command.is_some() {
            return;
        }
        let was_selected = self.sessions.selected_draft.as_deref() == Some(id);
        let target = draft_target(id);
        self.composer.images.remove(&target);
        self.composer.pastes.remove(&target);
        self.workspace.session_surfaces.remove(&target);
        self.workspace.editor.session_tabs.remove(&target);
        self.sessions.drafts.retain(|draft| draft.id != id);
        self.sessions.draft_session_ids.remove(id);
        self.sessions.submitted_drafts.remove(id);
        self.activity.run_statuses.remove(&target);
        self.activity.recent_completions.remove(&target);
        self.activity.recent_completion_expiries.remove(&target);
        if was_selected {
            self.sessions.selected_draft = None;
            if let Some(session) = self.sessions.visible.first().cloned() {
                self.select_project(session.project.clone(), cx);
                let snapshot = self
                    .composer
                    .sessions
                    .discard_and_switch(&target, session_target(&session.path));
                self.apply_composer_snapshot(snapshot, window, cx);
                self.restore_center_surface(session.project.clone(), window, cx);
                self.send_project_command(
                    &session.project,
                    RuntimeCommand::SelectSession {
                        path: session.path,
                        harness: session.harness,
                        session_id: session.id,
                        project: session.project.clone(),
                    },
                    window,
                    cx,
                );
            } else {
                let snapshot = self
                    .composer
                    .sessions
                    .discard_and_switch(&target, project_target(&self.project.path));
                self.apply_composer_snapshot(snapshot, window, cx);
                self.restore_center_surface(self.project.path.clone(), window, cx);
            }
        } else {
            let current = self.composer.sessions.current_target().to_owned();
            let _ = self.composer.sessions.discard_and_switch(&target, current);
        }
        self.save_project_registry();
        self.notify_session_rail(cx);
        self.notify_composer(cx);
        self.notify_run_panel(cx);
        cx.notify();
    }

    pub(in crate::app) fn move_session(
        &mut self,
        path: PathBuf,
        target_project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.pending_trust_command.is_some() {
            return;
        }
        let Some(session) = self
            .sessions
            .all
            .iter()
            .find(|session| session.path == path)
        else {
            self.sessions.error = Some("The session is no longer available to move".to_owned());
            self.notify_session_rail(cx);
            return;
        };
        if session.project == target_project {
            return;
        }
        if !crate::agents::supports_session_move(session.harness) {
            self.sessions.error = Some(format!(
                "Moving {} sessions between projects is not supported",
                session.harness
            ));
            self.notify_session_rail(cx);
            return;
        }
        if self.session_family_has_active_work(&path) {
            self.sessions.error = Some(
                "Wait for the session family to finish before moving it to another project"
                    .to_owned(),
            );
            self.notify_session_rail(cx);
            return;
        }
        self.send_project_command(
            &target_project,
            RuntimeCommand::MoveSession {
                path,
                target_project: target_project.clone(),
            },
            window,
            cx,
        );
    }

    pub(in crate::app) fn set_session_active(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.set_session_archived(path, false, cx);
    }

    pub(in crate::app) fn set_session_archived(
        &mut self,
        path: PathBuf,
        archived: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self
            .sessions
            .visible
            .iter_mut()
            .find(|session| session.path == path)
        {
            session.archived = archived;
        }
        if !self.sessions.visible.iter().any(|session| session.archived) {
            self.sessions.archived_expanded = false;
        }
        self.send(RuntimeCommand::SetSessionArchived { path, archived }, cx);
        self.notify_session_rail(cx);
        self.notify_run_panel(cx);
    }
}
