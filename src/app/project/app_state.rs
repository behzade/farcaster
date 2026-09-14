use crate::app::*;

pub(in crate::app) struct ProjectState {
    pub(in crate::app) path: PathBuf,
    pub(in crate::app) registered: Vec<PathBuf>,
    pub(in crate::app) excluded: Vec<PathBuf>,
    pub(in crate::app) repository: repository::RepositoryState,
    pub(in crate::app) trust_error: Option<String>,
    pub(in crate::app) trust_project: Option<PathBuf>,
    pub(in crate::app) trust_backend: Option<Backend>,
    pub(in crate::app) pending_trust_command: Option<RuntimeCommand>,
}
