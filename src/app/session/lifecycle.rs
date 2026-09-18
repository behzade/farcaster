use super::*;

pub(in crate::app) const USER_SESSION_SWITCH_RESTORES_CENTER: bool = true;

pub(in crate::app) struct PendingMove {
    pub(in crate::app) focus: FocusHandle,
    path: PathBuf,
    target_project: PathBuf,
    return_focus: Option<FocusHandle>,
}

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
    fn target_for_path(&self, path: &Path) -> Option<SessionTarget> {
        let path = crate::sessions::normalize_session_path(path);
        self.sessions
            .all
            .iter()
            .find(|session| session.path == path)
            .map(SessionSummary::target)
            .or_else(|| self.runtime.session_targets.get(&path).cloned())
            .or_else(|| {
                self.snapshot
                    .session_target()
                    .filter(|target| target.path == path)
            })
    }

    pub(in crate::app) fn retry_after_session_refresh(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let current_target = self.composer.sessions.current_target().to_owned();
        let mut action = Some(action);
        cx.spawn_in(window, async move |weak, cx| {
            for _ in 0..50 {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                let done = weak
                    .update_in(cx, |this, window, cx| {
                        if this.composer.sessions.current_target() != current_target {
                            if this.sessions.error.as_deref() == Some("Refreshing session details…")
                            {
                                this.sessions.error = None;
                                this.notify_session_rail(cx);
                            }
                            return true;
                        }
                        if this.target_for_path(&path).is_some() {
                            if let Some(action) = action.take() {
                                this.sessions.error = None;
                                action(this, window, cx);
                            }
                            return true;
                        }
                        false
                    })
                    .unwrap_or(true);
                if done {
                    return;
                }
            }
            let _ = weak.update_in(cx, |this, _, cx| {
                if this.composer.sessions.current_target() == current_target {
                    this.sessions.error = Some(
                        "This session was not found after refreshing. It may have been removed."
                            .into(),
                    );
                    this.notify_session_rail(cx);
                }
            });
        })
        .detach();
    }

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
        let target = self.target_for_path(path);
        if target.is_none() {
            self.sessions.error = Some("Refreshing session details…".into());
            self.send(RuntimeCommand::RefreshSessions, cx);
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
            self.retry_after_session_refresh(path.clone(), window, cx, move |this, window, cx| {
                this.select_session_restoring_center(path, project, restore_center, window, cx);
            });
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
        if self.project.pending_trust_command.is_some() || self.center_surface_switch_blocked() {
            return;
        }
        let Some(target) = self.backend_target_for_path(&path, cx) else {
            self.retry_after_session_refresh(path.clone(), window, cx, move |this, window, cx| {
                this.fork_session(path, project, window, cx);
            });
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
        if self.workspace.surface == AppSurface::Work {
            self.show_chat_surface(window, cx);
        }
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
        let draft = match super::draft_store::new(
            project.clone(),
            self.sessions.preferred_harness,
            self.sessions.preferred_profile_id.clone(),
        ) {
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
        self.sync_project_folders(cx);
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
                self.retry_after_session_refresh(
                    path.clone(),
                    window,
                    cx,
                    move |this, window, cx| {
                        this.resume_draft_restoring_center(id, project, restore_center, window, cx);
                    },
                );
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
        self.remove_session_draft(id);
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
        let family_paths = crate::sessions::session_family_for_path(&self.sessions.all, &path)
            .map(|family| {
                family
                    .iter()
                    .map(|session| session.path.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec![path.clone()]);
        let has_pending_messages = match crate::app::persistence::open()
            .and_then(|store| store.has_queued_prompts_for(&family_paths))
        {
            Ok(pending) => pending,
            Err(error) => {
                self.sessions.error = Some(format!("Could not check pending messages: {error}"));
                self.notify_session_rail(cx);
                return;
            }
        };
        if self.session_family_has_active_work(&path) || has_pending_messages {
            self.cover_native_workspace_surface(cx);
            let pending = PendingMove {
                focus: cx.focus_handle(),
                path,
                target_project,
                return_focus: window.focused(cx),
            };
            pending.focus.focus(window, cx);
            self.sessions.pending_move = Some(pending);
            cx.notify();
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

    pub(in crate::app) fn close_move_confirmation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PendingMove> {
        let pending = self.sessions.pending_move.take()?;
        self.restore_overlay_focus(pending.return_focus.clone(), &pending.focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
        Some(pending)
    }

    pub(in crate::app) fn stop_and_move_pending_session(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.close_move_confirmation(window, cx) else {
            return;
        };
        self.send_project_command(
            &pending.target_project,
            RuntimeCommand::StopAndMoveSession {
                path: pending.path,
                target_project: pending.target_project.clone(),
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
