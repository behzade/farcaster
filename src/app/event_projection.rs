use super::*;

#[derive(Default)]
struct DirtyRegions {
    root: bool,
    rail: bool,
    archived_rail: bool,
    transcript: bool,
    composer: bool,
    run: bool,
    workgraph_session: bool,
    workgraph_goal: bool,
}

impl DirtyRegions {
    fn observe(&mut self, app: &FarcasterApp, event: &RuntimeEvent) {
        match event {
            RuntimeEvent::Snapshot { snapshot, .. } => {
                let roots = SessionRootIndex::new(&app.sessions);
                self.rail |= session_rail_snapshot_changed(&roots, &app.snapshot, snapshot);
                self.archived_rail |=
                    inactive_session_rail_snapshot_changed(&roots, &app.snapshot, snapshot);
                self.composer |= composer_snapshot_changed(&app.snapshot, snapshot);
                self.root |= app.snapshot.pending_question != snapshot.pending_question;
                self.run |= run_panel_snapshot_changed(&app.snapshot, snapshot);
                self.workgraph_session |=
                    app.snapshot.selected_session != snapshot.selected_session;
                self.workgraph_goal |= app.snapshot.session_goal != snapshot.session_goal;
            }
            RuntimeEvent::Sessions { .. }
            | RuntimeEvent::SessionsFailed { .. }
            | RuntimeEvent::SessionFilesModified { .. }
            | RuntimeEvent::ExtensionUi { .. } => {}
            RuntimeEvent::SessionMoved { .. } | RuntimeEvent::SessionDeleted { .. } => {
                self.root = true;
                self.rail = true;
                self.archived_rail = true;
                self.transcript = true;
                self.composer = true;
                self.run = true;
            }
            RuntimeEvent::SessionStatus {
                target, session, ..
            } => {
                self.rail |= session_event_affects_active_rail(
                    &app.drafts,
                    &app.submitted_drafts,
                    &app.sessions,
                    target,
                    session.as_deref(),
                );
                self.archived_rail |= archive::session_event_affects_archived_rail(
                    &app.sessions,
                    target,
                    session.as_deref(),
                );
            }
            RuntimeEvent::HistoryReset { .. } => self.transcript = true,
            RuntimeEvent::SessionReset { .. } => {
                self.root = true;
                self.transcript = true;
                self.composer = true;
                self.run = true;
            }
            RuntimeEvent::PromptResult {
                target, session, ..
            } => {
                self.root = true;
                self.rail |= session_event_affects_active_rail(
                    &app.drafts,
                    &app.submitted_drafts,
                    &app.sessions,
                    target,
                    session.as_deref(),
                );
                self.archived_rail |= archive::session_event_affects_archived_rail(
                    &app.sessions,
                    target,
                    session.as_deref(),
                );
                self.composer = true;
                self.run = true;
            }
            RuntimeEvent::RefreshCatalog | RuntimeEvent::Stopped => self.run = true,
        }
    }

    fn notify(self, app: &mut FarcasterApp, cx: &mut Context<FarcasterApp>) {
        if self.workgraph_session {
            app.refresh_workgraph_sidebar(cx);
        }
        if self.workgraph_goal {
            app.refresh_workgraph_goal(cx);
        }
        app.sync_notification_expiries(cx);
        app.sync_recent_completion_expiries(cx);
        if self.rail {
            app.notify_session_rail_shell(cx);
        }
        if self.archived_rail {
            app.notify_archived_session_rail(cx);
        }
        if self.transcript {
            app.notify_transcript(cx);
        }
        if self.composer {
            app.notify_composer(cx);
        }
        if self.run {
            app.notify_run_panel(cx);
        }
        if self.root {
            cx.notify();
        }
    }
}

impl FarcasterApp {
    fn project_snapshot(
        &mut self,
        generation: u64,
        snapshot: Arc<RuntimeSnapshot>,
        dirty: &mut DirtyRegions,
        cx: &mut Context<Self>,
    ) {
        if self
            .pending_session_switch
            .as_ref()
            .is_some_and(|(path, _)| snapshot.selected_session.as_deref() == Some(path.as_path()))
        {
            drop(self.pending_session_switch.take());
        }
        let session_changed = generation > self.runtime_generation;
        let transcript_preselected =
            session_changed && self.snapshot.selected_session == snapshot.selected_session;
        if session_changed {
            self.reset_session_ui(generation, transcript_preselected);
            dirty.root = true;
        }
        let row_update = if transcript_preselected {
            self.project_transcript_rows(&snapshot)
        } else if session_changed {
            let _timing = crate::app::infrastructure::performance::OperationTiming::new(
                crate::app::infrastructure::performance::OperationKind::FullProjection,
                snapshot.conversation.items.len(),
            );
            crate::app::views::transcript::TranscriptRowUpdate::replace(
                crate::app::views::transcript::project_rows(&snapshot.conversation.items),
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
            dirty.root = true;
            park_extension_surface(&mut self.extension, &mut self.parked_extension);
            self.pending_dialog_setup = false;
            self.dialog_return_focus = None;
        } else if !snapshot.history_preview && self.snapshot.history_preview {
            dirty.root = true;
            self.clear_restored_dialog();
            restore_extension_surface(&mut self.extension, &mut self.parked_extension);
            self.pending_dialog_setup = self.extension.dialog.is_some();
            self.dialog_return_focus = None;
        }
        self.snapshot = snapshot;
        dirty.transcript |= self.apply_transcript_rows(row_update);
        self.view.transcript.last_count = count;
        self.sync_restored_dialog();
        self.sync_composer_history();
        dirty.rail |= self.reconcile_submitted_drafts(cx);
    }
    fn project_sessions(
        &mut self,
        generation: u64,
        mut sessions: Vec<SessionSummary>,
        mut all_sessions: Vec<SessionSummary>,
        activities: Option<(HashMap<String, AgentActivity>, bool)>,
        dirty: &mut DirtyRegions,
        cx: &mut Context<Self>,
    ) {
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
        dirty.rail |= catalog_changed;
        dirty.archived_rail |= archived_catalog_changed;
        dirty.composer |= composer_usage_changed;
        dirty.run |= run_catalog_changed || visible_activities_changed;
        dirty.workgraph_session |= previous_workgraph_session != self.active_workgraph_session();
        dirty.rail |= self.reconcile_submitted_drafts(cx);
    }
    fn project_session_deleted(
        &mut self,
        generation: u64,
        paths: Arc<HashSet<PathBuf>>,
        cx: &mut Context<Self>,
    ) {
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
            let (next_target, next_draft) =
                match project_registry::new_draft(self.project.clone(), &self.preferred_harness) {
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
    fn project_session_moved(
        &mut self,
        target_root: PathBuf,
        target_project: PathBuf,
        paths: Arc<HashMap<PathBuf, PathBuf>>,
        cx: &mut Context<Self>,
    ) {
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
            if let Some(expiry) = self.recent_completion_expiries.remove(&source_target) {
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
    fn project_extension_ui(
        &mut self,
        generation: u64,
        request: crate::protocol::ExtensionUiRequest,
        system_notification_target: Option<(PathBuf, PathBuf)>,
        dirty: &mut DirtyRegions,
        cx: &mut Context<Self>,
    ) {
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
            dirty.root = true;
            dirty.composer = true;
        }
    }
    fn project_prompt_result(
        &mut self,
        target: String,
        accepted: bool,
        session: Option<PathBuf>,
        dirty: &mut DirtyRegions,
        cx: &mut Context<Self>,
    ) {
        self.record_draft_submission(&target, accepted, session.clone());
        if !accepted {
            self.run_statuses.insert(target.clone(), "Failed".into());
        }
        if let Some(pending) = self.pending_submissions.get_mut(&target) {
            pending.result = Some((accepted, session));
        }
        dirty.rail |= self.reconcile_submitted_drafts(cx);
    }

    fn project_runtime_event(
        &mut self,
        event: RuntimeEvent,
        dirty: &mut DirtyRegions,
        cx: &mut Context<Self>,
    ) {
        match event {
            RuntimeEvent::Snapshot {
                generation,
                snapshot,
            } if generation >= self.runtime_generation => {
                self.project_snapshot(generation, snapshot, dirty, cx);
            }
            RuntimeEvent::SessionReset {
                generation,
                preserve_submission,
            } if generation >= self.runtime_generation => {
                self.reset_session_ui(generation, preserve_submission);
            }
            RuntimeEvent::HistoryReset { generation } if generation == self.runtime_generation => {
                self.reset_transcript_ui();
            }
            RuntimeEvent::Sessions {
                generation,
                sessions,
                all_sessions,
                activities,
            } if generation >= self.session_generation => {
                self.project_sessions(generation, sessions, all_sessions, activities, dirty, cx);
            }
            RuntimeEvent::SessionDeleted { generation, paths } => {
                self.project_session_deleted(generation, paths, cx);
            }
            RuntimeEvent::SessionMoved {
                target_root,
                target_project,
                paths,
            } => self.project_session_moved(target_root, target_project, paths, cx),
            RuntimeEvent::SessionsFailed {
                generation,
                message,
            } if generation >= self.session_generation => {
                self.session_generation = generation;
                let changed = self.sessions_error.as_deref() != Some(message.as_str());
                self.sessions_error = Some(message);
                dirty.rail |= changed;
                dirty.run |= changed;
            }
            RuntimeEvent::ExtensionUi {
                generation,
                request,
                system_notification_target,
            } if generation == self.runtime_generation => {
                self.project_extension_ui(
                    generation,
                    request,
                    system_notification_target,
                    dirty,
                    cx,
                );
            }
            RuntimeEvent::PromptResult {
                generation,
                target,
                accepted,
                session,
            } if generation == self.runtime_generation => {
                self.project_prompt_result(target, accepted, session, dirty, cx);
            }
            RuntimeEvent::SessionStatus {
                target,
                session,
                status,
            } => {
                self.record_session_status(target, session, status);
                dirty.rail |= self.reconcile_submitted_drafts(cx);
            }
            RuntimeEvent::Stopped => Arc::make_mut(&mut self.snapshot).status = "Stopped".into(),
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
}

impl FarcasterApp {
    pub(super) fn drain_runtime(&mut self, cx: &mut Context<Self>) {
        let mut operation = crate::app::infrastructure::performance::OperationTiming::new(
            crate::app::infrastructure::performance::OperationKind::RuntimeDrain,
            0,
        );
        let _timing = crate::app::infrastructure::performance::Timing::new("runtime.drain_events");
        let mut dirty = DirtyRegions {
            run: self.performance_monitor.as_mut().is_some_and(
                crate::app::infrastructure::performance::PerformanceMonitor::sample_if_due,
            ),
            ..DirtyRegions::default()
        };
        while let Ok(event) = self.runtime.try_recv() {
            operation.increment_work();
            dirty.observe(self, &event);
            self.project_runtime_event(event, &mut dirty, cx);
        }
        dirty.notify(self, cx);
    }
}
