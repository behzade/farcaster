use super::*;

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
    pub(in crate::app) fn backend_target_for_path(&self, path: &Path) -> SessionTarget {
        self.all_sessions
            .iter()
            .find(|session| session.path == path)
            .map(SessionSummary::target)
            .unwrap_or_else(|| SessionTarget::pi(path.to_path_buf()))
    }

    pub(in crate::app) fn select_session(
        &mut self,
        path: PathBuf,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_project_trust_command.is_some() {
            return;
        }
        let _timing =
            crate::app::infrastructure::performance::Timing::new("switch.session_request");
        if self.snapshot.selected_session.as_deref() == Some(path.as_path())
            && self.selected_draft.is_none()
        {
            self.close_sessions_sheet_after_selection(window, cx);
            return;
        }
        let previous_root =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .map(|session| session.id.clone());
        let next_root =
            root_session_for_path(&self.sessions, Some(&path)).map(|session| session.id.clone());
        self.switch_composer_target(session_target(&path), window, cx);
        self.selected_draft = None;
        self.select_project(project.clone(), cx);
        self.restore_center_surface(project.clone(), window, cx);
        if let Some((_, timing)) = self.pending_session_switch.take() {
            timing.cancel();
        }
        self.pending_session_switch = Some((
            path.clone(),
            crate::app::infrastructure::performance::Timing::new("switch.session_total"),
        ));
        let target = self.backend_target_for_path(&path);
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
        if self.pending_project_trust_command.is_some() || self.workspace_switch_blocked() {
            return;
        }
        self.reset_run_panel_scroll(cx);
        self.selected_draft = None;
        self.select_project(project.clone(), cx);
        self.restore_center_surface(project.clone(), window, cx);
        let target = self.backend_target_for_path(&path);
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
        if self.pending_project_trust_command.is_some() {
            return;
        }
        self.reset_run_panel_scroll(cx);
        let draft = match project_registry::new_draft(project.clone(), &self.preferred_harness) {
            Ok(draft) => draft,
            Err(error) => {
                self.sessions_error = Some(error);
                self.notify_session_rail(cx);
                cx.notify();
                return;
            }
        };
        let draft_key = draft_target(&draft.id);
        self.switch_composer_target(draft_key, window, cx);
        self.selected_draft = Some(draft.id.clone());
        self.draft_session_ids
            .insert(draft.id.clone(), draft.app_session_id);
        self.drafts.push(draft.clone());
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
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.close_sessions_sheet_after_selection(window, cx);
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
        if self.pending_project_trust_command.is_some() {
            return;
        }
        if self.selected_draft.as_deref() == Some(id.as_str()) && !self.snapshot.history_preview {
            self.close_sessions_sheet_after_selection(window, cx);
            return;
        }
        self.reset_run_panel_scroll(cx);
        self.switch_composer_target(draft_target(&id), window, cx);
        self.selected_draft = Some(id.clone());
        self.select_project(project.clone(), cx);
        self.restore_center_surface(project.clone(), window, cx);
        let draft_harness = self
            .drafts
            .iter()
            .find(|draft| draft.id == id)
            .map(|draft| draft.harness.clone())
            .unwrap_or_else(|| "pi".into());
        let command = if let Some(Some(path)) = self.submitted_drafts.get(&id).cloned() {
            let target = self.backend_target_for_path(&path);
            RuntimeCommand::SelectSession {
                path,
                harness: target.harness,
                session_id: target.id,
                project: project.clone(),
            }
        } else {
            RuntimeCommand::ResumeDraft {
                id,
                harness: draft_harness,
                project: project.clone(),
            }
        };
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
        if self.pending_project_trust_command.is_some() {
            return;
        }
        let was_selected = self.selected_draft.as_deref() == Some(id);
        let target = draft_target(id);
        self.composer_images.remove(&target);
        self.composer_pastes.remove(&target);
        self.session_surfaces.remove(&target);
        self.drafts.retain(|draft| draft.id != id);
        self.draft_session_ids.remove(id);
        self.submitted_drafts.remove(id);
        self.run_statuses.remove(&target);
        self.recent_completions.remove(&target);
        self.recent_completion_expiries.remove(&target);
        if was_selected {
            self.selected_draft = None;
            if let Some(session) = self.sessions.first().cloned() {
                self.select_project(session.project.clone(), cx);
                let snapshot = self
                    .composer_sessions
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
                    .composer_sessions
                    .discard_and_switch(&target, project_target(&self.project));
                self.apply_composer_snapshot(snapshot, window, cx);
                self.restore_center_surface(self.project.clone(), window, cx);
            }
        } else {
            let current = self.composer_sessions.current_target().to_owned();
            let _ = self.composer_sessions.discard_and_switch(&target, current);
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
        if self.pending_project_trust_command.is_some() {
            return;
        }
        let Some(session) = self
            .all_sessions
            .iter()
            .find(|session| session.path == path)
        else {
            self.sessions_error = Some("The session is no longer available to move".to_owned());
            self.notify_session_rail(cx);
            return;
        };
        if session.project == target_project {
            return;
        }
        if session.harness != "pi" {
            self.sessions_error = Some(format!(
                "Moving {} sessions between projects is not supported",
                session.harness
            ));
            self.notify_session_rail(cx);
            return;
        }
        if session.is_running {
            self.sessions_error = Some(
                "Wait for the session to finish before moving it to another project".to_owned(),
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
            .iter_mut()
            .find(|session| session.path == path)
        {
            session.archived = archived;
        }
        if !self.sessions.iter().any(|session| session.archived) {
            self.archived_sessions_expanded = false;
        }
        self.send(RuntimeCommand::SetSessionArchived { path, archived }, cx);
        self.notify_session_rail(cx);
        self.notify_run_panel(cx);
    }
}
