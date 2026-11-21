use crate::runtime::RunStatus;

use super::*;

fn merge_keyed_activity(
    activities: &mut HashMap<crate::agent_activity::WorkerActivityKey, AgentActivity>,
    key: crate::agent_activity::WorkerActivityKey,
    incoming: AgentActivity,
) -> bool {
    let Some(existing) = activities.get_mut(&key) else {
        activities.insert(key, incoming);
        return true;
    };
    if (matches!(
        incoming.lifecycle,
        crate::agent_activity::AgentLifecycle::Unknown
    ) && !matches!(
        existing.lifecycle,
        crate::agent_activity::AgentLifecycle::Unknown
    )) || (!incoming.explicit_outcome
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
    activities: &HashMap<crate::agent_activity::WorkerActivityKey, AgentActivity>,
) -> HashSet<crate::agent_activity::WorkerActivityKey> {
    sessions
        .iter()
        .filter(|session| {
            session.parent_app_session_id.is_some() || session.parent_session.is_some()
        })
        .map(SessionSummary::activity_key)
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
    archive_keys: bool,
    transcript: bool,
    composer: bool,
    run: bool,
    worker_rows: bool,
    workgraph_session: bool,
    workgraph_goal: bool,
}

impl DirtyRegions {
    fn archive_change(&mut self, change: ArchiveChange) {
        self.archive_keys |= change == ArchiveChange::Rebuild;
        self.archived_rail |= change == ArchiveChange::Repaint;
    }

    fn observe_snapshot(&mut self, app: &FarcasterApp, snapshot: &RuntimeSnapshot) {
        self.transcript |= app.lifecycle.pending_session_switch.is_some()
            || (app.snapshot.status == "Loading history") != (snapshot.status == "Loading history");
        let roots = &app.sessions.visible;
        self.rail |= session_rail_snapshot_changed(roots, &app.snapshot, snapshot);
        self.archived_rail |=
            inactive_session_rail_snapshot_changed(&app.sessions.all, &app.snapshot, snapshot);
        self.composer |= composer_snapshot_changed(&app.snapshot, snapshot);
        self.root |= app.snapshot.pending_question != snapshot.pending_question;
        self.run |= run_panel_snapshot_changed(&app.sessions.all, &app.snapshot, snapshot);
        let previous_root = app
            .sessions
            .all
            .root_for_path(app.snapshot.selected_session.as_deref());
        let next_root = app
            .sessions
            .all
            .root_for_path(snapshot.selected_session.as_deref());
        self.workgraph_session |=
            previous_root.map(|root| &root.path) != next_root.map(|root| &root.path);
        self.workgraph_goal |= app.snapshot.session_goal != snapshot.session_goal;
    }

    fn observe(&mut self, app: &FarcasterApp, event: &RuntimeEvent) {
        match event {
            RuntimeEvent::AccountsChanged(_) => {
                self.rail = true;
                self.root = true;
            }
            RuntimeEvent::Snapshot { .. }
            | RuntimeEvent::Sessions { .. }
            | RuntimeEvent::DraftPromptCommitted { .. }
            | RuntimeEvent::DraftIdentitySaved { .. }
            | RuntimeEvent::SessionArchiveResult { .. }
            | RuntimeEvent::SessionUpdated(_)
            | RuntimeEvent::AgentActivityUpdated { .. }
            | RuntimeEvent::SelectionRouted(_)
            | RuntimeEvent::SelectionRejected(_)
            | RuntimeEvent::WorkerSnapshot(_)
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
                self.archive_keys = true;
                self.transcript = true;
                self.composer = true;
            }
            RuntimeEvent::SessionStatus {
                target,
                session,
                record,
                ..
            } => {
                if let Some(root) = record
                    .as_ref()
                    .and_then(|record| record.resolve(&app.sessions.visible))
                    .and_then(|row| app.sessions.visible.root_for_row(row))
                {
                    self.rail |= !root.archived;
                    self.archived_rail |= root.archived;
                }
                if let Some(id) = sessions::draft_id(target) {
                    self.archived_rail |= app
                        .sessions
                        .drafts
                        .iter()
                        .any(|draft| draft.id == id && draft.archived);
                }
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

    fn worker_changed(&mut self, app: &FarcasterApp, path: &Path, cx: &Context<FarcasterApp>) {
        for row in app.sessions.all.at_path(path) {
            if let Some(root) = app.sessions.all.root_for_row(row) {
                self.worker_family_changed(app, &root.catalog_key(), cx);
            }
        }
    }

    fn worker_family_changed(
        &mut self,
        app: &FarcasterApp,
        root: &sessions::SessionKey,
        cx: &Context<FarcasterApp>,
    ) {
        self.worker_rows |= app.views.run_panel.read(cx).has_prepared_workers_for(root);
        self.run |= app
            .sidebar_worker_root()
            .is_some_and(|row| &row.catalog_key() == root);
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
        if self.archive_keys {
            app.notify_archived_session_rail(cx);
        } else if self.archived_rail {
            app.repaint_archived_session_rail(cx);
        }
        if self.transcript {
            app.notify_transcript(cx);
        }
        if self.composer {
            app.notify_composer(cx);
        }
        if self.worker_rows {
            app.views.run_panel.read(cx).invalidate_workers();
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
    pub(in crate::app) fn restore_navigation_snapshot(
        &mut self,
        generation: u64,
        snapshot: Arc<RuntimeSnapshot>,
        cx: &mut Context<Self>,
    ) {
        let mut dirty = DirtyRegions::default();
        if generation >= self.runtime_generation {
            self.project_snapshot(generation, snapshot, &mut dirty, cx);
        }
        dirty.notify(self, cx);
    }

    fn project_snapshot(
        &mut self,
        generation: u64,
        snapshot: Arc<RuntimeSnapshot>,
        dirty: &mut DirtyRegions,
        cx: &mut Context<Self>,
    ) {
        crate::app::session::remembered_transcript::remember(&snapshot);
        // Events already in flight belong to the previous selection. The
        // supervisor publishes the routed request before its selected snapshot.
        if self.lifecycle.awaiting_switch_route.is_some() {
            if let Some(recovery) = &mut self.lifecycle.switch_recovery {
                recovery.snapshot = Some((generation, snapshot));
            }
            return;
        }
        if let Some(id) = &self.lifecycle.awaiting_draft_snapshot
            && self.sessions.selected_draft.as_ref() == Some(id)
            && let Some(draft) = self.sessions.drafts.iter().find(|row| &row.id == id)
        {
            let path = self.sidebar_session_path();
            let matches = match self.sidebar_catalog_row(&self.sessions.all) {
                Some(row) => snapshot.matches_session_row(row),
                None => {
                    draft.project == snapshot.project
                        && draft.harness == snapshot.harness
                        && draft.profile_id == snapshot.profile_id
                }
            };
            if !matches || path != snapshot.selected_session.as_deref() {
                return;
            }
            self.lifecycle.awaiting_draft_snapshot = None;
            dirty.composer = true;
        }
        dirty.observe_snapshot(self, &snapshot);
        let snapshot = crate::app::session::remembered_transcript::stand_in(snapshot);
        if self.pending_switch_matches(&snapshot) {
            drop(self.lifecycle.pending_session_switch.take());
            self.lifecycle.switch_recovery = None;
            dirty.composer = true;
        }
        let session_changed = generation > self.runtime_generation;
        let transcript_preselected =
            session_changed && self.snapshot.selected_session == snapshot.selected_session;
        if session_changed {
            self.reset_session_ui(generation, transcript_preselected, cx);
            dirty.root = true;
        }
        let previous_items = self.snapshot.transcript_presentation().items.clone();
        let tail_changed = match (&self.snapshot.history_tail, &snapshot.history_tail) {
            (Some(previous), Some(next)) => !Arc::ptr_eq(previous, next),
            (None, None) => false,
            _ => true,
        };
        let replacing_stand_in = self.snapshot.status == "Loading history"
            && self.snapshot.selected_session == snapshot.selected_session
            && !self.snapshot.conversation.items.is_empty()
            && (snapshot.status != "Loading history" || snapshot.conversation.items.is_empty());
        let same_ready_document = !session_changed
            && snapshot.document_ready
            && snapshot.status == "Ready"
            && self.snapshot.selected_session == snapshot.selected_session
            && self.snapshot.project == snapshot.project
            && self.snapshot.harness == snapshot.harness
            && self.snapshot.profile_id == snapshot.profile_id;
        let history_handoff = same_ready_document
            && self.awaiting_history_rows()
            && self.snapshot.history_tail.is_some();
        let transcript_changed = self.views.transcript.update(cx, |transcript, _| {
            if same_ready_document && self.snapshot.saved_transcript && !snapshot.saved_transcript {
                transcript.prepare_saved_history_append(
                    &previous_items,
                    &snapshot.transcript_presentation().items,
                );
            }
            if replacing_stand_in {
                // Save the complete stand-in's reading position at the handoff.
                transcript.reset(&previous_items);
            }
            let changed =
                transcript.select_session(snapshot.selected_session.as_deref(), &previous_items);
            changed || replacing_stand_in
        });
        dirty.transcript |= tail_changed;
        let row_update = if transcript_changed || (session_changed && !transcript_preselected) {
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
        let picker_choices_changed = self.snapshot.harness != snapshot.harness
            || self.snapshot.models != snapshot.models
            || self.snapshot.configuration_status != snapshot.configuration_status
            || self.snapshot.thinking_levels != snapshot.thinking_levels
            || self.snapshot.sandbox_adapter != snapshot.sandbox_adapter
            || self.snapshot.access_mode != snapshot.access_mode
            || self.snapshot.session_identity() != snapshot.session_identity();
        self.snapshot = snapshot;
        if let Some(session) = self.snapshot.selected_session.as_ref()
            && let Some(progress) = self.extensions.question_sessions.remove(session)
        {
            self.extensions.active.question_progress.extend(progress);
        }
        if picker_choices_changed {
            self.refresh_configuration_picker(cx);
        }
        dirty.transcript |= self.apply_transcript_rows(row_update, &previous_items, cx);
        if tail_changed || transcript_changed || session_changed {
            self.views.transcript.update(cx, |transcript, _| {
                if history_handoff {
                    transcript.preserve_painted_history(
                        &self.snapshot.transcript_presentation().items,
                        self.snapshot.saved_transcript,
                    );
                }
                transcript.set_history_tail(
                    self.snapshot
                        .history_tail
                        .as_deref()
                        .filter(|_| self.snapshot.conversation.items.is_empty()),
                    &self.snapshot.project,
                );
            });
        }
        self.sync_restored_dialog();
        self.sync_composer_history();
        dirty.rail |= self.reconcile_submitted_drafts(cx);
    }
    fn update_worker_catalog_row(
        &mut self,
        session: &SessionSummary,
        dirty: &mut DirtyRegions,
        cx: &Context<Self>,
    ) {
        let catalog = &self.sessions.all;
        let previous = catalog.matching_record(session);
        let identity = session.native_identity();
        let previous_identity = previous
            .map(SessionSummary::native_identity)
            .filter(|previous| *previous != identity);
        let parent_identities = [Some(&identity), previous_identity.as_ref()];
        // Updating one row may transfer workers linked by native ID between two
        // other roots. Compare their resolved families across the catalog edit.
        let native_families = |catalog: &sessions::SessionCatalog| {
            parent_identities.map(|id| {
                let parent = catalog.native_worker_parent(id?)?;
                Some(catalog.root_for_row(parent)?.catalog_key())
            })
        };
        let before_families = native_families(catalog);
        let selected_before = self.sidebar_worker_root().map(SessionSummary::catalog_key);
        if let Some(root) = previous.and_then(|row| catalog.root_for_row(row)) {
            dirty.worker_family_changed(self, &root.catalog_key(), cx);
        }
        self.sessions.all.upsert_by_recency(session.clone());
        let catalog = &self.sessions.all;
        if let Some(root) = catalog.root_for_row(session) {
            dirty.worker_family_changed(self, &root.catalog_key(), cx);
        }
        for (before, after) in before_families.into_iter().zip(native_families(catalog)) {
            if before != after {
                dirty.archived_rail = true;
                for root in before.iter().chain(&after) {
                    dirty.worker_family_changed(self, root, cx);
                }
            }
        }
        // An orphan and its subtree can acquire a newly published parent even
        // though the edited row had no previous family to invalidate.
        dirty.run |= self.sidebar_worker_root().map(SessionSummary::catalog_key) != selected_before;
        if let Some(prepared) = self.views.run_panel.read(cx).prepared_worker_root() {
            dirty.worker_rows |= catalog
                .by_key(&prepared)
                .and_then(|row| catalog.root_for_row(row))
                .is_none_or(|root| root.catalog_key() != prepared);
        }
    }

    fn adopt_catalog_draft(&mut self, row: &SessionSummary, dirty: &mut DirtyRegions) {
        if let (Some(draft), Some(id)) = (
            &row.draft_id,
            sessions::AppSessionId::new(row.app_session_id),
        ) {
            // A full refresh can see the bound session before the outbox receipt.
            self.confirm_draft_submission(
                draft,
                (!row.title.is_empty()).then_some(row.title.as_str()),
            );
            if self.adopt_draft_identity(draft.clone(), id) {
                dirty.rail = true;
                dirty.archive_keys = true;
            }
        }
    }

    fn project_sessions(
        &mut self,
        generation: u64,
        mut sessions: Vec<SessionSummary>,
        mut all_sessions: Vec<SessionSummary>,
        activities: Option<(
            HashMap<crate::agent_activity::WorkerActivityKey, AgentActivity>,
            bool,
        )>,
        dirty: &mut DirtyRegions,
        cx: &mut Context<Self>,
    ) {
        self.sessions.generation = generation;
        if self.sessions.archive_writes.needs_overlay() {
            for row in all_sessions.iter_mut().chain(sessions.iter_mut()) {
                self.sessions
                    .archive_writes
                    .overlay(row, &self.sessions.all);
            }
        }
        self.reconcile_pending_session_titles(&mut sessions, &mut all_sessions);
        let sessions = sessions::SessionCatalog::from(sessions);
        let all_sessions = sessions::SessionCatalog::from(all_sessions);
        let catalog_changed = session_catalog_changed(
            &self.sessions.visible,
            &self.sessions.all,
            self.sessions.error.as_deref(),
            &sessions,
            &all_sessions,
        );
        let archive_change = archive_catalog_change(
            &self.sessions.visible,
            &self.sessions.all,
            &sessions,
            &all_sessions,
        );
        let previous_root = self.sidebar_worker_root().map(SessionSummary::catalog_key);
        let composer_usage_changed = composer_usage_sessions_changed(
            &self.sessions.all,
            &all_sessions,
            self.snapshot.selected_session.as_deref(),
        );
        let previous_workgraph_session = self.active_workgraph_session();
        let graph_identities_changed =
            session_identities_changed(&self.sessions.all, &all_sessions);
        for session in all_sessions.iter() {
            self.adopt_catalog_draft(session, dirty);
            projects::add_visible(
                &mut self.project.registered,
                &self.project.excluded,
                session.project.clone(),
            );
        }
        self.sessions.error = None;
        self.sessions.visible = sessions;
        let previous_catalog = std::mem::replace(&mut self.sessions.all, all_sessions);
        let catalog = &self.sessions.all;
        let current_root = self.sidebar_worker_root();
        let selected_root = current_root.map(SessionSummary::catalog_key);
        let run_catalog_changed = run_panel_family_changed(
            &previous_catalog,
            catalog,
            previous_root
                .as_ref()
                .and_then(|key| previous_catalog.by_key(key)),
            current_root,
        );
        let prepared_root = self.views.run_panel.read(cx).prepared_worker_root();
        dirty.worker_rows |= prepared_root.as_ref().is_some_and(|key| {
            if Some(key) == selected_root.as_ref() && Some(key) == previous_root.as_ref() {
                run_catalog_changed
            } else {
                run_panel_family_changed(
                    &previous_catalog,
                    catalog,
                    previous_catalog
                        .by_key(key)
                        .and_then(|row| previous_catalog.root_for_row(row)),
                    catalog
                        .by_key(key)
                        .and_then(|row| catalog.root_for_row(row)),
                )
            }
        });
        self.remember_rail_projects(
            self.sessions
                .all
                .iter()
                .map(|session| session.project.clone())
                .collect::<Vec<_>>(),
        );
        let catalog = &self.sessions.all;
        let mut changed_worker = |key: &crate::agent_activity::WorkerActivityKey| {
            if let Some(root) = catalog
                .worker_for_activity(key)
                .and_then(|row| catalog.root_for_row(row))
            {
                let root = root.catalog_key();
                dirty.worker_rows |= prepared_root.as_ref() == Some(&root);
                dirty.run |= selected_root.as_ref() == Some(&root);
            }
        };
        if let Some((activities, _exhaustive)) = activities {
            dirty.rail = true;
            for (key, activity) in activities {
                if merge_keyed_activity(&mut self.activity.agents, key.clone(), activity) {
                    changed_worker(&key);
                }
            }
        }
        let agent_ids = agent_focus_keys(catalog, &self.activity.agents);
        self.activity
            .row_focus
            .retain(|id, _| agent_ids.contains(id));
        for id in agent_ids {
            if let std::collections::hash_map::Entry::Vacant(entry) =
                self.activity.row_focus.entry(id)
            {
                changed_worker(entry.key());
                entry.insert(cx.focus_handle());
            }
        }
        dirty.rail |= catalog_changed;
        dirty.archive_change(archive_change);
        dirty.composer |= composer_usage_changed;
        dirty.run |= run_catalog_changed;
        dirty.workgraph_session |= graph_identities_changed
            || previous_workgraph_session != self.active_workgraph_session();
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
            self.workspace.session_surfaces.remove(&target);
            for buffer in self.composer.sessions.buffers_for_target(&target) {
                self.composer.sessions.remove(&buffer);
                self.composer.images.remove(&buffer);
                self.composer.pastes.remove(&buffer);
            }
            self.composer.pending_submissions.retain(|_, pending| {
                sessions::composer_command_target(&pending.submitted_target) != target
            });
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
            self.sessions.pending_draft_saves.remove(id);
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
            self.cancel_pending_session_switch();
        }
        if selected_was_deleted && generation >= self.runtime_generation {
            let current_target = self.composer.sessions.current_buffer().to_owned();
            let draft = session::draft_store::new(
                self.project.path.clone(),
                self.sessions.preferred_harness,
                self.sessions.preferred_profile_id.clone(),
            );
            let next_target = draft_target(&draft.id);
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
            self.sessions.selected_draft = Some(draft.id.clone());
            self.sessions
                .draft_session_ids
                .insert(draft.id.clone(), draft.app_session_id);
            self.sessions.drafts.push(draft.clone());
            self.send(
                RuntimeCommand::NewSession {
                    request: None,
                    id: draft.id,
                    harness: draft.harness,
                    profile_id: draft.profile_id,
                    project: draft.project,
                },
                cx,
            );
            let snapshot = Arc::make_mut(&mut self.snapshot);
            snapshot.live_session = None;
            snapshot.selected_session = None;
            snapshot.session = None;
            snapshot.conversation = Arc::default();
            snapshot.history_preview = false;
            snapshot.pending_question = None;
        }
        self.extensions
            .question_sessions
            .retain(|path, _| !paths.contains(path));
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
                for old in self.composer.sessions.buffers_for_target(&source_target) {
                    let new = sessions::composer_buffer_parts(&old).map_or_else(
                        || target_target.clone(),
                        |(harness, _)| sessions::session_buffer(harness, target),
                    );
                    self.composer.sessions.promote(&old, new.clone());
                    self.promote_composer_images(&old, &new);
                    self.promote_composer_pastes(&old, &new);
                }
            }
            self.promote_center_surface(&source_target, &target_target);
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
                    key: None,
                    request: None,
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
        self.workspace.voice.submission_result(
            submission_id.as_deref(),
            accepted,
            session.as_deref(),
        );
        self.code_task_result(
            submission_id.as_deref(),
            &target,
            accepted,
            session.as_deref(),
            cx,
        );
        self.record_draft_submission(&target, accepted, session.clone());
        if outcome == crate::agents::PromptOutcome::RejectedBeforeAcceptance
            && !submission_id
                .as_ref()
                .and_then(|id| self.composer.pending_submissions.get(id))
                .is_some_and(|rejected| {
                    self.composer.pending_submissions.values().any(|other| {
                        other.submitted_target == rejected.submitted_target
                            && super::composer::submissions::submission_order(other, rejected)
                                .is_gt()
                    })
                })
        {
            self.activity
                .run_statuses
                .insert(target.clone(), RunStatus::Failed);
        }
        if submission_id.is_some()
            || sessions::session_path(&target)
                .is_none_or(|path| !self.sessions.all.has_shared_path(path))
        {
            record_pending_prompt_result_for_submission(
                &mut self.composer.pending_submissions,
                submission_id.as_deref(),
                &target,
                outcome,
                session,
            );
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
            RuntimeEvent::SelectionRejected(sequence) => {
                self.finish_selection_route(sequence, false, cx);
            }
            RuntimeEvent::SelectionRouted(sequence) => {
                self.finish_selection_route(sequence, true, cx);
            }
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
                // Keep the displayed tail or stand-in until the replacement snapshot.
                if self.snapshot.status != "Loading history" {
                    self.reset_transcript_ui(cx);
                }
            }
            RuntimeEvent::Sessions {
                generation,
                sessions,
                all_sessions,
                activities,
            } if generation >= self.sessions.generation => {
                self.project_sessions(generation, sessions, all_sessions, activities, dirty, cx);
            }
            RuntimeEvent::DraftPromptCommitted {
                draft,
                app_session_id,
                provisional_title,
            } => {
                self.confirm_draft_submission(&draft, provisional_title.as_deref());
                dirty.archive_keys |= self.adopt_draft_identity(draft, app_session_id);
                dirty.rail = true;
                dirty.composer = true;
            }
            RuntimeEvent::DraftIdentitySaved {
                draft,
                app_session_id,
            } => {
                if self.adopt_draft_identity(draft, app_session_id) {
                    dirty.rail = true;
                    dirty.archive_keys = true;
                }
            }
            RuntimeEvent::SessionArchiveResult { request, rows } => {
                self.finish_archive(request, rows, cx);
            }
            RuntimeEvent::SessionUpdated(mut session) => {
                self.adopt_catalog_draft(&session, dirty);
                self.sessions
                    .archive_writes
                    .overlay(&mut session, &self.sessions.all);
                self.reconcile_pending_session_titles(&mut [], std::slice::from_mut(&mut session));
                dirty.archive_change(archive_row_change(&self.sessions.all, &session));
                let previous_workgraph_session = self.active_workgraph_session();
                // Worker cards derive fallback activity from the current catalog row.
                // Only pool and native events supply an independent activity record.
                if session.parent_session.is_some() || session.parent_app_session_id.is_some() {
                    self.activity
                        .row_focus
                        .entry(session.activity_key())
                        .or_insert_with(|| cx.focus_handle());
                }
                projects::add_visible(
                    &mut self.project.registered,
                    &self.project.excluded,
                    session.project.clone(),
                );
                dirty.composer |= self.snapshot.selected_session.as_ref() == Some(&session.path);
                self.update_worker_catalog_row(&session, dirty, cx);
                let query = self.navigation.search.read(cx).value();
                if query.trim().is_empty() {
                    self.sessions.visible.upsert_by_recency(session);
                } else {
                    let visible = crate::sessions::filter_session_tree(
                        self.sessions.all.to_vec(),
                        query.trim(),
                    )
                    .into();
                    dirty.archive_change(archive_catalog_change(
                        &self.sessions.visible,
                        &self.sessions.all,
                        &visible,
                        &self.sessions.all,
                    ));
                    self.sessions.visible = visible;
                }
                dirty.rail = true;
                dirty.workgraph_session |=
                    previous_workgraph_session != self.active_workgraph_session();
                dirty.rail |= self.reconcile_submitted_drafts(cx);
            }
            RuntimeEvent::AgentActivityUpdated {
                key: activity_key,
                activity,
            } => {
                let changed =
                    merge_keyed_activity(&mut self.activity.agents, activity_key.clone(), activity);
                let new_focus = !self.activity.row_focus.contains_key(&activity_key);
                if (changed || new_focus)
                    && let Some(root) = self
                        .sessions
                        .all
                        .worker_for_activity(&activity_key)
                        .and_then(|row| self.sessions.all.root_for_row(row))
                {
                    dirty.worker_family_changed(self, &root.catalog_key(), cx);
                }
                self.activity
                    .row_focus
                    .entry(activity_key)
                    .or_insert_with(|| cx.focus_handle());
                dirty.rail = true;
            }
            RuntimeEvent::SessionDeleted { generation, paths } => {
                for path in paths.iter() {
                    dirty.worker_changed(self, path, cx);
                }
                dirty.run |= self
                    .sidebar_session_path()
                    .is_some_and(|path| paths.contains(path));
                self.project_session_deleted(generation, paths.clone(), cx);
                let mut completed = Vec::new();
                for (folder, remaining) in &mut self.sessions.deleting_folders {
                    remaining.retain(|path| !paths.contains(path));
                    if remaining.is_empty() {
                        completed.push(*folder);
                    }
                }
                if !completed.is_empty() {
                    let mut next = self.sessions.folders.clone();
                    for folder in completed {
                        self.sessions.deleting_folders.remove(&folder);
                        next.remove(folder);
                    }
                    self.save_session_folders(next, cx);
                }
            }
            RuntimeEvent::SessionMoved {
                target,
                target_project,
                paths,
            } => {
                for (source, destination) in paths.iter() {
                    dirty.worker_changed(self, source, cx);
                    if destination != source {
                        dirty.worker_changed(self, destination, cx);
                    }
                }
                dirty.run |= self
                    .sidebar_session_path()
                    .is_some_and(|path| paths.contains_key(path));
                self.project_session_moved(target, target_project, paths, cx);
            }
            RuntimeEvent::SessionsFailed {
                generation,
                message,
            } if generation >= self.sessions.generation => {
                self.sessions.deleting_folders.clear();
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
                self.speak_voice_reply(&body, target.as_ref(), cx);
                let active = cx.active_window().is_some();
                if !completion_notification_is_redundant(active, target.as_ref(), &self.snapshot) {
                    if active {
                        self.show_completion_notice(&body, target);
                        dirty.root = true;
                        dirty.rail = true;
                    } else {
                        self.show_attention_notification(
                            TURN_COMPLETED_NOTIFICATION_TITLE,
                            &body,
                            target,
                            cx,
                        );
                    }
                }
            }
            RuntimeEvent::PromptResult {
                submission_id,
                target,
                outcome,
                session,
            } => {
                self.project_prompt_result(submission_id, target, outcome, session, dirty, cx);
            }
            RuntimeEvent::SessionStatus {
                target,
                session,
                status,
                record,
            } => {
                let record = record.or_else(|| {
                    sessions::AppSessionId::from_key(&target)
                        .map(crate::agent_activity::SessionStatusRecord::App)
                });
                let shared = session
                    .as_deref()
                    .is_some_and(|path| self.sessions.all.has_shared_path(path));
                let actor_path = session.as_deref().filter(|_| !shared || record.is_none());
                if matches!(status, RunStatus::Stopped | RunStatus::Failed) {
                    self.workspace.voice.stopped(&target, actor_path);
                }
                if status == RunStatus::Stopped {
                    let session_key = actor_path.map(session_target);
                    let buffer = record
                        .as_ref()
                        .and_then(|record| record.resolve(&self.sessions.all))
                        .filter(|row| session.as_ref() == Some(&row.path))
                        .map(|row| sessions::session_buffer(row.harness, &row.path));
                    for pending in self.composer.pending_submissions.values_mut() {
                        let command_target =
                            sessions::composer_command_target(&pending.submitted_target);
                        if Some(&pending.submitted_target) == buffer.as_ref()
                            || (command_target == target
                                && (!shared
                                    || record.is_none()
                                    || sessions::session_path(&target).is_none()))
                            || Some(command_target) == session_key.as_deref()
                        {
                            pending.result.get_or_insert((
                                crate::agents::PromptOutcome::RejectedBeforeAcceptance,
                                session.clone(),
                            ));
                        }
                    }
                    if let Some(path) = session.as_deref() {
                        if self.sessions.all.at_path(path).any(|row| row.is_running) {
                            dirty.worker_changed(self, path, cx);
                        }
                        if let Some(record) = &record {
                            self.sessions.visible.set_record_running(record, false);
                            self.sessions.all.set_record_running(record, false);
                        } else {
                            self.sessions.visible.mark_stopped(path);
                            self.sessions.all.mark_stopped(path);
                        }
                        if record.as_ref().is_none_or(|record| {
                            record.resolve(&self.sessions.all).is_some_and(|row| {
                                row.path == path
                                    && self.snapshot.matches_catalog_row(&self.sessions.all, row)
                            })
                        }) {
                            clear_stopped_snapshot(Arc::make_mut(&mut self.snapshot), path);
                        }
                    }
                    dirty.root = true;
                    dirty.composer = true;
                    dirty.run = true;
                }
                self.workspace
                    .code_tasks
                    .associate(&target, session.as_deref());
                dirty.root |= self.workspace.code_tasks.notice_message().is_some();
                if status == RunStatus::Stopped {
                    self.code_tasks_stopped(&target, session.as_deref(), cx);
                }
                self.record_session_status(target, session, status, record.as_ref());
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
            RuntimeEvent::AccountsChanged(accounts) => self.accounts = accounts,
            RuntimeEvent::Stopped => Arc::make_mut(&mut self.snapshot).status = "Stopped".into(),
            RuntimeEvent::Snapshot { .. }
            | RuntimeEvent::RefreshCatalog
            | RuntimeEvent::WorkerSnapshot(_)
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
    let key = match submission_id {
        Some(id) => pending
            .get(id)
            .is_some_and(|row| {
                sessions::composer_command_target(&row.submitted_target) == target
                    || (target.starts_with("draft:")
                        && session.as_deref().is_some_and(|path| {
                            sessions::composer_command_target(&row.submitted_target)
                                == session_target(path)
                        }))
            })
            .then(|| id.to_owned()),
        None => {
            let mut matches = pending.iter().filter(|(_, pending)| {
                sessions::composer_command_target(&pending.submitted_target) == target
                    && pending.result.is_none()
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
    snapshot.live_status = RunStatus::Stopped;
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
