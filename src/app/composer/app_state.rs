use crate::app::*;

pub(in crate::app) struct ComposerState {
    pub(in crate::app) input: Entity<TextareaState>,
    pub(in crate::app) project_files: Vec<String>,
    pub(in crate::app) project_files_project: Option<PathBuf>,
    pub(in crate::app) project_files_loading: Option<PathBuf>,
    pub(in crate::app) sessions: ComposerSessions,
    pub(in crate::app) history_marker: Option<(String, usize, String)>,
    pub(in crate::app) escape_armed: Option<(String, Instant)>,
    pub(in crate::app) images: HashMap<String, Vec<ComposerImage>>,
    pub(in crate::app) pastes: HashMap<String, Vec<ComposerPaste>>,
    pub(in crate::app) focus: FocusHandle,
    pub(in crate::app) pending_restore: Option<(String, ComposerSnapshot)>,
    pub(in crate::app) pending_submissions: HashMap<String, PendingSubmission>,
    pub(in crate::app) _subscription: Subscription,
}
