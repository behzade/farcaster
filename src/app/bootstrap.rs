use super::*;

mod inputs;
mod persisted;
mod regions;
mod subscriptions;
mod tasks;

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
        let persisted = persisted::load(&project);

        let runtime_timing =
            crate::app::infrastructure::performance::StartupTiming::new("app.spawn_runtime");
        let runtime = RuntimeHandle::spawn(
            project.clone(),
            persisted.selected_draft.clone(),
            None,
            persisted.saved_proxy.clone(),
        );
        drop(runtime_timing);

        let inputs = inputs::create(
            &persisted.composer_sessions,
            persisted.saved_proxy.as_deref(),
            window,
            cx,
        );
        let subscriptions = subscriptions::create(&inputs, window, cx);
        let tasks = tasks::spawn(&runtime, workgraph_updates, worker_updates, cx);
        let performance = tasks::start_performance_monitor(window, cx);
        let regions = regions::create(&project, window, cx);

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
            session_order: persisted.session_order,
            session_drop_target: None,
            run_statuses: HashMap::new(),
            recent_completions: HashMap::new(),
            recent_completion_expiries: HashMap::new(),
            system_notification_target: None,
            projects: persisted.registry.projects,
            excluded_projects: persisted.registry.excluded_projects,
            drafts: persisted.registry.drafts,
            draft_session_ids: persisted.draft_session_ids,
            selected_draft: Some(persisted.selected_draft),
            preferred_harness: "pi".into(),
            submitted_drafts: persisted.submitted_drafts,
            sessions_error: persisted.error,
            session_project_filter: None,
            picker: None,
            picker_return_focus: None,
            session_generation: 0,
            runtime_generation: 0,
            composer: inputs.composer,
            composer_project_files: Vec::new(),
            composer_project_files_project: None,
            composer_project_files_loading: None,
            session_rail_view: regions.session_rail,
            archived_session_rail_view: regions.archived_session_rail,
            transcript_view: regions.transcript,
            composer_view: regions.composer,
            run_panel_view: regions.run_panel,
            workgraph_view: regions.workgraph,
            workgraph_detail_view: regions.workgraph_detail,
            workgraph_sidebar_view: regions.workgraph_sidebar,
            editor: None,
            editor_request_generation: 0,
            editor_return_focus: None,
            terminal: None,
            terminal_project: None,
            native_surface_snapshot: None,
            native_surface_covered: false,
            surface: AppSurface::Chat,
            workgraph_inspector_issue: None,
            composer_sessions: persisted.composer_sessions,
            session_surfaces: HashMap::new(),
            composer_history_marker: None,
            composer_escape_armed: None,
            composer_images: HashMap::new(),
            composer_pastes: HashMap::new(),
            search: inputs.search,
            search_focus: inputs.search_focus,
            session_title_input: inputs.session_title,
            worker_task_editor: workspace::worker_tasks::WorkerTaskEditor::default(),
            network_proxy_input: inputs.network_proxy,
            network_proxy_error: None,
            settings_application_modifier: ui::keybindings::application_modifier(),
            editing_session_title: None,
            pending_session_titles: HashMap::new(),
            pending_session_title_focus: false,
            dialog_input: inputs.dialog,
            composer_focus: inputs.composer_focus,
            dialog_focus: inputs.dialog_focus,
            dialog_return_focus: None,
            image_preview: None,
            image_preview_focus: cx.focus_handle(),
            image_preview_return_focus: None,
            sheet_focus: cx.focus_handle(),
            sheet_return_focus: None,
            overlays: views::overlay_state::OverlayViewState {
                draft_inspector: persisted.draft_inspector,
                ..Default::default()
            },
            performance_monitor: performance.monitor,
            _performance_task: performance.task,
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
            project_trust_backend: None,
            pending_project_trust_command: None,
            _composer_subscription: subscriptions.composer,
            _search_subscription: subscriptions.search,
            _session_title_subscription: subscriptions.session_title,
            _window_activation_subscription: subscriptions.window_activation,
            _window_placement_subscription: subscriptions.window_placement,
            _event_task: tasks.runtime_events,
            _workgraph_update_task: tasks.workgraph_updates,
            _worker_update_task: tasks.worker_updates,
        };
        this.request_repository_refresh(cx);
        this
    }
}
