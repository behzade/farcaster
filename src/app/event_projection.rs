use super::*;

fn update_session_row(sessions: &mut Vec<SessionSummary>, session: SessionSummary) {
    if let Some(index) = sessions
        .iter()
        .position(|previous| previous.path == session.path)
    {
        sessions.remove(index);
    }
    let index = sessions.partition_point(|previous| previous.modified > session.modified);
    sessions.insert(index, session);
}

#[derive(Clone, Copy)]
enum ActivityUpdateSource {
    Metadata,
    Native,
    Catalog,
}

fn merge_agent_activity(
    activities: &mut HashMap<String, AgentActivity>,
    incoming: AgentActivity,
    source: ActivityUpdateSource,
) -> bool {
    let key = crate::agent_activity::agent_activity_key(&incoming.session_path);
    let Some(existing) = activities.get_mut(&key) else {
        activities.insert(key, incoming);
        return true;
    };
    if matches!(source, ActivityUpdateSource::Metadata)
        || (matches!(
            incoming.lifecycle,
            crate::agent_activity::AgentLifecycle::Unknown
        ) && !matches!(
            existing.lifecycle,
            crate::agent_activity::AgentLifecycle::Unknown
        ))
    {
        return false;
    }
    let next = if !existing.limited && incoming.limited {
        let mut merged = existing.clone();
        merged.lifecycle = incoming.lifecycle;
        merged.ended = incoming.ended;
        merged.elapsed = incoming.elapsed;
        if matches!(
            merged.lifecycle,
            crate::agent_activity::AgentLifecycle::Completed(_)
        ) {
            merged.recent_tool = merged.current_tool.take().or(merged.recent_tool);
        }
        merged
    } else {
        incoming
    };
    if *existing == next {
        false
    } else {
        *existing = next;
        true
    }
}

fn agent_focus_keys(
    sessions: &[SessionSummary],
    activities: &HashMap<String, AgentActivity>,
) -> HashSet<String> {
    sessions
        .iter()
        .filter(|session| session.parent_session.is_some())
        .map(|session| crate::agent_activity::agent_activity_key(&session.path))
        .chain(activities.keys().cloned())
        .collect()
}

#[cfg(test)]
#[path = "event_projection_tests.rs"]
mod tests;

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
            | RuntimeEvent::SessionUpdated(_)
            | RuntimeEvent::AgentActivityUpdated(_)
            | RuntimeEvent::SessionMetadata(_)
            | RuntimeEvent::SessionTarget(_)
            | RuntimeEvent::SystemNotification { .. }
            | RuntimeEvent::SessionsFailed { .. }
            | RuntimeEvent::ImportPreview { .. }
            | RuntimeEvent::ImportPreviewFailed { .. }
            | RuntimeEvent::ExtensionUiDismissed { .. }
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
            self.reset_session_ui(generation, transcript_preselected, cx);
            dirty.root = true;
        }
        let row_update = if transcript_preselected {
            self.project_transcript_rows(&snapshot, cx)
        } else if session_changed {
            let _timing = crate::app::infrastructure::performance::OperationTiming::new(
                crate::app::infrastructure::performance::OperationKind::FullProjection,
                snapshot.conversation.items.len(),
            );
            crate::app::views::transcript::TranscriptRowUpdate::replace(
                crate::app::views::transcript::project_conversation_rows(&snapshot.conversation),
            )
        } else {
            self.project_transcript_rows(&snapshot, cx)
        };
        let count = row_update.row_count(self.transcript_view.read(cx).rows.len());
        self.transcript_view
            .update(cx, |transcript, _| transcript.update_count(count));
        if snapshot.history_preview && !self.snapshot.history_preview {
            dirty.root = true;
            park_extension_for_history(&mut self.extension, &mut self.parked_extension);
            self.pending_dialog_setup = self.extension.dialog.is_some();
            if self.extension.dialog.is_none() {
                self.dialog_return_focus = None;
            }
        } else if !snapshot.history_preview && self.snapshot.history_preview {
            dirty.root = true;
            self.clear_restored_dialog();
            restore_extension_after_history(&mut self.extension, &mut self.parked_extension);
            self.pending_dialog_setup = self.extension.dialog.is_some();
            if self.extension.dialog.is_none() {
                self.dialog_return_focus = None;
            }
        }
        self.snapshot = snapshot;
        dirty.transcript |= self.apply_transcript_rows(row_update, cx);
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
        if let Some((activities, _exhaustive)) = activities {
            for activity in activities.into_values() {
                dirty.run |= merge_agent_activity(
                    &mut self.agent_activities,
                    activity,
                    ActivityUpdateSource::Catalog,
                );
            }
        }
        let agent_ids = agent_focus_keys(&self.all_sessions, &self.agent_activities);
        self.agent_row_focus.retain(|id, _| agent_ids.contains(id));
        for id in agent_ids {
            self.agent_row_focus
                .entry(id)
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
        self.runtime
            .session_targets
            .retain(|path, _| !paths.contains(path));
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
            self.session_editor_tabs.remove(&target);
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
            self.session_editor_tabs.remove(&target);
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
        self.system_notification_targets
            .retain(|_, (path, _)| !paths.contains(path));
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
            self.reset_session_ui(generation, false, cx);
            self.pending_composer_restore = Some((next_target, composer));
            self.selected_draft = next_draft.as_ref().map(|draft| draft.id.clone());
            if let Some(draft) = next_draft {
                self.draft_session_ids
                    .insert(draft.id.clone(), draft.app_session_id);
                self.drafts.push(draft.clone());
                self.save_project_registry();
                self.send(
                    RuntimeCommand::NewSession {
                        id: draft.id,
                        harness: draft.harness,
                        project: draft.project,
                    },
                    cx,
                );
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
        target: crate::sessions::SessionTarget,
        target_project: PathBuf,
        paths: Arc<HashMap<PathBuf, PathBuf>>,
        cx: &mut Context<Self>,
    ) {
        for (source, target) in paths.iter() {
            if let Some(mut identity) = self.runtime.session_targets.remove(source) {
                identity.path = target.clone();
                self.runtime
                    .session_targets
                    .insert(target.clone(), identity);
            }
            let source_target = session_target(source);
            let target_target = session_target(target);
            if source_target != target_target {
                self.composer_sessions
                    .promote(&source_target, target_target.clone());
            }
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
        for (session, project) in self.system_notification_targets.values_mut() {
            if let Some(target) = paths.get(session) {
                *session = target.clone();
                *project = target_project.clone();
            }
        }
        let selected_was_moved = self
            .snapshot
            .selected_session
            .as_ref()
            .or(self.snapshot.live_session.as_ref())
            .is_some_and(|path| paths.contains_key(path));
        if selected_was_moved {
            self.select_project(target_project.clone(), cx);
            self.send(
                RuntimeCommand::SelectSession {
                    session_id: target.id,
                    path: target.path,
                    harness: target.harness,
                    project: target_project,
                },
                cx,
            );
        }
        self.save_project_registry();
    }
    fn project_extension_ui(
        &mut self,
        generation: u64,
        request: crate::protocol::ExtensionUiRequest,
        dirty: &mut DirtyRegions,
        cx: &mut Context<Self>,
    ) {
        if crate::app::runtime::recovery::is_recovery_dialog(&request) {
            self.apply_extension_request(request, generation, cx);
            dirty.root = true;
            dirty.composer = true;
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
        outcome: crate::agents::PromptOutcome,
        session: Option<PathBuf>,
        dirty: &mut DirtyRegions,
        cx: &mut Context<Self>,
    ) {
        let accepted = outcome == crate::agents::PromptOutcome::Accepted;
        self.code_task_result(&target, accepted, session.as_deref(), cx);
        self.record_draft_submission(
            &target,
            outcome != crate::agents::PromptOutcome::RejectedBeforeAcceptance,
            session.clone(),
        );
        if outcome == crate::agents::PromptOutcome::RejectedBeforeAcceptance {
            self.run_statuses.insert(target.clone(), "Failed".into());
        } else if outcome == crate::agents::PromptOutcome::DeliveryUnknown {
            self.run_statuses
                .insert(target.clone(), "Delivery unknown".into());
        }
        record_pending_prompt_result(&mut self.pending_submissions, &target, outcome, session);
        dirty.rail |= self.reconcile_submitted_drafts(cx);
    }

    fn project_runtime_event(
        &mut self,
        event: RuntimeEvent,
        dirty: &mut DirtyRegions,
        cx: &mut Context<Self>,
    ) {
        match event {
            RuntimeEvent::SessionTarget(target) => {
                self.runtime
                    .session_targets
                    .insert(target.path.clone(), target);
            }
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
                self.reset_session_ui(generation, preserve_submission, cx);
            }
            RuntimeEvent::HistoryReset { generation } if generation == self.runtime_generation => {
                self.reset_transcript_ui(cx);
            }
            RuntimeEvent::Sessions {
                generation,
                sessions,
                all_sessions,
                activities,
            } if generation >= self.session_generation => {
                self.project_sessions(generation, sessions, all_sessions, activities, dirty, cx);
            }
            RuntimeEvent::SessionUpdated(mut session) => {
                self.reconcile_pending_session_titles(&mut [], std::slice::from_mut(&mut session));
                dirty.archived_rail |= archive::session_event_affects_archived_rail(
                    &self.all_sessions,
                    "",
                    Some(&session.path),
                );
                let previous_workgraph_session = self.active_workgraph_session();
                let activity = crate::sessions::activity::ActivityBuilder::default().finish(
                    session.id.clone(),
                    session.path.clone(),
                    &session.title,
                    &session.first_user_message,
                    session.usage,
                    session.modified,
                    session.modified,
                    session.is_running,
                    true,
                );
                merge_agent_activity(
                    &mut self.agent_activities,
                    activity,
                    ActivityUpdateSource::Metadata,
                );
                let activity_key = crate::agent_activity::agent_activity_key(&session.path);
                self.agent_row_focus
                    .entry(activity_key)
                    .or_insert_with(|| cx.focus_handle());
                projects::add_visible(
                    &mut self.projects,
                    &self.excluded_projects,
                    session.project.clone(),
                );
                dirty.composer |= self.snapshot.selected_session.as_ref() == Some(&session.path);
                update_session_row(&mut self.all_sessions, session.clone());
                dirty.archived_rail |= archive::session_event_affects_archived_rail(
                    &self.all_sessions,
                    "",
                    Some(&session.path),
                );
                let query = self.search.read(cx).value();
                if query.trim().is_empty() {
                    update_session_row(&mut self.sessions, session);
                } else {
                    self.sessions = crate::sessions::filter_session_tree(
                        self.all_sessions.clone(),
                        query.trim(),
                    );
                }
                dirty.rail = true;
                dirty.run = true;
                dirty.workgraph_session |=
                    previous_workgraph_session != self.active_workgraph_session();
                dirty.rail |= self.reconcile_submitted_drafts(cx);
            }
            RuntimeEvent::AgentActivityUpdated(activity) => {
                let activity_key =
                    crate::agent_activity::agent_activity_key(&activity.session_path);
                let activity_path = activity.session_path.clone();
                dirty.run |= merge_agent_activity(
                    &mut self.agent_activities,
                    activity,
                    ActivityUpdateSource::Native,
                );
                self.agent_row_focus
                    .entry(activity_key)
                    .or_insert_with(|| cx.focus_handle());
                dirty.run |= self
                    .all_sessions
                    .iter()
                    .find(|session| {
                        crate::sessions::normalize_session_path(&session.path)
                            == crate::sessions::normalize_session_path(&activity_path)
                    })
                    .and_then(|session| {
                        root_session_for_path(&self.all_sessions, Some(&session.path))
                    })
                    .is_some_and(|root| {
                        self.snapshot.selected_session.as_deref() == Some(root.path.as_path())
                            || root_session_for_path(
                                &self.all_sessions,
                                self.snapshot.selected_session.as_deref(),
                            )
                            .is_some_and(|selected| selected.id == root.id)
                    });
            }
            RuntimeEvent::SessionDeleted { generation, paths } => {
                self.project_session_deleted(generation, paths, cx);
            }
            RuntimeEvent::SessionMoved {
                target,
                target_project,
                paths,
            } => self.project_session_moved(target, target_project, paths, cx),
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
                ..
            } if generation == self.runtime_generation => {
                self.project_extension_ui(generation, request, dirty, cx);
            }
            RuntimeEvent::ExtensionUiDismissed { generation, id } => {
                if project_dialog_dismissal(
                    generation,
                    self.runtime_generation,
                    &id,
                    &mut self.extension,
                    self.parked_extension.as_mut(),
                    &mut self.restored_dialog_id,
                    &mut self.dismissed_restored_dialog_id,
                    &mut self.pending_dialog_setup,
                ) {
                    dirty.root = true;
                    dirty.composer = true;
                }
            }
            RuntimeEvent::SystemNotification {
                title,
                body,
                target,
            } => {
                self.show_attention_notification(&title, &body, target, cx);
            }
            RuntimeEvent::PromptResult {
                target,
                outcome,
                session,
            } => {
                // Replies belong to a submission, even after navigation changes generations.
                self.project_prompt_result(target, outcome, session, dirty, cx);
            }
            RuntimeEvent::SessionStatus {
                target,
                session,
                status,
            } => {
                if status == "Stopped" {
                    let session_key = session.as_deref().map(session_target);
                    for (key, pending) in &mut self.pending_submissions {
                        if pending.submitted_target == target || Some(key) == session_key.as_ref() {
                            pending.result.get_or_insert((
                                crate::agents::PromptOutcome::RejectedBeforeAcceptance,
                                session.clone(),
                            ));
                        }
                    }
                    if let Some(path) = session.as_deref() {
                        for row in self.sessions.iter_mut().chain(self.all_sessions.iter_mut()) {
                            if row.path == path {
                                row.is_running = false;
                            }
                        }
                        clear_stopped_snapshot(Arc::make_mut(&mut self.snapshot), path);
                    }
                    dirty.root = true;
                    dirty.composer = true;
                    dirty.run = true;
                }
                self.code_tasks.associate(&target, session.as_deref());
                dirty.root |= self.code_tasks.notice_message().is_some();
                if status == "Stopped" {
                    self.code_task_result(&target, false, session.as_deref(), cx);
                }
                self.record_session_status(target, session, status);
                dirty.rail |= self.reconcile_submitted_drafts(cx);
            }
            RuntimeEvent::ImportPreview {
                generation,
                harness,
                sessions,
            } => {
                self.apply_import_preview(generation, harness, sessions, cx);
                dirty.root = true;
            }
            RuntimeEvent::ImportPreviewFailed {
                generation,
                harness,
                message,
            } => {
                self.apply_import_preview_failed(generation, harness, message, cx);
                dirty.root = true;
            }
            RuntimeEvent::Stopped => Arc::make_mut(&mut self.snapshot).status = "Stopped".into(),
            RuntimeEvent::Snapshot { .. }
            | RuntimeEvent::RefreshCatalog
            | RuntimeEvent::SessionMetadata(_)
            | RuntimeEvent::SessionReset { .. }
            | RuntimeEvent::HistoryReset { .. }
            | RuntimeEvent::ExtensionUi { .. }
            | RuntimeEvent::Sessions { .. }
            | RuntimeEvent::SessionsFailed { .. } => {}
        }
    }
}

fn park_extension_for_history(
    visible: &mut crate::app::extensions::ExtensionUiState,
    parked: &mut Option<crate::app::extensions::ExtensionUiState>,
) {
    let recovery_dialogs =
        visible.take_dialogs_matching(crate::app::runtime::recovery::is_recovery_dialog);
    park_extension_surface(visible, parked);
    visible.prepend_dialogs(recovery_dialogs);
}

fn restore_extension_after_history(
    visible: &mut crate::app::extensions::ExtensionUiState,
    parked: &mut Option<crate::app::extensions::ExtensionUiState>,
) {
    let recovery_dialogs =
        visible.take_dialogs_matching(crate::app::runtime::recovery::is_recovery_dialog);
    restore_extension_surface(visible, parked);
    visible.prepend_dialogs(recovery_dialogs);
}

#[allow(clippy::too_many_arguments)]
fn project_dialog_dismissal(
    generation: u64,
    runtime_generation: u64,
    id: &str,
    extension: &mut crate::app::extensions::ExtensionUiState,
    parked_extension: Option<&mut crate::app::extensions::ExtensionUiState>,
    restored_dialog_id: &mut Option<String>,
    dismissed_restored_dialog_id: &mut Option<String>,
    pending_dialog_setup: &mut bool,
) -> bool {
    if generation != runtime_generation {
        return false;
    }
    let recovery = crate::app::runtime::recovery::is_recovery_dialog_id(id);
    if !recovery && let Some(parked) = parked_extension {
        parked.dismiss_dialog(id);
        return false;
    }
    match extension.dismiss_dialog(id) {
        crate::app::extensions::DialogDismissal::ActiveWithNext
        | crate::app::extensions::DialogDismissal::ActiveFinal => {
            // Root lifecycle owns the Window needed to focus the next dialog or restore focus
            // after the final one disappears.
            *pending_dialog_setup = true;
            if restored_dialog_id.as_deref() == Some(id) {
                *restored_dialog_id = None;
                *dismissed_restored_dialog_id = Some(id.to_owned());
            }
            true
        }
        crate::app::extensions::DialogDismissal::NotFound
        | crate::app::extensions::DialogDismissal::Queued => false,
    }
}

fn record_pending_prompt_result(
    pending: &mut HashMap<String, PendingSubmission>,
    target: &str,
    outcome: crate::agents::PromptOutcome,
    session: Option<PathBuf>,
) {
    if let Some(pending) = pending
        .values_mut()
        .find(|pending| pending.submitted_target == target)
    {
        if pending.result.as_ref().is_some_and(|(previous, _)| {
            matches!(
                previous,
                crate::agents::PromptOutcome::Accepted
                    | crate::agents::PromptOutcome::RejectedBeforeAcceptance
            )
        }) {
            return;
        }
        let session = session.or_else(|| {
            pending
                .result
                .as_ref()
                .and_then(|(_, session)| session.clone())
        });
        pending.result = Some((outcome, session));
    }
}

fn clear_stopped_snapshot(snapshot: &mut RuntimeSnapshot, path: &Path) {
    if snapshot
        .live_session
        .as_deref()
        .or(snapshot.selected_session.as_deref())
        != Some(path)
    {
        return;
    }
    let conversation = Arc::make_mut(&mut snapshot.conversation);
    conversation.running = false;
    conversation.compacting = false;
    conversation.retrying = false;
    snapshot.pending_question = None;
    snapshot.connected = false;
    snapshot.status = "Stopped".into();
    snapshot.live_status = "Stopped".into();
    if let Some(session) = snapshot.session.as_mut() {
        session.is_streaming = false;
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
