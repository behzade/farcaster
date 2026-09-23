use super::*;
use crate::agents::Backend;

pub(super) struct PersistedState {
    pub(super) projects: projects::ProjectList,
    pub(super) drafts: Vec<sessions::DraftSession>,
    pub(super) error: Option<String>,
    pub(super) session_order: Vec<i64>,
    pub(super) session_folders: crate::app::session_folders::SessionFolders,
    pub(super) selected_draft: String,
    pub(super) preferred_harness: Option<Backend>,
    pub(super) draft_session_ids: HashMap<String, i64>,
    pub(super) composer_sessions: ComposerSessions,
    pub(super) submitted_drafts: HashMap<String, Option<PathBuf>>,
    pub(super) saved_proxy: Option<String>,
    pub(super) expand_transcript_folders: bool,
    pub(super) editor_choice: crate::storage::EditorChoice,
}

pub(super) fn load(project: &Path, saved_proxy: Option<String>) -> PersistedState {
    let registry_timing =
        crate::app::infrastructure::performance::StartupTiming::new("app.load_registry");
    let (mut projects, mut error) = match project_registry::load() {
        Ok(projects) => (projects, None),
        Err(error) => (projects::ProjectList::default(), Some(error)),
    };
    drop(registry_timing);

    projects::select(
        &mut projects.projects,
        &projects.excluded_projects,
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
    let session_folders =
        match crate::app::persistence::open().and_then(|store| store.load_session_folders()) {
            Ok(folders) => folders,
            Err(load_error) => {
                error.get_or_insert(load_error);
                Default::default()
            }
        };

    let preferred_harness = match crate::app::persistence::open()
        .and_then(|store| store.load_preferred_harness(project))
    {
        Ok(harness) => harness,
        Err(load_error) => {
            error.get_or_insert(load_error);
            None
        }
    };
    let mut drafts =
        match crate::app::persistence::open().and_then(|store| sessions::load_drafts(&*store)) {
            Ok(drafts) => drafts,
            Err(load_error) => {
                error.get_or_insert(load_error);
                Vec::new()
            }
        };
    let draft_timing =
        crate::app::infrastructure::performance::StartupTiming::new("app.create_draft");
    let initial_draft = match session::draft_store::new(project.to_path_buf(), preferred_harness) {
        Ok(draft) => draft,
        Err(load_error) => {
            error.get_or_insert(load_error);
            sessions::DraftSession::with_id(
                preferred_harness,
                format!("untracked-draft-{}", std::process::id()),
                project.to_path_buf(),
            )
        }
    };
    drop(draft_timing);

    let selected_draft = initial_draft.id.clone();
    drafts.push(initial_draft);
    let draft_session_ids = drafts
        .iter()
        .map(|draft| (draft.id.clone(), draft.app_session_id))
        .collect::<HashMap<_, _>>();

    let save_registry_timing =
        crate::app::infrastructure::performance::StartupTiming::new("app.save_registry");
    if error.is_none()
        && let Err(save_error) = project_registry::save(&projects)
    {
        error = Some(save_error);
    }
    drop(save_registry_timing);

    let composer_timing =
        crate::app::infrastructure::performance::StartupTiming::new("app.load_composer_sessions");
    let (composer_sessions, composer_error) =
        crate::app::composer::sessions::load(draft_target(&selected_draft));
    drop(composer_timing);
    if error.is_none() {
        error = composer_error;
    }

    let submitted_drafts = sessions::submitted_draft_associations(&drafts);
    let expand_transcript_folders = crate::app::persistence::open()
        .and_then(|store| store.load_expand_transcript_folders())
        .unwrap_or_else(|load_error| {
            error.get_or_insert(load_error);
            false
        });
    let editor_choice = crate::app::persistence::open()
        .and_then(|store| store.load_editor_choice())
        .unwrap_or_else(|load_error| {
            error.get_or_insert(load_error);
            Default::default()
        });

    PersistedState {
        projects,
        drafts,
        error,
        session_order,
        session_folders,
        selected_draft,
        preferred_harness,
        draft_session_ids,
        composer_sessions,
        submitted_drafts,
        saved_proxy,
        expand_transcript_folders,
        editor_choice,
    }
}
