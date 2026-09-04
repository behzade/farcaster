use super::*;

pub(super) struct PersistedState {
    pub(super) registry: projects::Registry,
    pub(super) error: Option<String>,
    pub(super) session_order: Vec<i64>,
    pub(super) selected_draft: String,
    pub(super) draft_session_ids: HashMap<String, i64>,
    pub(super) composer_sessions: ComposerSessions,
    pub(super) submitted_drafts: HashMap<String, Option<PathBuf>>,
    pub(super) saved_proxy: Option<String>,
}

pub(super) fn load(project: &Path) -> PersistedState {
    let registry_timing =
        crate::app::infrastructure::performance::StartupTiming::new("app.load_registry");
    let (mut registry, mut error) = match project_registry::load() {
        Ok(registry) => (registry, None),
        Err(error) => (projects::Registry::default(), Some(error)),
    };
    drop(registry_timing);

    projects::select(
        &mut registry.projects,
        &registry.excluded_projects,
        project.to_path_buf(),
    );

    let session_order_timing =
        crate::app::infrastructure::performance::StartupTiming::new("app.load_session_order");
    let session_order = match project_registry::load_app_session_order() {
        Ok(order) => order,
        Err(load_error) => {
            error.get_or_insert(load_error);
            Vec::new()
        }
    };
    drop(session_order_timing);

    let draft_timing =
        crate::app::infrastructure::performance::StartupTiming::new("app.create_draft");
    let initial_draft = match project_registry::new_draft(project.to_path_buf(), "pi") {
        Ok(draft) => draft,
        Err(load_error) => {
            error.get_or_insert(load_error);
            projects::DraftSession::with_id(
                format!("untracked-draft-{}", std::process::id()),
                project.to_path_buf(),
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
    if error.is_none()
        && let Err(save_error) = project_registry::save(&registry)
    {
        error = Some(save_error);
    }
    drop(save_registry_timing);

    let composer_timing =
        crate::app::infrastructure::performance::StartupTiming::new("app.load_composer_sessions");
    let (composer_sessions, composer_error) = ComposerSessions::load(draft_target(&selected_draft));
    drop(composer_timing);
    if error.is_none() {
        error = composer_error;
    }

    let submitted_drafts = drafts::submitted_draft_associations(&registry.drafts);
    let proxy_timing =
        crate::app::infrastructure::performance::StartupTiming::new("app.load_proxy");
    let saved_proxy = crate::app::infrastructure::persistence::StateStore::open()
        .and_then(|store| crate::access::load_proxy(&store))
        .unwrap_or(None);
    drop(proxy_timing);

    PersistedState {
        registry,
        error,
        session_order,
        selected_draft,
        draft_session_ids,
        composer_sessions,
        submitted_drafts,
        saved_proxy,
    }
}
