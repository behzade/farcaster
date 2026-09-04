use super::*;

impl FarcasterApp {
    pub(crate) fn new(
        project: PathBuf,
        repository_execution_allowed: bool,
        workgraph_updates: async_channel::Receiver<()>,
        worker_updates: async_channel::Receiver<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let _startup_timing =
            crate::app::infrastructure::performance::StartupTiming::new("app.start");
        let registry_timing =
            crate::app::infrastructure::performance::StartupTiming::new("app.load_registry");
        let (mut registry, mut project_registry_error) = match project_registry::load() {
            Ok(registry) => (registry, None),
            Err(error) => (projects::Registry::default(), Some(error)),
        };
        drop(registry_timing);
        projects::select(
            &mut registry.projects,
            &registry.excluded_projects,
            project.clone(),
        );
        let session_order_timing =
            crate::app::infrastructure::performance::StartupTiming::new("app.load_session_order");
        let session_order = match project_registry::load_app_session_order() {
            Ok(order) => order,
            Err(error) => {
                if project_registry_error.is_none() {
                    project_registry_error = Some(error);
                }
                Vec::new()
            }
        };
        drop(session_order_timing);
        let draft_timing =
            crate::app::infrastructure::performance::StartupTiming::new("app.create_draft");
        let initial_draft = match project_registry::new_draft(project.clone(), "pi") {
            Ok(draft) => draft,
            Err(error) => {
                if project_registry_error.is_none() {
                    project_registry_error = Some(error);
                }
                projects::DraftSession::with_id(
                    format!("untracked-draft-{}", std::process::id()),
                    project.clone(),
                )
            }
        };
        drop(draft_timing);
        let selected_draft = initial_draft.id.clone();
        let mut draft_session_ids = registry
            .drafts
            .iter()
            .map(|draft| (draft.id.clone(), draft.app_session_id))
            .collect::<HashMap<_, _>>();
        draft_session_ids.insert(initial_draft.id, initial_draft.app_session_id);
        let save_registry_timing =
            crate::app::infrastructure::performance::StartupTiming::new("app.save_registry");
        if project_registry_error.is_none()
            && let Err(error) = project_registry::save(&registry)
        {
            project_registry_error = Some(error);
        }
        drop(save_registry_timing);
        let composer_timing = crate::app::infrastructure::performance::StartupTiming::new(
            "app.load_composer_sessions",
        );
        let (composer_sessions, composer_error) =
            ComposerSessions::load(draft_target(&selected_draft));
        drop(composer_timing);
        if project_registry_error.is_none() {
            project_registry_error = composer_error;
        }
        let submitted_drafts = drafts::submitted_draft_associations(&registry.drafts);
        let proxy_timing =
            crate::app::infrastructure::performance::StartupTiming::new("app.load_proxy");
        let saved_proxy = crate::app::infrastructure::persistence::StateStore::open()
            .and_then(|store| crate::access::load_proxy(&store))
            .unwrap_or(None);
        drop(proxy_timing);
        let runtime_timing =
            crate::app::infrastructure::performance::StartupTiming::new("app.spawn_runtime");
        let runtime = RuntimeHandle::spawn(
            project.clone(),
            selected_draft.clone(),
            None,
            saved_proxy.clone(),
        );
        drop(runtime_timing);
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 8)
                .submit_on_enter(true)
                .placeholder("Ask Pi")
        });
        let initial_composer = composer_sessions.current();
        composer.update(cx, |input, cx| {
            input.set_value(initial_composer.text.clone(), window, cx);
            input.set_selected_range(initial_composer.restore_range(), cx);
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search sessions"));
        let search_focus = search.read(cx).focus_handle(cx);
        let session_title_input = cx.new(|cx| InputState::new(window, cx));
        let network_proxy_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("http://127.0.0.1:8080")
                .default_value(saved_proxy.unwrap_or_default())
        });
        let dialog_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 12)
                .submit_on_enter(false)
        });
        let composer_focus = composer.read(cx).focus_handle(cx);
        let dialog_focus = cx.focus_handle();
        let composer_subscription = subscribe_composer(&composer, window, cx);
        let search_subscription =
            cx.subscribe_in(&search, window, |this, state, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = state.read(cx).value().trim().to_owned();
                    this.send(RuntimeCommand::LoadSessions(query));
                }
            });
        let session_title_subscription = cx.subscribe_in(
            &session_title_input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    this.commit_session_title_edit(window, cx);
                }
                InputEvent::Change | InputEvent::Focus => {}
            },
        );
        let window_activation_subscription =
            cx.observe_window_activation(window, |this, window, cx| {
                let visible = session_shortcuts_visible_for_window(
                    this.view.session_rail.shortcuts_visible,
                    window.is_window_active(),
                );
                if this.view.session_rail.shortcuts_visible != visible {
                    this.view.session_rail.shortcuts_visible = visible;
                    this.notify_session_rail(cx);
                }
            });
        let window_placement_subscription = launch::observe_window_placement(window, cx);
        let runtime_wake = runtime.wake_receiver();
        let event_task = cx.spawn(async move |weak, cx| {
            while runtime_wake.recv().await.is_ok() {
                if weak.update(cx, |this, cx| this.drain_runtime(cx)).is_err() {
                    break;
                }
            }
        });
        let workgraph_update_task = cx.spawn(async move |weak, cx| {
            while workgraph_updates.recv().await.is_ok() {
                if weak
                    .update(cx, |this, cx| this.refresh_workgraph_sidebar(cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let worker_update_task = cx.spawn(async move |weak, cx| {
            while worker_updates.recv().await.is_ok() {
                if weak
                    .update(cx, |this, _| {
                        this.send(RuntimeCommand::RefreshSessions);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let transcript_list = TranscriptListState::new();
        transcript_list.scroll_to_end();
        let debug = std::env::var("DEBUG").ok().as_deref() == Some("true");
        let performance_monitor = debug.then(|| {
            crate::app::infrastructure::performance::PerformanceMonitor::new(
                window.window_handle().window_id(),
            )
        });
        let performance_task = debug.then(|| {
            cx.spawn(async move |weak, cx| {
                loop {
                    cx.background_executor()
                        .timer(crate::app::infrastructure::performance::sample_interval())
                        .await;
                    if weak
                        .update(cx, |this, cx| {
                            if this.performance_monitor.as_mut().is_some_and(
                                crate::app::infrastructure::performance::PerformanceMonitor::sample_if_due,
                            ) {
                                this.notify_run_panel(cx);
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
        });
        let app = cx.entity().downgrade();
        let session_rail_view = cx.new(|_| SessionRailView::new(app.clone()));
        let archived_session_rail_view =
            cx.new(|_| InactiveSessionRailView::new(app.clone(), SessionRailKind::Archived));
        let transcript_view = cx.new(|_| TranscriptView::new(app.clone()));
        let composer_view = cx.new(|_| ComposerView::new(app.clone()));
        let run_panel_view = cx.new(|_| RunPanelView::new(app.clone()));
        let workgraph_view = cx.new(|cx| {
            WorkGraphBoardView::new(
                crate::app::infrastructure::persistence::state_path(),
                project.clone(),
                window,
                cx,
            )
        });
        let workgraph_detail_view =
            cx.new(|cx| WorkGraphDetailView::new(app.clone(), workgraph_view.clone(), cx));
        let workgraph_sidebar_view = cx.new(|cx| {
            WorkGraphSidebarView::new(
                app.clone(),
                crate::app::infrastructure::persistence::state_path(),
                project.clone(),
                cx,
            )
        });
        transcript_list.set_scroll_handler(move |following, _, cx| {
            let needs_update = app.upgrade().is_some_and(|app| {
                let app = app.read(cx);
                transcript_follow_state_needs_update(
                    app.view.transcript.following,
                    app.view.transcript.unseen,
                    following,
                )
            });
            if !needs_update {
                return;
            }
            let app = app.clone();
            let deferred_at = Instant::now();
            cx.defer(move |cx| {
                crate::app::infrastructure::performance::record_scroll_defer(deferred_at.elapsed());
                let _ = app.update(cx, |this, cx| {
                    if update_transcript_follow_state(
                        &mut this.view.transcript.following,
                        &mut this.view.transcript.unseen,
                        following,
                    ) {
                        this.notify_transcript(cx);
                    }
                });
            });
        });
        let repository_timing = crate::app::infrastructure::performance::StartupTiming::new(
            "app.load_repository_state",
        );
        let repository =
            repository::RepositoryState::load(project.clone(), repository_execution_allowed);
        drop(repository_timing);
        let mut this = Self {
            project: project.clone(),
            runtime,
            snapshot: Arc::new(RuntimeSnapshot {
                status: "Starting".into(),
                project: project.clone(),
                ..RuntimeSnapshot::default()
            }),
            sessions: Vec::new(),
            all_sessions: Vec::new(),
            agent_activities: HashMap::new(),
            agent_row_focus: HashMap::new(),
            background_jobs: Vec::new(),
            repository,
            session_order,
            session_drop_target: None,
            run_statuses: HashMap::new(),
            recent_completions: HashMap::new(),
            recent_completion_expiries: HashMap::new(),
            system_notification_target: None,
            projects: registry.projects,
            excluded_projects: registry.excluded_projects,
            drafts: registry.drafts,
            draft_session_ids,
            selected_draft: Some(selected_draft),
            preferred_harness: "pi".into(),
            submitted_drafts,
            sessions_error: project_registry_error,
            session_project_filter: None,
            picker: None,
            picker_return_focus: None,
            session_list: ListState::new(
                0,
                ListAlignment::Top,
                crate::app::ui::theme::THEME.layout.transcript_overdraw,
            ),
            session_list_rows: RefCell::new(Vec::new()),
            archived_session_list: ListState::new(
                0,
                ListAlignment::Top,
                crate::app::ui::theme::THEME.layout.transcript_overdraw,
            ),
            archived_session_list_rows: RefCell::new(Vec::new()),
            session_generation: 0,
            runtime_generation: 0,
            composer,
            composer_project_files: Vec::new(),
            composer_project_files_project: None,
            composer_project_files_loading: None,
            session_rail_view,
            archived_session_rail_view,
            transcript_view,
            composer_view,
            run_panel_view,
            workgraph_view,
            workgraph_detail_view,
            workgraph_sidebar_view,
            editor: None,
            editor_error: None,
            editor_return_focus: None,
            terminal: None,
            terminal_project: None,
            terminal_error: None,
            native_surface_snapshot: None,
            native_surface_covered: false,
            surface: AppSurface::Chat,
            workgraph_inspector_issue: None,
            composer_sessions,
            session_surfaces: HashMap::new(),
            composer_history_marker: None,
            composer_escape_armed: None,
            composer_images: HashMap::new(),
            composer_pastes: HashMap::new(),
            search,
            search_focus,
            session_title_input,
            network_proxy_input,
            network_proxy_error: None,
            settings_application_modifier: ui::keybindings::application_modifier(),
            editing_session_title: None,
            pending_session_titles: HashMap::new(),
            pending_session_title_focus: false,
            dialog_input,
            composer_focus,
            dialog_focus,
            dialog_return_focus: None,
            image_preview: None,
            image_preview_focus: cx.focus_handle(),
            image_preview_return_focus: None,
            sheet_focus: cx.focus_handle(),
            sheet_return_focus: None,
            view: views::state::ViewState::new(transcript_list),
            performance_monitor,
            _performance_task: performance_task,
            pending_session_switch: None,
            extension: ExtensionUiState::default(),
            parked_extension: None,
            restored_dialog_id: None,
            dismissed_restored_dialog_id: None,
            notification_expiries: HashMap::new(),
            pending_dialog_setup: false,
            pending_title: None,
            pending_editor_text: None,
            pending_composer_restore: None,
            pending_submissions: HashMap::new(),
            post_render_focus: None,
            pending_archive: None,
            pending_delete: None,
            archived_sessions_expanded: false,
            project_trust_error: None,
            project_trust_project: None,
            pending_project_trust_command: None,
            _composer_subscription: composer_subscription,
            _search_subscription: search_subscription,
            _session_title_subscription: session_title_subscription,
            _window_activation_subscription: window_activation_subscription,
            _window_placement_subscription: window_placement_subscription,
            _event_task: event_task,
            _workgraph_update_task: workgraph_update_task,
            _worker_update_task: worker_update_task,
        };
        this.request_repository_refresh(cx);
        this
    }
}

fn subscribe_composer(
    composer: &Entity<TextareaState>,
    window: &mut Window,
    cx: &mut Context<FarcasterApp>,
) -> Subscription {
    cx.subscribe_in(
        composer,
        window,
        |this, state, event: &InputEvent, window, cx| match event {
            InputEvent::Change => {
                this.composer_view.update(cx, |view, _| {
                    view.reset_suggestion_selection();
                });
                this.composer_sessions.exit_history();
                let snapshot = input_snapshot(state.read(cx));
                let has_mention =
                    file_mentions::query_at_cursor(&snapshot.text, snapshot.cursor).is_some();
                this.composer_sessions.capture_current(snapshot);
                if has_mention {
                    this.request_composer_project_files(cx);
                }
                this.notify_composer(cx);
            }
            InputEvent::Blur => {
                this.composer_sessions
                    .capture_current(input_snapshot(state.read(cx)));
            }
            InputEvent::PressEnter { shift: false, .. } => {
                let input = state.read(cx);
                let value = input.value();
                if let Some(completion) = composer_completion::resolve_for_harness(
                    &value,
                    input.cursor(),
                    &this.composer_project_files,
                    this.composer_view.read(cx).suggestion_selection(),
                    &this.snapshot.commands,
                    this.active_harness(),
                ) {
                    let submitted_value = completion
                        .submit
                        .then(|| completion.snapshot.text.trim().to_owned());
                    this.apply_composer_snapshot(completion.snapshot, window, cx);
                    if let Some(value) = submitted_value {
                        this.submit(value, this.enter_mode(), window, cx);
                    } else {
                        this.composer_focus.focus(window, cx);
                    }
                } else {
                    let value = value.trim().to_owned();
                    if !value.is_empty() || this.has_composer_attachments() {
                        this.submit(value, this.enter_mode(), window, cx);
                    }
                }
            }
            InputEvent::PressEnter { .. } | InputEvent::Focus => {}
        },
    )
}
