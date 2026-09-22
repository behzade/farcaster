use std::path::PathBuf;

use farcaster_sessions::DraftSession;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupTrust {
    Ready,
    Prompt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustChoice {
    TrustProject,
    TrustParent,
    DistrustProject,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustOption {
    pub label: String,
    pub choice: TrustChoice,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppliedTrust {
    pub trusted: bool,
    pub saved_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Registry {
    pub projects: Vec<PathBuf>,
    #[serde(default, skip_serializing)]
    pub excluded_projects: Vec<PathBuf>,
    pub drafts: Vec<DraftSession>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProjectList {
    pub projects: Vec<PathBuf>,
    pub excluded_projects: Vec<PathBuf>,
}
