use super::*;

mod inputs;
mod persisted;
mod regions;
mod subscriptions;
mod tasks;

impl FarcasterApp {
    pub(crate) fn new(
        project: PathBuf,
        agent_launch: crate::agents::AgentLaunchConfig,
        repository_execution_allowed: bool,
        workgraph_updates: async_channel::Receiver<()>,
        worker_updates: async_channel::Receiver<()>,
        notice_board: mcp_server::NoticeBoard,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let _startup_timing =
            crate::app::infrastructure::performance::StartupTiming::always("app.start");
        let persisted = persisted::load(&project, agent_launch.app_proxy.clone());
        let harness_profiles = agent_launch.profiles.clone();

        let runtime_timing =
            crate::app::infrastructure::performance::StartupTiming::new("app.spawn_runtime");
        let runtime = RuntimeHandle::spawn(
            project.clone(),
            persisted
                .drafts
                .iter()
                .find(|draft| draft.id == persisted.selected_draft)
                .expect("startup draft is registered")
                .clone(),
            None,
            agent_launch,
            runtime_host::host(),
        );
        drop(runtime_timing);

        Self::from_bootstrap_state(
            project,
            repository_execution_allowed,
            workgraph_updates,
            worker_updates,
            notice_board,
            persisted,
            runtime,
            harness_profiles,
            window,
            cx,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_offline_for_test(
        project: PathBuf,
        runtime: RuntimeHandle,
        workgraph_updates: async_channel::Receiver<()>,
        worker_updates: async_channel::Receiver<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft_id = format!("offline-test-{}", uuid::Uuid::new_v4().simple());
        let draft = sessions::DraftSession::with_id(
            Some(crate::agents::Backend::Pi),
            draft_id.clone(),
            project.clone(),
        );
        let persisted = persisted::PersistedState {
            projects: projects::ProjectList {
                projects: vec![project.clone()],
                ..Default::default()
            },
            drafts: vec![draft],
            error: None,
            session_order: Vec::new(),
            session_folders: Default::default(),
            selected_draft: draft_id.clone(),
            preferred_harness: Some(crate::agents::Backend::Pi),
            preferred_profile_id: None,
            draft_session_ids: HashMap::new(),
            composer_sessions: ComposerSessions::for_test(draft_target(&draft_id)),
            submitted_drafts: HashMap::new(),
            saved_proxy: None,
            expand_transcript_folders: false,
            editor_choice: Default::default(),
            theme_css: None,
            active_theme: None,
        };
        Self::from_bootstrap_state(
            project,
            false,
            workgraph_updates,
            worker_updates,
            mcp_server::NoticeBoard::default(),
            persisted,
            runtime,
            std::sync::Arc::new(crate::agents::HarnessProfiles::default()),
            window,
            cx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_bootstrap_state(
        project: PathBuf,
        repository_execution_allowed: bool,
        workgraph_updates: async_channel::Receiver<()>,
        worker_updates: async_channel::Receiver<()>,
        notice_board: mcp_server::NoticeBoard,
        persisted: persisted::PersistedState,
        runtime: RuntimeHandle,
        harness_profiles: std::sync::Arc<crate::agents::HarnessProfiles>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let inputs = inputs::create(
            &persisted.composer_sessions,
            persisted.saved_proxy.as_deref(),
            window,
            cx,
        );
        let subscriptions = subscriptions::create(&inputs, window, cx);
        let tasks = tasks::spawn(
            &runtime,
            workgraph_updates,
            worker_updates,
            notice_board.updates(),
            cx,
        );
        let performance = tasks::start_performance_monitor(window, cx);
        let regions = regions::create(&project, window, cx);

        let repository_timing = crate::app::infrastructure::performance::StartupTiming::new(
            "app.load_repository_state",
        );
        let repository =
            repository::RepositoryState::load(project.clone(), repository_execution_allowed);
        drop(repository_timing);

        let (composer_images, composer_pastes) =
            composer::attachments::restore(&persisted.composer_sessions);

        let mut this = Self {
            runtime,
            snapshot: Arc::new(RuntimeSnapshot {
                status: "Starting".into(),
                project: project.clone(),
                ..RuntimeSnapshot::default()
            }),
            runtime_generation: 0,
            project: project::ProjectState {
                path: project.clone(),
                registered: persisted.projects.projects,
                excluded: persisted.projects.excluded_projects,
                repository,
                trust_error: None,
                trust_project: None,
                trust_backend: None,
                pending_trust_command: None,
                pending_trust_action: None,
            },
            sessions: session::SessionState {
                visible: Vec::new(),
                all: Vec::new(),
                order: persisted.session_order,
                folders: persisted.session_folders,
                editing_folder: None,
                drop_target: None,
                drafts: persisted.drafts,
                draft_session_ids: persisted.draft_session_ids,
                selected_draft: Some(persisted.selected_draft),
                preferred_harness: persisted.preferred_harness,
                preferred_profile_id: persisted.preferred_profile_id,
                submitted_drafts: persisted.submitted_drafts,
                error: persisted.error,
                project_filter: None,
                generation: 0,
                title_input: inputs.session_title,
                editing_title: None,
                pending_titles: HashMap::new(),
                pending_title_focus: false,
                pending_archive: None,
                pending_delete: None,
                pending_move: None,
                import: None,
                import_generation: 0,
                archived_expanded: false,
                _title_subscription: subscriptions.session_title,
            },
            activity: session::ActivityState {
                agents: HashMap::new(),
                row_focus: HashMap::new(),
                background_jobs: Vec::new(),
                run_statuses: HashMap::new(),
                recent_completions: HashMap::new(),
                recent_completion_expiries: HashMap::new(),
                system_notification_targets: HashMap::new(),
            },
            composer: composer::ComposerState {
                input: inputs.composer,
                project_files: Vec::new(),
                project_files_project: None,
                project_files_loading: None,
                sessions: persisted.composer_sessions,
                history_marker: None,
                escape_armed: None,
                images: composer_images,
                pastes: composer_pastes,
                focus: inputs.composer_focus,
                pending_restore: None,
                pending_submissions: HashMap::new(),
                _subscription: subscriptions.composer,
            },
            navigation: navigation::NavigationState {
                picker: None,
                pending_model_access: None,
                picker_return_focus: None,
                search: inputs.search,
                search_focus: inputs.search_focus,
                chat: ui::navigation::ChatNavigation {
                    focus: cx.focus_handle(),
                    activation: Default::default(),
                    activation_focus: None,
                    activation_blur: None,
                    return_shortcut: None,
                },
                _search_subscription: subscriptions.search,
            },
            workspace: workspace::WorkspaceState {
                editor: workspace::EditorState {
                    view: None,
                    terminal_editor_view: None,
                    terminal_editors: HashMap::new(),
                    active_review: None,
                    project_editors: HashMap::new(),
                    session_tabs: HashMap::new(),
                    ready: false,
                    request_generation: 0,
                    return_focus: None,
                },
                terminal: workspace::TerminalState {
                    view: None,
                    project: None,
                    project_terminals: HashMap::new(),
                },
                native_surface_snapshot: None,
                native_surface_refresh: None,
                native_surface_covered: false,
                tooltip_watch: None,
                surface: AppSurface::Chat,
                session_surfaces: HashMap::new(),
                worker_profile_editor: workspace::worker_tasks::WorkerProfileEditor::default(),
                runtime_picker: workspace::runtime_picker::RuntimePickerState::default(),
                send_to_chat: None,
                send_to_chat_capture: None,
                code_tasks: Default::default(),
            },
            settings: workspace::SettingsState {
                harness_profiles,
                harness_profile_name: inputs.harness_profile_name,
                harness_profile_executable: inputs.harness_profile_executable,
                harness_profile_data_directory: inputs.harness_profile_data_directory,
                harness_profile_backend: crate::agents::Backend::Claude,
                harness_profile_error: None,
                themes: workspace::theme_settings::ThemeSettings::load(
                    persisted.theme_css.as_deref(),
                    persisted.active_theme.as_deref(),
                ),
                network_proxy_input: inputs.network_proxy,
                network_proxy_error: None,
                proxy_save: None,
                mcp_error: None,
                expand_transcript_folders: persisted.expand_transcript_folders,
                editor_choice: persisted.editor_choice,
                editor_error: None,
                transcript_error: None,
                _network_proxy_subscription: subscriptions.network_proxy,
            },
            extensions: extensions::ExtensionState {
                active: ExtensionUiState::default(),
                parked: None,
                restored_dialog_id: None,
                dismissed_restored_dialog_id: None,
                notification_expiries: HashMap::new(),
                pending_dialog_setup: false,
                pending_title: None,
                pending_editor_text: None,
                dialog_input: inputs.dialog,
                dialog_focus: inputs.dialog_focus,
                dialog_return_focus: None,
            },
            views: views::AppViews {
                session_rail: regions.session_rail,
                archived_session_rail: regions.archived_session_rail,
                transcript: regions.transcript,
                composer: regions.composer,
                run_panel: regions.run_panel,
                workgraph: regions.workgraph,
                workgraph_detail: regions.workgraph_detail,
                workgraph_sidebar: regions.workgraph_sidebar,
                workgraph_inspector_issue: None,
                notification_panel: Default::default(),
                archived_panel: Default::default(),
                rail_region_height: None,
            },
            overlays: views::AppOverlays {
                view: Default::default(),
                image_preview: None,
                image_preview_focus: cx.focus_handle(),
                image_preview_return_focus: None,
                sheet_focus: cx.focus_handle(),
                sheet_return_focus: None,
                post_render_focus: None,
            },
            lifecycle: infrastructure::AppLifecycle {
                performance_monitor: performance.monitor,
                pending_session_switch: None,
                pending_quit: None,
                _performance_task: performance.task,
                _window_placement_subscription: subscriptions.window_placement,
                _event_task: tasks.runtime_events,
                _workgraph_update_task: tasks.workgraph_updates,
                _worker_update_task: tasks.worker_updates,
                _worker_notice_task: tasks.worker_notices,
            },
            worker_notices: notice_board,
        };
        this.activate_theme(cx);
        this.initialize_chat_navigation(window, cx);
        this.request_repository_refresh(cx);
        this
    }
}
