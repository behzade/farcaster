use super::*;

impl FarcasterApp {
    pub(super) fn drain_runtime(&mut self, cx: &mut Context<Self>) {
        let mut operation = crate::app::infrastructure::performance::OperationTiming::new(
            crate::app::infrastructure::performance::OperationKind::RuntimeDrain,
            0,
        );
        let _timing = crate::app::infrastructure::performance::Timing::new("runtime.drain_events");
        let mut root_dirty = false;
        let performance_changed = self.performance_monitor.as_mut().is_some_and(
            crate::app::infrastructure::performance::PerformanceMonitor::sample_if_due,
        );
        let mut rail_dirty = false;
        let mut archived_rail_dirty = false;
        let mut transcript_dirty = false;
        let mut composer_dirty = false;
        let mut run_dirty = performance_changed;
        let mut workgraph_session_dirty = false;
        let mut workgraph_goal_dirty = false;
        while let Ok(event) = self.runtime.try_recv() {
            operation.increment_work();
            match &event {
                RuntimeEvent::Snapshot { snapshot, .. } => {
                    let roots = SessionRootIndex::new(&self.sessions);
                    rail_dirty |= session_rail_snapshot_changed(&roots, &self.snapshot, snapshot);
                    archived_rail_dirty |=
                        inactive_session_rail_snapshot_changed(&roots, &self.snapshot, snapshot);
                    composer_dirty |= composer_snapshot_changed(&self.snapshot, snapshot);
                    root_dirty |= self.snapshot.pending_question != snapshot.pending_question;
                    run_dirty |= run_panel_snapshot_changed(&self.snapshot, snapshot);
                    workgraph_session_dirty |=
                        self.snapshot.selected_session != snapshot.selected_session;
                    workgraph_goal_dirty |= self.snapshot.session_goal != snapshot.session_goal;
                }
                RuntimeEvent::Sessions { .. }
                | RuntimeEvent::SessionsFailed { .. }
                | RuntimeEvent::SessionFilesModified { .. } => {}
                RuntimeEvent::SessionMoved { .. } | RuntimeEvent::SessionDeleted { .. } => {
                    root_dirty = true;
                    rail_dirty = true;
                    archived_rail_dirty = true;
                    transcript_dirty = true;
                    composer_dirty = true;
                    run_dirty = true;
                }
                RuntimeEvent::SessionStatus {
                    target, session, ..
                } => {
                    rail_dirty |= session_event_affects_active_rail(
                        &self.drafts,
                        &self.submitted_drafts,
                        &self.sessions,
                        target,
                        session.as_deref(),
                    );
                    archived_rail_dirty |= archive::session_event_affects_archived_rail(
                        &self.sessions,
                        target,
                        session.as_deref(),
                    );
                }
                RuntimeEvent::HistoryReset { .. } => transcript_dirty = true,
                RuntimeEvent::SessionReset { .. } => {
                    root_dirty = true;
                    transcript_dirty = true;
                    composer_dirty = true;
                    run_dirty = true;
                }
                RuntimeEvent::ExtensionUi { .. } => {}
                RuntimeEvent::PromptResult {
                    target, session, ..
                } => {
                    root_dirty = true;
                    rail_dirty |= session_event_affects_active_rail(
                        &self.drafts,
                        &self.submitted_drafts,
                        &self.sessions,
                        target,
                        session.as_deref(),
                    );
                    archived_rail_dirty |= archive::session_event_affects_archived_rail(
                        &self.sessions,
                        target,
                        session.as_deref(),
                    );
                    composer_dirty = true;
                    run_dirty = true;
                }
                RuntimeEvent::RefreshCatalog | RuntimeEvent::Stopped => run_dirty = true,
            }
            match event {
                RuntimeEvent::Snapshot {
                    generation,
                    snapshot,
                } if generation >= self.runtime_generation => {
                    if self
                        .pending_session_switch
                        .as_ref()
                        .is_some_and(|(path, _)| {
                            snapshot.selected_session.as_deref() == Some(path.as_path())
                        })
                    {
                        drop(self.pending_session_switch.take());
                    }
                    let session_changed = generation > self.runtime_generation;
                    let transcript_preselected = session_changed
                        && self.snapshot.selected_session == snapshot.selected_session;
                    if session_changed {
                        self.reset_session_ui(generation, transcript_preselected);
                        root_dirty = true;
                    }
                    let row_update = if transcript_preselected {
                        self.project_transcript_rows(&snapshot)
                    } else if session_changed {
                        let _timing = crate::app::infrastructure::performance::OperationTiming::new(
                            crate::app::infrastructure::performance::OperationKind::FullProjection,
                            snapshot.conversation.items.len(),
                        );
                        crate::app::views::transcript::TranscriptRowUpdate::replace(
                            crate::app::views::transcript::project_rows(
                                &snapshot.conversation.items,
                            ),
                        )
                    } else {
                        self.project_transcript_rows(&snapshot)
                    };
                    let count = row_update.row_count(self.view.transcript.rows.len());
                    if count > self.view.transcript.last_count && !self.view.transcript.following {
                        self.view.transcript.unseen = self
                            .view
                            .transcript
                            .unseen
                            .saturating_add(count - self.view.transcript.last_count);
                    }
                    if snapshot.history_preview && !self.snapshot.history_preview {
                        root_dirty = true;
                        park_extension_surface(&mut self.extension, &mut self.parked_extension);
                        self.pending_dialog_setup = false;
                        self.dialog_return_focus = None;
                    } else if !snapshot.history_preview && self.snapshot.history_preview {
                        root_dirty = true;
                        self.clear_restored_dialog();
                        restore_extension_surface(&mut self.extension, &mut self.parked_extension);
                        self.pending_dialog_setup = self.extension.dialog.is_some();
                        self.dialog_return_focus = None;
                    }
                    self.snapshot = snapshot;
                    transcript_dirty |= self.apply_transcript_rows(row_update);
                    self.view.transcript.last_count = count;
                    self.sync_restored_dialog();
                    self.sync_composer_history();
                    rail_dirty |= self.reconcile_submitted_drafts(cx);
                }
                RuntimeEvent::SessionReset {
                    generation,
                    preserve_submission,
                } if generation >= self.runtime_generation => {
                    self.reset_session_ui(generation, preserve_submission);
                }
                RuntimeEvent::HistoryReset { generation }
                    if generation == self.runtime_generation =>
                {
                    self.reset_transcript_ui();
                }
                RuntimeEvent::Sessions {
                    generation,
                    mut sessions,
                    mut all_sessions,
                    activities,
                    ..
                } if generation >= self.session_generation => {
                    self.session_generation = generation;
                    self.reconcile_pending_session_titles(&mut sessions, &mut all_sessions);
                    let catalog_changed = session_catalog_changed(
                        &self.sessions,
                        &self.all_sessions,
                        self.sessions_error.as_deref(),
                        &sessions,
                        &all_sessions,
                    );
                    let archived_catalog_changed = inactive_session_catalog_changed(
                        &self.sessions,
                        &self.all_sessions,
                        &sessions,
                        &all_sessions,
                    );
                    let run_catalog_changed = run_panel_sessions_changed(
                        &self.all_sessions,
                        &all_sessions,
                        self.snapshot.selected_session.as_deref(),
                    );
                    let composer_usage_changed = composer_usage_sessions_changed(
                        &self.all_sessions,
                        &all_sessions,
                        self.snapshot.selected_session.as_deref(),
                    );
                    let previous_workgraph_session = self.active_workgraph_session();
                    let visible_activities_changed = run_panel_activities_changed(
                        &self.agent_activities,
                        activities.as_ref(),
                        &self.all_sessions,
                        self.snapshot.selected_session.as_deref(),
                    );
                    for session in &all_sessions {
                        projects::add_visible(
                            &mut self.projects,
                            &self.excluded_projects,
                            session.project.clone(),
                        );
                    }
                    self.sessions_error = None;
                    self.sessions = sessions;
                    self.all_sessions = all_sessions;
                    if let Some((activities, exhaustive)) = activities {
                        if exhaustive {
                            self.agent_activities = activities;
                        } else {
                            self.agent_activities.extend(activities);
                        }
                    }
                    self.agent_row_focus
                        .retain(|id, _| self.agent_activities.contains_key(id));
                    for id in self.agent_activities.keys() {
                        self.agent_row_focus
                            .entry(id.clone())
                            .or_insert_with(|| cx.focus_handle());
                    }
                    rail_dirty |= catalog_changed;
                    archived_rail_dirty |= archived_catalog_changed;
                    composer_dirty |= composer_usage_changed;
                    run_dirty |= run_catalog_changed || visible_activities_changed;
                    workgraph_session_dirty |=
                        previous_workgraph_session != self.active_workgraph_session();
                    rail_dirty |= self.reconcile_submitted_drafts(cx);
                }
                RuntimeEvent::SessionDeleted { generation, paths } => {
                    let selected_was_deleted = self
                        .snapshot
                        .selected_session
                        .as_ref()
                        .or(self.snapshot.live_session.as_ref())
                        .is_some_and(|path| paths.contains(path));
                    let deleted_draft_ids = self
                        .drafts
                        .iter()
                        .filter(|draft| {
                            draft
                                .session_path
                                .as_ref()
                                .is_some_and(|path| paths.contains(path))
                        })
                        .map(|draft| draft.id.clone())
                        .chain(self.submitted_drafts.iter().filter_map(|(id, path)| {
                            path.as_ref()
                                .is_some_and(|path| paths.contains(path))
                                .then_some(id.clone())
                        }))
                        .collect::<HashSet<_>>();
                    for path in paths.iter() {
                        let target = session_target(path);
                        self.composer_sessions.remove(&target);
                        self.session_surfaces.remove(&target);
                        self.composer_images.remove(&target);
                        self.composer_pastes.remove(&target);
                        self.pending_submissions.remove(&target);
                        self.run_statuses.remove(&target);
                        self.recent_completions.remove(&target);
                        self.recent_completion_expiries.remove(&target);
                    }
                    for id in &deleted_draft_ids {
                        let target = draft_target(id);
                        self.composer_sessions.remove(&target);
                        self.session_surfaces.remove(&target);
                        self.composer_images.remove(&target);
                        self.composer_pastes.remove(&target);
                        self.pending_submissions.remove(&target);
                        self.submitted_drafts.remove(id);
                        self.draft_session_ids.remove(id);
                        self.run_statuses.remove(&target);
                        self.recent_completions.remove(&target);
                        self.recent_completion_expiries.remove(&target);
                    }
                    if !deleted_draft_ids.is_empty() {
                        self.drafts
                            .retain(|draft| !deleted_draft_ids.contains(&draft.id));
                        if self
                            .selected_draft
                            .as_ref()
                            .is_some_and(|id| deleted_draft_ids.contains(id))
                        {
                            self.selected_draft = None;
                        }
                        self.save_project_registry();
                    }
                    if self
                        .system_notification_target
                        .as_ref()
                        .is_some_and(|(path, _)| paths.contains(path))
                    {
                        self.system_notification_target = None;
                    }
                    if self
                        .pending_session_switch
                        .as_ref()
                        .is_some_and(|(path, _)| paths.contains(path))
                    {
                        drop(self.pending_session_switch.take());
                    }
                    if selected_was_deleted && generation >= self.runtime_generation {
                        let current_target = self.composer_sessions.current_target().to_owned();
                        let (next_target, next_draft) = match project_registry::new_draft(
                            self.project.clone(),
                            &self.preferred_harness,
                        ) {
                            Ok(draft) => (draft_target(&draft.id), Some(draft)),
                            Err(error) => {
                                self.sessions_error = Some(error);
                                (project_target(&self.project), None)
                            }
                        };
                        let composer = self
                            .composer_sessions
                            .discard_and_switch(&current_target, next_target.clone());
                        self.hide_native_workspace_surfaces(cx);
                        if self.surface != AppSurface::Work {
                            self.set_surface(AppSurface::Chat, cx);
                        }
                        self.reset_session_ui(generation, false);
                        self.pending_composer_restore = Some((next_target, composer));
                        self.selected_draft = next_draft.as_ref().map(|draft| draft.id.clone());
                        if let Some(draft) = next_draft {
                            self.draft_session_ids
                                .insert(draft.id.clone(), draft.app_session_id);
                            self.drafts.push(draft.clone());
                            self.save_project_registry();
                            self.send(RuntimeCommand::NewSession {
                                id: draft.id,
                                harness: draft.harness,
                                project: draft.project,
                            });
                        }
                        let snapshot = Arc::make_mut(&mut self.snapshot);
                        snapshot.live_session = None;
                        snapshot.selected_session = None;
                        snapshot.session = None;
                        snapshot.conversation = Arc::default();
                        snapshot.history_preview = false;
                        snapshot.pending_question = None;
                    }
                }
                RuntimeEvent::SessionMoved {
                    target_root,
                    target_project,
                    paths,
                } => {
                    for (source, target) in paths.iter() {
                        let source_target = session_target(source);
                        let target_target = session_target(target);
                        self.composer_sessions
                            .promote(&source_target, target_target.clone());
                        self.promote_center_surface(&source_target, &target_target);
                        if let Some(images) = self.composer_images.remove(&source_target) {
                            self.composer_images.insert(target_target.clone(), images);
                        }
                        self.promote_composer_pastes(&source_target, &target_target);
                        if let Some(status) = self.run_statuses.remove(&source_target) {
                            self.run_statuses.insert(target_target.clone(), status);
                        }
                        if let Some(completion) = self.recent_completions.remove(&source_target) {
                            self.recent_completions
                                .insert(target_target.clone(), completion);
                        }
                        if let Some(expiry) = self.recent_completion_expiries.remove(&source_target)
                        {
                            self.recent_completion_expiries
                                .insert(target_target.clone(), expiry);
                        }
                        for draft in &mut self.drafts {
                            if draft.session_path.as_deref() == Some(source.as_path()) {
                                draft.session_path = Some(target.clone());
                                draft.project = target_project.clone();
                            }
                        }
                        for session_path in self.submitted_drafts.values_mut().flatten() {
                            if session_path == source {
                                *session_path = target.clone();
                            }
                        }
                    }
                    if let Some((session, project)) = self.system_notification_target.as_mut()
                        && let Some(target) = paths.get(session)
                    {
                        *session = target.clone();
                        *project = target_project.clone();
                    }
                    let selected_was_moved = self
                        .snapshot
                        .selected_session
                        .as_ref()
                        .or(self.snapshot.live_session.as_ref())
                        .is_some_and(|path| paths.contains_key(path));
                    if selected_was_moved {
                        self.select_project(target_project.clone(), cx);
                        self.send(RuntimeCommand::SelectSession {
                            session_id: target_root.to_string_lossy().into_owned(),
                            path: target_root,
                            harness: "pi".into(),
                            project: target_project,
                        });
                    }
                    self.save_project_registry();
                }
                RuntimeEvent::SessionsFailed {
                    generation,
                    message,
                } if generation >= self.session_generation => {
                    self.session_generation = generation;
                    let changed = self.sessions_error.as_deref() != Some(message.as_str());
                    self.sessions_error = Some(message);
                    rail_dirty |= changed;
                    run_dirty |= changed;
                }
                RuntimeEvent::ExtensionUi {
                    generation,
                    request,
                    system_notification_target,
                } if generation == self.runtime_generation => {
                    if let Some((title, body)) = request.gpui_system_notification() {
                        self.system_notification_target = system_notification_target;
                        cx.show_system_notification(SystemNotification {
                            tag: SYSTEM_NOTIFICATION_TAG.into(),
                            title: title.into(),
                            body: body.into(),
                            actions: Vec::new(),
                        });
                    } else if let Some(extension) = self.parked_extension.as_mut() {
                        let _ = extension.apply(request);
                    } else {
                        self.apply_extension_request(request, generation, cx);
                        root_dirty = true;
                        composer_dirty = true;
                    }
                }
                RuntimeEvent::PromptResult {
                    generation,
                    target,
                    accepted,
                    session,
                } if generation == self.runtime_generation => {
                    self.record_draft_submission(&target, accepted, session.clone());
                    if !accepted {
                        self.run_statuses.insert(target.clone(), "Failed".into());
                    }
                    if let Some(pending) = self.pending_submissions.get_mut(&target) {
                        pending.result = Some((accepted, session));
                    }
                    rail_dirty |= self.reconcile_submitted_drafts(cx);
                }
                RuntimeEvent::SessionStatus {
                    target,
                    session,
                    status,
                } => {
                    self.record_session_status(target, session, status);
                    rail_dirty |= self.reconcile_submitted_drafts(cx);
                }
                RuntimeEvent::Stopped => {
                    Arc::make_mut(&mut self.snapshot).status = "Stopped".into()
                }
                RuntimeEvent::Snapshot { .. }
                | RuntimeEvent::RefreshCatalog
                | RuntimeEvent::SessionReset { .. }
                | RuntimeEvent::HistoryReset { .. }
                | RuntimeEvent::ExtensionUi { .. }
                | RuntimeEvent::PromptResult { .. }
                | RuntimeEvent::Sessions { .. }
                | RuntimeEvent::SessionsFailed { .. }
                | RuntimeEvent::SessionFilesModified { .. } => {}
            }
        }
        if workgraph_session_dirty {
            self.refresh_workgraph_sidebar(cx);
        }
        if workgraph_goal_dirty {
            self.refresh_workgraph_goal(cx);
        }
        self.sync_notification_expiries(cx);
        self.sync_recent_completion_expiries(cx);
        if rail_dirty {
            self.notify_session_rail_shell(cx);
        }
        if archived_rail_dirty {
            self.notify_archived_session_rail(cx);
        }
        if transcript_dirty {
            self.notify_transcript(cx);
        }
        if composer_dirty {
            self.notify_composer(cx);
        }
        if run_dirty {
            self.notify_run_panel(cx);
        }
        if root_dirty {
            cx.notify();
        }
    }
}
