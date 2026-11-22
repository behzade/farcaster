use super::*;

pub(in crate::app) const USER_SESSION_SWITCH_RESTORES_CENTER: bool = true;

struct SwitchOrigin {
    target: String,
    project: PathBuf,
    draft: Option<crate::sessions::DraftSession>,
    key: Option<crate::sessions::SessionKey>,
}

pub(in crate::app) struct SwitchRecovery {
    origin: SwitchOrigin,
    // Earlier requests can still be routed while a later click waits for trust.
    sent: HashMap<u64, SwitchOrigin>,
    rejected: bool,
    pub(in crate::app) snapshot: Option<(u64, Arc<RuntimeSnapshot>)>,
}

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

fn draft_catalog_row<'a>(
    all: &'a sessions::SessionCatalog,
    draft: &sessions::DraftSession,
    path: &Path,
) -> Option<&'a SessionSummary> {
    if let Some(row) = all.by_app_id(draft.app_session_id) {
        return (row.path == path).then_some(row);
    }
    let harness = draft.harness?;
    let mut matches = all.at_path(path).filter(|row| {
        row.harness == harness && row.project == draft.project && row.profile_id == draft.profile_id
    });
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

impl FarcasterApp {
    pub(super) fn bound_draft_row<'a>(
        &self,
        catalog: &'a crate::sessions::SessionCatalog,
        draft_id: &str,
        path: &Path,
    ) -> Option<&'a SessionSummary> {
        let draft = self
            .sessions
            .drafts
            .iter()
            .find(|draft| draft.id == draft_id)?;
        let row = draft_catalog_row(&self.sessions.all, draft, path)?;
        crate::agent_activity::SessionStatusRecord::for_row(row)
            .resolve(catalog)
            .filter(|row| row.path == path)
    }

    pub(in crate::app) fn sidebar_session_path(&self) -> Option<&Path> {
        if let Some(draft) = &self.sessions.selected_draft {
            return self
                .sessions
                .submitted_drafts
                .get(draft)
                .and_then(Option::as_deref);
        }
        self.lifecycle
            .pending_session_switch
            .as_ref()
            .map(|(path, _)| path.as_path())
            .or(self.snapshot.selected_session.as_deref())
    }

    pub(in crate::app) fn sidebar_catalog_row<'a>(
        &self,
        catalog: &'a crate::sessions::SessionCatalog,
    ) -> Option<&'a SessionSummary> {
        let path = self.sidebar_session_path()?;
        if let Some(draft_id) = &self.sessions.selected_draft {
            return self.bound_draft_row(catalog, draft_id, path);
        } else {
            if self.lifecycle.pending_session_switch.is_some() {
                if let Some(key) = &self.lifecycle.requested_session_key {
                    return catalog.by_key(key);
                }
            } else {
                if self.sessions.all.has_shared_path(path) {
                    let row = self.snapshot.catalog_row_at(&self.sessions.all, path)?;
                    return catalog.by_key(&row.catalog_key());
                }
                return self.snapshot.catalog_row_at(catalog, path);
            }
        }
        catalog.by_path(path)
    }

    fn begin_navigation_intent(&mut self, cx: &mut Context<Self>) {
        self.lifecycle.navigation_intent += 1;
        self.lifecycle.opening_local = None;
        self.lifecycle.display_fit = None;
        if self.sessions.error.as_deref() == Some(super::REFRESHING_SESSION_DETAILS) {
            self.sessions.error = None;
            self.notify_session_rail_shell(cx);
        }
    }

    pub(in crate::app) fn cancel_pending_session_switch(&mut self) {
        self.lifecycle.opening_local = None;
        self.lifecycle.display_fit = None;
        self.project.navigation_trust = None;
        self.lifecycle.switch_recovery = None;
        self.lifecycle.requested_session_key = None;
        self.lifecycle.awaiting_switch_route = None;
        if let Some((_, timing)) = self.lifecycle.pending_session_switch.take() {
            timing.cancel();
        }
    }

    fn composer_selection(&self) -> SwitchOrigin {
        SwitchOrigin {
            target: self.composer.sessions.current_buffer().to_owned(),
            project: self.project.path.clone(),
            draft: self.sessions.selected_draft.as_ref().and_then(|id| {
                self.sessions
                    .drafts
                    .iter()
                    .find(|row| &row.id == id)
                    .cloned()
            }),
            key: self
                .sidebar_catalog_row(&self.sessions.all)
                .map(SessionSummary::catalog_key),
        }
    }

    pub(in crate::app) fn begin_navigation(&mut self) {
        self.begin_navigation_from_draft(None);
    }

    pub(super) fn begin_navigation_from_draft(&mut self, previous: Option<sessions::DraftSession>) {
        if let Some(path) = sessions::session_path(self.composer.sessions.current_buffer())
            && let Some(row) = self
                .sidebar_catalog_row(&self.sessions.all)
                .filter(|row| row.path == path)
        {
            let (path, harness) = (row.path.clone(), row.harness);
            self.bind_legacy_composer(&path, harness);
        }
        // Preserve the last routed destination across clicks that supersede a trust check.
        let mut recovery = self.lifecycle.switch_recovery.take().unwrap_or_else(|| {
            let mut origin = self.composer_selection();
            if let Some(draft) = previous {
                origin.draft = Some(draft);
            }
            SwitchRecovery {
                origin,
                sent: HashMap::new(),
                rejected: false,
                snapshot: None,
            }
        });
        self.cancel_pending_session_switch();
        recovery.rejected = false;
        self.lifecycle.switch_recovery = Some(recovery);
        self.lifecycle.switch_sequence += 1;
        self.lifecycle.awaiting_switch_route = Some(self.lifecycle.switch_sequence);
    }

    pub(in crate::app) fn awaiting_selection_snapshot(&self) -> bool {
        self.lifecycle.opening_local.is_some()
            || self.lifecycle.awaiting_switch_route.is_some()
            || self.lifecycle.pending_session_switch.is_some()
            || self
                .lifecycle
                .awaiting_draft_snapshot
                .as_ref()
                .is_some_and(|id| self.sessions.selected_draft.as_ref() == Some(id))
    }

    pub(in crate::app) fn record_selection_dispatch(
        &mut self,
        command: &RuntimeCommand,
    ) -> Option<u64> {
        let id = match command {
            RuntimeCommand::NewSession { request, .. }
            | RuntimeCommand::ResumeDraft { request, .. }
            | RuntimeCommand::SelectSession { request, .. }
            | RuntimeCommand::RestartSession { request, .. } => (*request)?,
            _ => return None,
        };
        let destination = self.composer_selection();
        if let Some(recovery) = &mut self.lifecycle.switch_recovery {
            recovery.sent.insert(id, destination);
        }
        Some(id)
    }

    pub(in crate::app) fn finish_selection_route(
        &mut self,
        sequence: u64,
        routed: bool,
        cx: &mut Context<Self>,
    ) {
        let current = self.lifecycle.awaiting_switch_route == Some(sequence);
        if routed && current {
            self.lifecycle.awaiting_switch_route = None;
        }
        let Some(recovery) = self.lifecycle.switch_recovery.as_mut() else {
            return;
        };
        if let Some(destination) = recovery.sent.remove(&sequence)
            && routed
        {
            self.lifecycle.awaiting_draft_snapshot =
                destination.draft.as_ref().map(|d| d.id.clone());
            recovery.origin = destination;
            recovery.snapshot = None;
        }
        if routed && current && self.lifecycle.pending_session_switch.is_none() {
            self.lifecycle.switch_recovery = None;
            self.notify_composer(cx);
            return;
        }
        recovery.rejected |= current && !routed;
        if recovery.rejected && recovery.sent.is_empty() {
            let sequence = self.lifecycle.switch_sequence;
            let app = cx.entity().downgrade();
            cx.defer(move |cx| {
                let _ = app.update_in(cx, |app, window, cx| {
                    if app.lifecycle.switch_sequence == sequence {
                        app.restore_rejected_selection(window, cx);
                    }
                });
            });
        }
    }

    pub(in crate::app) fn restore_rejected_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(recovery) = &mut self.lifecycle.switch_recovery else {
            return;
        };
        recovery.rejected = true;
        if !recovery.sent.is_empty() {
            return;
        }
        let recovery = self.lifecycle.switch_recovery.take().unwrap();
        self.cancel_pending_session_switch();
        let SwitchOrigin {
            mut target,
            project,
            draft,
            mut key,
        } = recovery.origin;
        let selected_draft = draft.and_then(|draft| {
            if let Some(row) = self
                .sessions
                .drafts
                .iter_mut()
                .find(|row| row.id == draft.id)
            {
                if row.can_change_project()
                    && (row.project != draft.project
                        || row.harness != draft.harness
                        || row.profile_id != draft.profile_id)
                {
                    row.project = draft.project;
                    row.harness = draft.harness;
                    row.profile_id = draft.profile_id;
                    self.save_session_draft(&draft.id);
                }
                return Some(draft.id);
            }
            if draft.app_session_id > 0
                && let Some(row) = self.sessions.all.by_app_id(draft.app_session_id)
            {
                target = sessions::session_buffer(row.harness, &row.path);
                key = Some(row.catalog_key());
                return None;
            }
            // Leaving an empty, unsaved draft removes it from the rail.
            let id = draft.id.clone();
            self.sessions
                .draft_session_ids
                .insert(id.clone(), draft.app_session_id);
            self.sessions.drafts.push(draft);
            Some(id)
        });
        if let Some(path) = sessions::session_path(sessions::composer_command_target(&target)) {
            self.lifecycle.pending_session_switch = Some((
                path.to_owned(),
                crate::app::infrastructure::performance::Timing::new("switch.session_total"),
            ));
        }
        self.switch_composer_target(target, window, cx);
        self.sessions.selected_draft = selected_draft;
        self.lifecycle.requested_session_key = key;
        if self.project.path == project {
            self.remember_session_selection();
        } else {
            self.select_project_for_navigation(project.clone(), cx);
        }
        self.restore_center_surface(project, window, cx);
        if let Some((generation, snapshot)) = recovery.snapshot {
            self.restore_navigation_snapshot(generation, snapshot, cx);
        } else if self.pending_switch_matches(&self.snapshot) {
            drop(self.lifecycle.pending_session_switch.take());
        }
        self.notify_session_rail(cx);
        self.notify_transcript(cx);
        self.notify_composer(cx);
        self.notify_run_panel(cx);
        cx.notify();
    }

    pub(in crate::app) fn pending_switch_matches(&self, snapshot: &RuntimeSnapshot) -> bool {
        let Some((path, _)) = &self.lifecycle.pending_session_switch else {
            return false;
        };
        if snapshot.selected_session.as_ref() != Some(path) {
            return false;
        }
        // The route and its buffer share the same (harness, locator). Catalog
        // rows can disappear or gain native metadata while the actor loads.
        match sessions::composer_buffer_parts(self.composer.sessions.current_buffer()) {
            Some((harness, _)) => snapshot.harness == Some(harness),
            None => self.lifecycle.requested_session_key.is_none(),
        }
    }

    pub(in crate::app) fn remember_session_selection(&mut self) {
        let target = self.composer.sessions.current_target();
        let selected = crate::sessions::draft_id(target)
            .and_then(|id| self.sessions.drafts.iter().find(|draft| draft.id == id))
            .map(|draft| (draft.project.clone(), draft.app_session_id))
            .or_else(|| {
                let path = crate::sessions::session_path(target)?;
                let catalog = &self.sessions.all;
                let row = if catalog.has_shared_path(path) {
                    if self.lifecycle.pending_session_switch.is_some() {
                        catalog.by_key(self.lifecycle.requested_session_key.as_ref()?)
                    } else {
                        self.snapshot.catalog_row_at(catalog, path)
                    }
                } else {
                    catalog.by_path(path)
                }?;
                (row.path == path).then(|| (row.project.clone(), row.app_session_id))
            });
        let (project, id) = selected.unwrap_or((self.project.path.clone(), 0));
        if let Err(error) = self
            .sessions
            .writer
            .save_selected_session(project, (id > 0).then_some(id))
        {
            self.sessions.error = Some(error);
        }
    }

    fn target_for_path(&self, path: &Path) -> Option<SessionTarget> {
        let path = crate::sessions::normalize_session_path(path);
        self.sessions
            .all
            .by_path(&path)
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
        self.retry_after_catalog_refresh(
            window,
            cx,
            move |app| app.target_for_path(&path),
            move |app, _, window, cx| action(app, window, cx),
        );
    }

    fn retry_after_catalog_refresh<T: 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        resolve: impl Fn(&Self) -> Option<T> + 'static,
        action: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let intent = self.lifecycle.navigation_intent;
        if let Some(owner) = self.modal_owner_window(window, cx) {
            crate::app::views::native_overlay::in_owner_window(
                cx.entity().downgrade(),
                Some(owner),
                window,
                cx,
                move |app, window, cx| {
                    if app.lifecycle.navigation_intent == intent {
                        app.retry_after_catalog_refresh(window, cx, resolve, action);
                    }
                },
            );
            return;
        }
        let current_target = self.composer.sessions.current_buffer().to_owned();
        let sequence = self.lifecycle.switch_sequence;
        let mut action = Some(action);
        cx.spawn_in(window, async move |weak, cx| {
            for _ in 0..50 {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                let done = weak
                    .update_in(cx, |this, window, cx| {
                        if this.lifecycle.navigation_intent != intent {
                            return true;
                        }
                        if this.lifecycle.switch_sequence != sequence
                            || this.composer.sessions.current_buffer() != current_target
                        {
                            if this.sessions.error.as_deref()
                                == Some(super::REFRESHING_SESSION_DETAILS)
                            {
                                this.sessions.error = None;
                                this.notify_session_rail(cx);
                            }
                            return true;
                        }
                        if let Some(target) = resolve(this) {
                            if let Some(action) = action.take() {
                                this.sessions.error = None;
                                action(this, target, window, cx);
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
                if this.lifecycle.navigation_intent == intent
                    && this.lifecycle.switch_sequence == sequence
                    && this.composer.sessions.current_buffer() == current_target
                {
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
            self.sidebar_session_path(),
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
            self.sessions.error = Some(super::REFRESHING_SESSION_DETAILS.into());
            self.send(RuntimeCommand::RefreshSessions, cx);
            self.notify_session_rail(cx);
        }
        target
    }

    pub(in crate::app) fn prefetch_session(
        &mut self,
        harness: crate::agents::Backend,
        path: PathBuf,
        project: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.prefetch_repository_observation(project.clone(), cx);
        let config = crate::agents::AgentLaunchConfig {
            profiles: self.settings.harness_profiles.clone(),
            profile_id: crate::agents::profile_id_from_locator(&path),
            ..Default::default()
        };
        cx.background_spawn(async move {
            let _ =
                crate::agents::warm_session_history_for_profile(&config, harness, &path, &project);
        })
        .detach();
    }

    pub(in crate::app) fn prefetch_folder(&mut self, folder: u64, cx: &mut Context<Self>) {
        let folders = self.sessions.folders.clone();
        let targets = self
            .sessions
            .all
            .iter()
            .filter(|session| folders.folder_for(session.app_session_id) == Some(folder))
            .map(|session| {
                (
                    session.harness,
                    session.path.clone(),
                    session.project.clone(),
                )
            })
            .collect::<Vec<_>>();
        let project = targets.first().map(|(_, _, project)| project.clone());
        for (harness, path, project) in targets {
            self.prefetch_session(harness, path, project, cx);
        }
        if let Some(project) = project {
            self.prefetch_repository_observation(project, cx);
        }
    }

    pub(in crate::app) fn select_session(
        &mut self,
        path: PathBuf,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_navigation_intent(cx);
        // Path-only entry points, including notifications, can still stage a saved
        // display when the catalog identifies a unique owner for this locator.
        let key = self
            .sessions
            .all
            .by_path(&path)
            .filter(|row| {
                row.has_local_transcript
                    && row.project == project
                    && !self.sessions.all.has_shared_path(&path)
            })
            .map(SessionSummary::catalog_key);
        self.open_session(path, project, key, true, window, cx);
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

    pub(in crate::app) fn select_rail_session(
        &mut self,
        key: crate::sessions::SessionKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_catalog_session(key, true, window, cx);
    }

    pub(in crate::app) fn select_catalog_session(
        &mut self,
        key: crate::sessions::SessionKey,
        reveal_worker: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_navigation_intent(cx);
        let Some(row) = self
            .sessions
            .all
            .by_key(&key)
            .or_else(|| self.sessions.visible.by_key(&key))
        else {
            return;
        };
        let (path, project) = (row.path.clone(), row.project.clone());
        self.open_session(path, project, Some(key), reveal_worker, window, cx);
        self.recover_keyboard_focus(window, cx);
    }

    fn open_session(
        &mut self,
        path: PathBuf,
        project: PathBuf,
        key: Option<crate::sessions::SessionKey>,
        reveal_worker: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.pending_trust_command.is_some() {
            return;
        }
        use crate::app::infrastructure::performance::{self, SwitchPaintStage, SwitchPaintTrace};
        let click_started = performance::tracing_every_operation().then(std::time::Instant::now);
        let _timing =
            crate::app::infrastructure::performance::Timing::new("switch.session_request");
        let row = key.as_ref().and_then(|key| {
            self.sessions
                .all
                .by_key(key)
                .or_else(|| self.sessions.visible.by_key(key))
        });
        if self.snapshot.selected_session.as_deref() == Some(path.as_path())
            && (key.is_none() || row.is_some_and(|row| self.snapshot.matches_session_row(row)))
            && (self.lifecycle.pending_session_switch.is_none()
                || key == self.lifecycle.requested_session_key)
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
        let profile = row
            .and_then(|row| row.profile_id.clone())
            .or_else(|| crate::agents::profile_id_from_locator(&path));
        let view = self.views.transcript.read(cx);
        let resident = row
            .filter(|row| view.has_resident_for(row))
            .map(|row| view.resident_stamp_for(row));
        let stage_local = row.is_some_and(|row| row.has_local_transcript) || resident.is_some();
        let full = view.needs_full_display(&path, key.clone());
        let Some(target) = row
            .map(SessionSummary::target)
            .or_else(|| self.backend_target_for_path(&path, cx))
        else {
            self.retry_after_session_refresh(path.clone(), window, cx, move |this, window, cx| {
                this.open_session(path, project, key, reveal_worker, window, cx);
            });
            return;
        };
        if let Some(started) = click_started {
            self.lifecycle.switch_paint =
                Some(SwitchPaintTrace::start(started, target.harness, &path));
        }
        let query = crate::app::views::DisplayQuery {
            trace: self.lifecycle.switch_paint.clone(),
            path: path.clone(),
            project: project.clone(),
            harness: target.harness,
            profile,
        };
        if stage_local {
            // Publish the destination's selection, buffer and saved rows together. An
            // intervening draw still has the complete previous view. Another navigation
            // drops this task, including when the user reselects the currently shown row.
            let read = query.clone().read(resident, full, cx);
            let intent = self.lifecycle.navigation_intent;
            self.lifecycle.opening_local = Some(cx.spawn_in(window, async move |weak, cx| {
                let mut page = read.await;
                let Ok(probe) = weak.update_in(cx, |app, window, cx| {
                    if app.lifecycle.navigation_intent == intent {
                        app.check_display_fit(&page, window, cx)
                    } else {
                        None
                    }
                }) else {
                    return;
                };
                if let Some(probe) = probe
                    && !probe.await.unwrap_or(false)
                {
                    let Ok(Some(read)) = weak.update(cx, |app, cx| {
                        (app.lifecycle.navigation_intent == intent)
                            .then(|| query.clone().read(None, true, cx))
                    }) else {
                        return;
                    };
                    page = read.await;
                }
                let _ = weak.update_in(cx, |app, window, cx| {
                    if app.lifecycle.navigation_intent != intent {
                        return;
                    }
                    if let Some(trace) = &query.trace {
                        trace.mark(SwitchPaintStage::Delivered);
                    }
                    app.lifecycle.opening_local = None;
                    if app.project.pending_trust_command.is_some() {
                        return;
                    }
                    let target = if let Some(key) = &key {
                        let Some(row) = app
                            .sessions
                            .all
                            .by_key(key)
                            .or_else(|| app.sessions.visible.by_key(key))
                            .filter(|row| query.matches(row))
                        else {
                            return;
                        };
                        row.target()
                    } else {
                        target
                    };
                    app.show_session(query, target, key, reveal_worker, Some(page), window, cx);
                    // macOS can suspend display frames for an occluded window.
                    // Publish the saved selection to accessibility when its rows arrive.
                    crate::app::ui::accessibility::refresh_hidden_accessibility(window, cx);
                });
            }));
            return;
        }
        self.show_session(query, target, key, reveal_worker, None, window, cx);
    }

    fn show_session(
        &mut self,
        query: crate::app::views::DisplayQuery,
        target: SessionTarget,
        key: Option<crate::sessions::SessionKey>,
        reveal_worker: bool,
        prepared: Option<crate::app::views::DisplayRead>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::app::infrastructure::performance::SwitchPaintStage;
        let path = query.path.clone();
        let project = query.project.clone();
        let previous_root = self.selected_rail_root();
        let previously_archived = previous_root.is_some_and(|row| row.archived)
            || self.sessions.selected_draft.as_ref().is_some_and(|id| {
                self.sessions
                    .drafts
                    .iter()
                    .any(|draft| draft.id == *id && draft.archived)
            });
        let catalog = &self.sessions.visible;
        let next_root = match &key {
            Some(key) => catalog
                .by_key(key)
                .and_then(|row| catalog.root_for_row(row)),
            None => catalog.root_for_path(Some(&path)),
        };
        let rail_selection_changed = previous_root.map(SessionSummary::catalog_key)
            != next_root.map(SessionSummary::catalog_key)
            || self.sessions.selected_draft.is_some();
        let archive_selection_changed =
            previously_archived || next_root.is_some_and(|row| row.archived);
        self.begin_navigation();
        self.switch_composer_target(sessions::session_buffer(target.harness, &path), window, cx);
        self.sessions.selected_draft = None;
        let request = self.lifecycle.awaiting_switch_route;
        self.lifecycle.requested_session_key = key;
        self.lifecycle.pending_session_switch = Some((
            path.clone(),
            crate::app::infrastructure::performance::Timing::new("switch.session_total"),
        ));
        let display_stamp = prepared
            .as_ref()
            .and_then(crate::app::views::DisplayRead::stamp);
        self.present_or_read_transcript(
            query,
            self.lifecycle.requested_session_key.clone(),
            prepared,
            cx,
        );
        self.select_project_for_navigation(project.clone(), cx);
        if USER_SESSION_SWITCH_RESTORES_CENTER {
            self.restore_center_surface(project.clone(), window, cx);
        }
        self.views.run_panel.update(cx, |view, _| {
            view.preserve_worker_scroll_for(
                self.lifecycle
                    .requested_session_key
                    .clone()
                    .filter(|_| !reveal_worker),
            );
        });
        self.send_navigation_command(
            &project,
            RuntimeCommand::SelectSession {
                display_stamp,
                path,
                harness: target.harness,
                session_id: target.id,
                project: project.clone(),
                key: self.lifecycle.requested_session_key.clone(),
                request,
            },
            window,
            cx,
        );
        self.close_sessions_sheet_after_selection(window, cx);
        if rail_selection_changed {
            self.reset_run_panel_scroll(cx);
            self.notify_session_rail_shell(cx);
            if archive_selection_changed {
                self.views
                    .archived_session_rail
                    .update(cx, |_, cx| cx.notify());
            }
        }
        self.notify_transcript(cx);
        self.notify_composer(cx);
        self.notify_run_panel(cx);
        cx.notify();
        if let Some(trace) = &self.lifecycle.switch_paint {
            trace.mark(SwitchPaintStage::NavigationDone);
        }
    }

    pub(in crate::app) fn fork_session(
        &mut self,
        path: PathBuf,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_navigation_intent(cx);
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
        self.begin_navigation_intent(cx);
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
        let draft = super::draft_store::new(
            project.clone(),
            self.sessions.preferred_harness,
            self.sessions.preferred_profile_id.clone(),
        );
        if let Some(folder) = folder
            && !self.save_new_folder_draft(draft.clone(), folder, window, cx)
        {
            return;
        }
        let draft_key = draft_target(&draft.id);
        self.begin_navigation();
        self.switch_composer_target(draft_key.clone(), window, cx);
        self.sessions.selected_draft = Some(draft.id.clone());
        self.sessions
            .draft_session_ids
            .insert(draft.id.clone(), draft.app_session_id);
        self.sessions.drafts.push(draft.clone());
        self.save_project_registry();
        self.select_project_for_navigation(project.clone(), cx);
        if folder.is_none() {
            self.start_new_draft(&draft, window, cx);
        }
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

    fn start_new_draft(
        &mut self,
        draft: &crate::sessions::DraftSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_navigation_command(
            &draft.project,
            RuntimeCommand::NewSession {
                request: None,
                id: draft.id.clone(),
                harness: draft.harness,
                profile_id: draft.profile_id.clone(),
                project: draft.project.clone(),
            },
            window,
            cx,
        );
    }

    fn save_new_folder_draft(
        &mut self,
        draft: crate::sessions::DraftSession,
        folder: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let receipt = match self.sessions.writer.save_draft_with_identity(draft.clone()) {
            Ok(receipt) => receipt,
            Err(error) => {
                self.sessions.error = Some(error);
                self.notify_session_rail(cx);
                return false;
            }
        };
        self.sessions.pending_draft_saves.remove(&draft.id);
        self.sessions
            .pending_folder_drafts
            .insert(draft.id.clone(), folder);
        cx.spawn_in(window, async move |weak, cx| {
            let result = receipt
                .recv()
                .await
                .unwrap_or_else(|_| Err("Session state writer stopped before saving".into()));
            let _ = weak.update_in(cx, |app, window, cx| {
                app.finish_new_folder_draft(draft, result, window, cx);
            });
        })
        .detach();
        true
    }

    fn finish_new_folder_draft(
        &mut self,
        mut original: crate::sessions::DraftSession,
        result: Result<crate::sessions::AppSessionId, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(folder) = self.sessions.pending_folder_drafts.remove(&original.id) else {
            return;
        };
        let Some(row) = self
            .sessions
            .drafts
            .iter_mut()
            .find(|row| row.id == original.id)
        else {
            return;
        };
        match result {
            Ok(id) => {
                original.app_session_id = id.get();
                row.app_session_id = id.get();
                self.sessions
                    .draft_session_ids
                    .insert(row.id.clone(), id.get());
                if *row != original
                    && let Err(error) = self.sessions.writer.save_draft(row.clone())
                {
                    self.sessions.error = Some(error);
                }
                let current = row.clone();
                if self
                    .sessions
                    .folders
                    .folders
                    .iter()
                    .any(|item| item.id == folder)
                {
                    self.assign_session_folder(id.get(), Some(folder), cx);
                }
                if self.sessions.selected_draft.as_deref() == Some(&current.id)
                    && self.project.path == current.project
                    && self.composer.sessions.current_target() == draft_target(&current.id)
                {
                    self.remember_session_selection();
                    self.start_new_draft(&current, window, cx);
                }
            }
            Err(error) => {
                self.sessions.error = Some(error);
                self.sessions
                    .pending_draft_saves
                    .insert(original.id.clone());
                self.sessions
                    .pending_folder_drafts
                    .insert(original.id, folder);
            }
        }
        self.notify_session_rail(cx);
        self.notify_composer(cx);
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
        self.begin_navigation_intent(cx);
        if self.project.pending_trust_command.is_some() {
            return;
        }
        if self.sessions.pending_draft_saves.contains(&id)
            && let Some(&folder) = self.sessions.pending_folder_drafts.get(&id)
            && let Some(row) = self.sessions.drafts.iter().find(|row| row.id == id)
        {
            self.save_new_folder_draft(row.clone(), folder, window, cx);
        }
        if self.sessions.selected_draft.as_deref() == Some(id.as_str())
            && !self.snapshot.history_preview
        {
            self.close_sessions_sheet_after_selection(window, cx);
            return;
        }
        let command = if let Some(Some(path)) = self.sessions.submitted_drafts.get(&id).cloned() {
            let Some(draft) = self.sessions.drafts.iter().find(|draft| draft.id == id) else {
                return;
            };
            let row = draft_catalog_row(&self.sessions.all, draft, &path);
            let key = row.map(SessionSummary::catalog_key);
            let target = row.map(SessionSummary::target).or_else(|| {
                self.target_for_path(&path).filter(|target| {
                    self.sessions.all.by_path(&path).is_none()
                        && draft.harness == Some(target.harness)
                })
            });
            let Some(target) = target else {
                let draft = draft.clone();
                self.sessions.error = Some(super::REFRESHING_SESSION_DETAILS.into());
                self.send(RuntimeCommand::RefreshSessions, cx);
                self.notify_session_rail(cx);
                self.retry_after_catalog_refresh(
                    window,
                    cx,
                    move |app| {
                        draft_catalog_row(&app.sessions.all, &draft, &path)
                            .map(SessionSummary::catalog_key)
                    },
                    move |app, key, window, cx| {
                        if app.sessions.submitted_drafts.contains_key(&id) {
                            app.resume_draft_restoring_center(
                                id,
                                project,
                                restore_center,
                                window,
                                cx,
                            );
                        } else {
                            app.select_catalog_session(key, true, window, cx);
                        }
                    },
                );
                return;
            };
            RuntimeCommand::SelectSession {
                display_stamp: None,
                path,
                harness: target.harness,
                session_id: target.id,
                project: project.clone(),
                key,
                request: None,
            }
        } else {
            let Some(draft) = self.sessions.drafts.iter().find(|draft| draft.id == id) else {
                self.sessions.error = Some("The draft's harness identity is unavailable".into());
                self.notify_session_rail(cx);
                return;
            };
            if draft.app_session_id > 0 {
                RuntimeCommand::ResumeDraft {
                    request: None,
                    id: id.clone(),
                    harness: draft.harness,
                    project: project.clone(),
                }
            } else {
                RuntimeCommand::NewSession {
                    request: None,
                    id: id.clone(),
                    harness: draft.harness,
                    profile_id: draft.profile_id.clone(),
                    project: project.clone(),
                }
            }
        };
        self.reset_run_panel_scroll(cx);
        self.begin_navigation();
        self.switch_composer_target(draft_target(&id), window, cx);
        self.sessions.selected_draft = Some(id);
        self.select_project_for_navigation(project.clone(), cx);
        if restore_center {
            self.restore_center_surface(project.clone(), window, cx);
        }
        self.send_navigation_command(&project, command, window, cx);
        self.close_sessions_sheet_after_selection(window, cx);
        self.notify_session_rail_shell(cx);
        self.repaint_archived_session_rail(cx);
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
        self.sessions.pending_draft_saves.remove(id);
        self.sessions.submitted_drafts.remove(id);
        self.activity.run_statuses.remove(&target);
        self.activity.recent_completions.remove(&target);
        self.activity.recent_completion_expiries.remove(&target);
        self.sessions.pending_folder_drafts.remove(id);
        if was_selected {
            self.begin_navigation_intent(cx);
            self.project.navigation_trust = None;
            self.sessions.selected_draft = None;
            if let Some(session) = self.sessions.visible.first().cloned() {
                self.select_project_for_navigation(session.project.clone(), cx);
                let snapshot = self.composer.sessions.discard_and_switch(
                    &target,
                    sessions::session_buffer(session.harness, &session.path),
                );
                self.apply_composer_snapshot(snapshot, window, cx);
                self.restore_center_surface(session.project.clone(), window, cx);
                self.send_navigation_command(
                    &session.project,
                    RuntimeCommand::SelectSession {
                        display_stamp: None,
                        path: session.path,
                        harness: session.harness,
                        session_id: session.id,
                        project: session.project.clone(),
                        key: None,
                        request: None,
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
            let current = self.composer.sessions.current_buffer().to_owned();
            let _ = self.composer.sessions.discard_and_switch(&target, current);
        }
        self.remove_session_draft(id);
        self.remember_session_selection();
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
            self.focus_overlay(&pending.focus, window, cx);
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
}
