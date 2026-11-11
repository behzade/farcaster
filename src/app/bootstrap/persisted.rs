use super::*;
use crate::agents::Backend;

pub(super) struct PersistedState {
    pub(super) projects: projects::ProjectList,
    pub(super) drafts: Vec<sessions::DraftSession>,
    pub(super) error: Option<String>,
    pub(super) session_order: Vec<i64>,
    pub(super) session_folders: crate::app::session_folders::SessionFolders,
    pub(super) initial_draft: sessions::DraftSession,
    pub(super) initial_session: Option<SessionSummary>,
    pub(super) preferred_harness: Option<Backend>,
    pub(super) preferred_profile_id: Option<String>,
    pub(super) draft_session_ids: HashMap<String, i64>,
    pub(super) composer_sessions: ComposerSessions,
    pub(super) submitted_drafts: HashMap<String, Option<PathBuf>>,
    pub(super) saved_proxy: Option<String>,
    pub(super) expand_transcript_folders: bool,
    pub(super) voice_enabled: bool,
    pub(super) voice_shortcut: Option<String>,
    pub(super) group_sessions_by_project: bool,
    pub(super) editor_command: String,
    pub(super) editor_choice: crate::storage::EditorChoice,
    pub(super) theme_css: Option<String>,
    pub(super) active_theme: Option<String>,
    pub(super) panel_layout: crate::app::infrastructure::persistence::PanelLayout,
    pub(super) theme_error: Option<String>,
}

pub(super) fn load(
    project: &Path,
    saved_proxy: Option<String>,
    cx: &mut Context<FarcasterApp>,
) -> PersistedState {
    let project_path = sessions::normalize_session_path(project);
    let project = project_path.as_path();
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
    let preferred_profile_id =
        match crate::app::persistence::open().and_then(|store| store.load_preferred_profile_id()) {
            Ok(id) => id,
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
    let (selected_id, initial_session) = crate::app::persistence::open()
        .and_then(|store| {
            let Some(id) = store.load_selected_session(project)? else {
                return Ok((None, None));
            };
            let session = store
                .cached_sessions("")?
                .into_iter()
                .find(|session| session.app_session_id == id);
            Ok((Some(id), session))
        })
        .unwrap_or_else(|load_error| {
            error.get_or_insert(load_error);
            (None, None)
        });
    let initial_draft = drafts
        .iter()
        .find(|draft| Some(draft.app_session_id) == selected_id)
        .cloned()
        .unwrap_or_else(|| {
            session::draft_store::new(
                project.to_path_buf(),
                preferred_harness,
                preferred_profile_id.clone(),
            )
        });
    drop(draft_timing);

    if initial_session.is_some() && initial_draft.app_session_id > 0 {
        // The selected draft has become a chat; keep its saved composer under the chat identity.
        match crate::app::persistence::open()
            .and_then(|mut store| store.remove_draft(&initial_draft.id))
        {
            Ok(()) => drafts.retain(|draft| draft.id != initial_draft.id),
            Err(save_error) => {
                error.get_or_insert(save_error);
            }
        }
    }
    if initial_session.is_none() && initial_draft.app_session_id <= 0 {
        drafts.push(initial_draft.clone());
    }
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
    let (composer_sessions, composer_error) = crate::app::composer::sessions::load(
        initial_session.as_ref().map_or_else(
            || draft_target(&initial_draft.id),
            |session| session_target(&session.path),
        ),
        cx,
    );
    drop(composer_timing);
    if error.is_none() {
        error = composer_error;
    }

    let submitted_drafts = sessions::submitted_draft_associations(&drafts);
    let group_sessions_by_project = crate::app::persistence::open()
        .and_then(|store| store.load_group_sessions_by_project())
        .unwrap_or_else(|load_error| {
            error.get_or_insert(load_error);
            false
        });
    let expand_transcript_folders = crate::app::persistence::open()
        .and_then(|store| store.load_expand_transcript_folders())
        .unwrap_or_else(|load_error| {
            error.get_or_insert(load_error);
            false
        });
    let voice_enabled = crate::app::persistence::open()
        .and_then(|store| store.load_voice_enabled())
        .unwrap_or_else(|load_error| {
            error.get_or_insert(load_error);
            false
        });
    let voice_shortcut = crate::app::persistence::open()
        .and_then(|store| store.load_voice_shortcut())
        .unwrap_or_else(|load_error| {
            error.get_or_insert(load_error);
            None
        });
    let editor_choice = crate::app::persistence::open()
        .and_then(|store| store.load_editor_choice())
        .unwrap_or_else(|load_error| {
            error.get_or_insert(load_error);
            Default::default()
        });

    let editor_command = crate::app::persistence::open()
        .and_then(|store| store.load_editor_command())
        .unwrap_or_else(|load_error| {
            error.get_or_insert(load_error);
            String::new()
        });

    let mut theme_error = None;
    let theme_css = crate::app::infrastructure::persistence::open()
        .and_then(|store| store.load_theme_css())
        .unwrap_or_else(|load_error| {
            theme_error = Some(load_error.clone());
            error.get_or_insert(load_error);
            None
        });
    let active_theme = crate::app::infrastructure::persistence::open()
        .and_then(|store| store.load_active_theme())
        .unwrap_or_else(|load_error| {
            theme_error.get_or_insert(load_error.clone());
            error.get_or_insert(load_error);
            None
        });

    let panel_layout = crate::app::infrastructure::persistence::open()
        .and_then(|store| store.load_panel_layout())
        .unwrap_or_else(|load_error| {
            error.get_or_insert(load_error);
            None
        })
        .unwrap_or_default();

    PersistedState {
        projects,
        drafts,
        error,
        session_order,
        session_folders,
        initial_draft,
        initial_session,
        preferred_harness,
        preferred_profile_id,
        draft_session_ids,
        composer_sessions,
        submitted_drafts,
        saved_proxy,
        expand_transcript_folders,
        voice_enabled,
        voice_shortcut,
        group_sessions_by_project,
        editor_choice,
        editor_command,
        theme_css,
        active_theme,
        panel_layout,
        theme_error,
    }
}
