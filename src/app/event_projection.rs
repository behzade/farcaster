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
        || (!incoming.explicit_outcome
            && existing.explicit_outcome
            && matches!(
                incoming.lifecycle,
                crate::agent_activity::AgentLifecycle::Completed(_)
            )
            && matches!(
                existing.lifecycle,
                crate::agent_activity::AgentLifecycle::Completed(_)
            ))
    {
        return false;
    }
    let next = if !existing.limited && incoming.limited {
        let mut merged = existing.clone();
        merged.lifecycle = incoming.lifecycle;
        merged.explicit_outcome = incoming.explicit_outcome;
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

const TURN_COMPLETED_NOTIFICATION_TITLE: &str = "Farcaster: Turn completed";

fn completion_notification_is_redundant(
    window_active: bool,
    target: Option<&(PathBuf, PathBuf)>,
    displayed: &RuntimeSnapshot,
) -> bool {
    window_active
        && target.is_some_and(|(session, project)| {
            displayed.selected_session.as_ref() == Some(session) && &displayed.project == project
        })
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
                let roots = SessionRootIndex::new(&app.sessions.visible);
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
            | RuntimeEvent::TurnCompletedNotification { .. }
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
                    &app.sessions.drafts,
                    &app.sessions.submitted_drafts,
                    &app.sessions.visible,
                    target,
                    session.as_deref(),
                );
                self.archived_rail |= archive::session_event_affects_archived_rail(
                    &app.sessions.visible,
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
                    &app.sessions.drafts,
                    &app.sessions.submitted_drafts,
                    &app.sessions.visible,
                    target,
                    session.as_deref(),
                );
                self.archived_rail |= archive::session_event_affects_archived_rail(
                    &app.sessions.visible,
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
            .lifecycle
            .pending_session_switch
            .as_ref()
            .is_some_and(|(path, _)| snapshot.selected_session.as_deref() == Some(path.as_path()))
        {
            drop(self.lifecycle.pending_session_switch.take());
        }
        let session_changed = generation > self.runtime_generation;
        let transcript_preselected =
            session_changed && self.snapshot.selected_session == snapshot.selected_session;
        if session_changed {
            self.reset_session_ui(generation, transcript_preselected, cx);
            dirty.root = true;
        }
        let row_update = if session_changed && !transcript_preselected {
            let _timing = crate::app::infrastructure::performance::OperationTiming::new(
                crate::app::infrastructure::performance::OperationKind::FullProjection,
                snapshot.conversation.items.len(),
            );
            crate::app::views::transcript::TranscriptRowUpdate::replace(
                crate::app::views::transcript::project_presentation_rows(
                    &snapshot.transcript_presentation(),
                ),
            )
        } else {
            self.project_transcript_rows(&snapshot, cx)
        };
        let count = row_update.row_count(self.views.transcript.read(cx).rows.len());
        self.views
            .transcript
            .update(cx, |transcript, _| transcript.update_count(count));
        if snapshot.history_preview && !self.snapshot.history_preview {
            dirty.root = true;
            park_extension_for_history(&mut self.extensions.active, &mut self.extensions.parked);
            self.extensions.pending_dialog_setup = self.extensions.active.dialog.is_some();
            if self.extensions.active.dialog.is_none() {
                self.extensions.dialog_return_focus = None;
            }
        } else if !snapshot.history_preview && self.snapshot.history_preview {
            dirty.root = true;
            self.clear_restored_dialog();
            restore_extension_after_history(
                &mut self.extensions.active,
                &mut self.extensions.parked,
            );
            self.extensions.pending_dialog_setup = self.extensions.active.dialog.is_some();
            if self.extensions.active.dialog.is_none() {
                self.extensions.dialog_return_focus = None;
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
        self.sessions.generation = generation;
        self.reconcile_pending_session_titles(&mut sessions, &mut all_sessions);
        let catalog_changed = session_catalog_changed(
            &self.sessions.visible,
            &self.sessions.all,
            self.sessions.error.as_deref(),
            &sessions,
            &all_sessions,
        );
        let archived_catalog_changed = inactive_session_catalog_changed(
            &self.sessions.visible,
            &self.sessions.all,
            &sessions,
            &all_sessions,
        );
        let run_catalog_changed = run_panel_sessions_changed(
            &self.sessions.all,
            &all_sessions,
            self.snapshot.selected_session.as_deref(),
        );
        let composer_usage_changed = composer_usage_sessions_changed(
            &self.sessions.all,
            &all_sessions,
            self.snapshot.selected_session.as_deref(),
        );
        let previous_workgraph_session = self.active_workgraph_session();
        let visible_activities_changed = run_panel_activities_changed(
            &self.activity.agents,
            activities.as_ref(),
            &self.sessions.all,
            self.snapshot.selected_session.as_deref(),
        );
        for session in &all_sessions {
            projects::add_visible(
                &mut self.project.registered,
                &self.project.excluded,
                session.project.clone(),
            );
        }
        self.sessions.error = None;
        self.sessions.visible = sessions;
        self.sessions.all = all_sessions;
        if let Some((activities, _exhaustive)) = activities {
            dirty.rail = true;
            for activity in activities.into_values() {
                dirty.run |= merge_agent_activity(
                    &mut self.activity.agents,
                    activity,
                    ActivityUpdateSource::Catalog,
                );
            }
        }
        let agent_ids = agent_focus_keys(&self.sessions.all, &self.activity.agents);
        self.activity
            .row_focus
            .retain(|id, _| agent_ids.contains(id));
        for id in agent_ids {
            self.activity
                .row_focus
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
            .sessions
            .drafts
            .iter()
            .filter(|draft| {
                draft
                    .session_path
                    .as_ref()
                    .is_some_and(|path| paths.contains(path))
            })
            .map(|draft| draft.id.clone())
            .chain(
                self.sessions
                    .submitted_drafts
                    .iter()
                    .filter_map(|(id, path)| {
                        path.as_ref()
                            .is_some_and(|path| paths.contains(path))
                            .then_some(id.clone())
                    }),
            )
            .collect::<HashSet<_>>();
        for path in paths.iter() {
            let target = session_target(path);
            self.workspace.editor.session_tabs.remove(&target);
            self.composer.sessions.remove(&target);
            self.workspace.session_surfaces.remove(&target);
            self.composer.images.remove(&target);
            self.composer.pastes.remove(&target);
            self.composer
                .pending_submissions
                .retain(|_, pending| pending.submitted_target != target);
            self.activity.run_statuses.remove(&target);
            self.activity.recent_completions.remove(&target);
            self.activity.recent_completion_expiries.remove(&target);
        }
        for id in &deleted_draft_ids {
            let target = draft_target(id);
            self.workspace.editor.session_tabs.remove(&target);
            self.composer.sessions.remove(&target);
            self.workspace.session_surfaces.remove(&target);
            self.composer.images.remove(&target);
            self.composer.pastes.remove(&target);
            self.composer
                .pending_submissions
                .retain(|_, pending| pending.submitted_target != target);
            self.sessions.submitted_drafts.remove(id);
            self.sessions.draft_session_ids.remove(id);
            self.activity.run_statuses.remove(&target);
            self.activity.recent_completions.remove(&target);
            self.activity.recent_completion_expiries.remove(&target);
        }
        if !deleted_draft_ids.is_empty() {
            self.sessions
                .drafts
                .retain(|draft| !deleted_draft_ids.contains(&draft.id));
            if self
                .sessions
                .selected_draft
                .as_ref()
                .is_some_and(|id| deleted_draft_ids.contains(id))
            {
                self.sessions.selected_draft = None;
            }
            for id in &deleted_draft_ids {
                self.remove_session_draft(id);
            }
        }
        self.activity
            .system_notification_targets
            .retain(|_, (path, _)| !paths.contains(path));
        if self
            .lifecycle
            .pending_session_switch
            .as_ref()
            .is_some_and(|(path, _)| paths.contains(path))
        {
            drop(self.lifecycle.pending_session_switch.take());
        }
        if selected_was_deleted && generation >= self.runtime_generation {
            let current_target = self.composer.sessions.current_target().to_owned();
            let (next_target, next_draft) = match session::draft_store::new(
                self.project.path.clone(),
                self.sessions.preferred_harness,
            ) {
                Ok(draft) => (draft_target(&draft.id), Some(draft)),
                Err(error) => {
                    self.sessions.error = Some(error);
                    (project_target(&self.project.path), None)
                }
            };
            let composer = self
                .composer
                .sessions
                .discard_and_switch(&current_target, next_target.clone());
            self.hide_native_workspace_surfaces(cx);
            if self.workspace.surface != AppSurface::Work {
                self.set_surface(AppSurface::Chat, cx);
            }
            self.reset_session_ui(generation, false, cx);
            self.composer.pending_restore = Some((next_target, composer));
            self.sessions.selected_draft = next_draft.as_ref().map(|draft| draft.id.clone());
            if let Some(draft) = next_draft {
                self.sessions
                    .draft_session_ids
                    .insert(draft.id.clone(), draft.app_session_id);
                self.sessions.drafts.push(draft.clone());
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
        let mut changed_drafts = Vec::new();
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
                self.composer
                    .sessions
                    .promote(&source_target, target_target.clone());
            }
            self.promote_center_surface(&source_target, &target_target);
            if let Some(images) = self.composer.images.remove(&source_target) {
                self.composer.images.insert(target_target.clone(), images);
            }
            self.promote_composer_pastes(&source_target, &target_target);
            if let Some(status) = self.activity.run_statuses.remove(&source_target) {
                self.activity
                    .run_statuses
                    .insert(target_target.clone(), status);
            }
            if let Some(completion) = self.activity.recent_completions.remove(&source_target) {
                self.activity
                    .recent_completions
                    .insert(target_target.clone(), completion);
            }
            if let Some(expiry) = self
                .activity
                .recent_completion_expiries
                .remove(&source_target)
            {
                self.activity
                    .recent_completion_expiries
                    .insert(target_target.clone(), expiry);
            }
            for draft in &mut self.sessions.drafts {
                if draft.session_path.as_deref() == Some(source.as_path()) {
                    draft.session_path = Some(target.clone());
                    draft.project = target_project.clone();
                    changed_drafts.push(draft.id.clone());
                }
            }
            for session_path in self.sessions.submitted_drafts.values_mut().flatten() {
                if session_path == source {
                    *session_path = target.clone();
                }
            }
        }
        for id in changed_drafts {
            self.save_session_draft(&id);
        }
        for (session, project) in self.activity.system_notification_targets.values_mut() {
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
        if let Some(extension) = self.extensions.parked.as_mut() {
            let _ = extension.apply(request);
        } else {
            self.apply_extension_request(request, generation, cx);
            dirty.root = true;
            dirty.composer = true;
        }
    }
    fn project_prompt_result(
        &mut self,
        submission_id: Option<String>,
        target: String,
        outcome: crate::agents::PromptOutcome,
        session: Option<PathBuf>,
        dirty: &mut DirtyRegions,
        cx: &mut Context<Self>,
    ) {
        let accepted = outcome == crate::agents::PromptOutcome::Accepted;
        self.code_task_result(&target, accepted, session.as_deref(), cx);
        self.record_draft_submission(&target, accepted, session.clone());
        if outcome == crate::agents::PromptOutcome::RejectedBeforeAcceptance {
            self.activity
                .run_statuses
                .insert(target.clone(), "Failed".into());
        }
        record_pending_prompt_result_for_submission(
            &mut self.composer.pending_submissions,
            submission_id.as_deref(),
            &target,
            outcome,
            session,
        );
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
            } if generation >= self.sessions.generation => {
                self.project_sessions(generation, sessions, all_sessions, activities, dirty, cx);
            }
            RuntimeEvent::SessionUpdated(mut session) => {
                self.reconcile_pending_session_titles(&mut [], std::slice::from_mut(&mut session));
                dirty.archived_rail |= archive::session_event_affects_archived_rail(
                    &self.sessions.all,
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
                    &mut self.activity.agents,
                    activity,
                    ActivityUpdateSource::Metadata,
                );
                let activity_key = crate::agent_activity::agent_activity_key(&session.path);
                self.activity
                    .row_focus
                    .entry(activity_key)
                    .or_insert_with(|| cx.focus_handle());
                projects::add_visible(
                    &mut self.project.registered,
                    &self.project.excluded,
                    session.project.clone(),
                );
                dirty.composer |= self.snapshot.selected_session.as_ref() == Some(&session.path);
                update_session_row(&mut self.sessions.all, session.clone());
                dirty.archived_rail |= archive::session_event_affects_archived_rail(
                    &self.sessions.all,
                    "",
                    Some(&session.path),
                );
                let query = self.navigation.search.read(cx).value();
                if query.trim().is_empty() {
                    update_session_row(&mut self.sessions.visible, session);
                } else {
                    self.sessions.visible = crate::sessions::filter_session_tree(
                        self.sessions.all.clone(),
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
                    &mut self.activity.agents,
                    activity,
                    ActivityUpdateSource::Native,
                );
                dirty.rail = true;
                self.activity
                    .row_focus
                    .entry(activity_key)
                    .or_insert_with(|| cx.focus_handle());
                dirty.run |= self
                    .sessions
                    .all
                    .iter()
                    .find(|session| {
                        crate::sessions::normalize_session_path(&session.path)
                            == crate::sessions::normalize_session_path(&activity_path)
                    })
                    .and_then(|session| {
                        root_session_for_path(&self.sessions.all, Some(&session.path))
                    })
                    .is_some_and(|root| {
                        self.snapshot.selected_session.as_deref() == Some(root.path.as_path())
                            || root_session_for_path(
                                &self.sessions.all,
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
            } if generation >= self.sessions.generation => {
                self.sessions.generation = generation;
                let changed = self.sessions.error.as_deref() != Some(message.as_str());
                self.sessions.error = Some(message);
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
                    &mut self.extensions.active,
                    self.extensions.parked.as_mut(),
                    &mut self.extensions.restored_dialog_id,
                    &mut self.extensions.dismissed_restored_dialog_id,
                    &mut self.extensions.pending_dialog_setup,
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
            RuntimeEvent::TurnCompletedNotification { body, target } => {
                if !completion_notification_is_redundant(
                    cx.active_window().is_some(),
                    target.as_ref(),
                    &self.snapshot,
                ) {
                    self.show_attention_notification(
                        TURN_COMPLETED_NOTIFICATION_TITLE,
                        &body,
                        target,
                        cx,
                    );
                }
            }
            RuntimeEvent::PromptResult {
                submission_id,
                target,
                outcome,
                session,
            } => {
                // Replies belong to a submission, even after navigation changes generations.
                self.project_prompt_result(submission_id, target, outcome, session, dirty, cx);
            }
            RuntimeEvent::SessionStatus {
                target,
                session,
                status,
            } => {
                if status == "Stopped" {
                    let session_key = session.as_deref().map(session_target);
                    for (key, pending) in &mut self.composer.pending_submissions {
                        if pending.submitted_target == target || Some(key) == session_key.as_ref() {
                            pending.result.get_or_insert((
                                crate::agents::PromptOutcome::RejectedBeforeAcceptance,
                                session.clone(),
                            ));
                        }
                    }
                    if let Some(path) = session.as_deref() {
                        for row in self
                            .sessions
                            .visible
                            .iter_mut()
                            .chain(self.sessions.all.iter_mut())
                        {
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
                self.workspace
                    .code_tasks
                    .associate(&target, session.as_deref());
                dirty.root |= self.workspace.code_tasks.notice_message().is_some();
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
    park_extension_surface(visible, parked);
}

fn restore_extension_after_history(
    visible: &mut crate::app::extensions::ExtensionUiState,
    parked: &mut Option<crate::app::extensions::ExtensionUiState>,
) {
    restore_extension_surface(visible, parked);
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
    if let Some(parked) = parked_extension {
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

pub(in crate::app) fn record_pending_prompt_result_for_submission(
    pending: &mut HashMap<String, PendingSubmission>,
    submission_id: Option<&str>,
    target: &str,
    outcome: crate::agents::PromptOutcome,
    session: Option<PathBuf>,
) {
    // A missing receipt is still pending work. Keep the composer submission
    // unresolved so the durable outbox can retry it after a restart.
    if outcome == crate::agents::PromptOutcome::DeliveryUnknown {
        return;
    }
    let key = match submission_id {
        Some(id) => pending.contains_key(id).then(|| id.to_owned()),
        None => {
            let mut matches = pending.iter().filter(|(_, pending)| {
                pending.submitted_target == target && pending.result.is_none()
            });
            let first = matches.next().map(|(id, _)| id.clone());
            first.filter(|_| matches.next().is_none())
        }
    };
    if let Some(pending) = key.and_then(|id| pending.get_mut(&id)) {
        if pending.result.as_ref().is_some_and(|(previous, _)| {
            matches!(
                previous,
                crate::agents::PromptOutcome::Accepted
                    | crate::agents::PromptOutcome::RejectedBeforeAcceptance
                    | crate::agents::PromptOutcome::Cancelled
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
            run: self.lifecycle.performance_monitor.as_mut().is_some_and(
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
