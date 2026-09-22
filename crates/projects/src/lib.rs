mod adapter;
mod contract;
mod core;
mod trust;
pub mod trust_store;

pub use adapter::{is_temporary_project, load_legacy};
pub use contract::{AppliedTrust, ProjectList, Registry, StartupTrust, TrustChoice, TrustOption};
pub use core::{
    ProjectStore, add_unique, add_visible, load_projects, remove, restore, save_projects, select,
};
pub use trust::{
    TRUST_DESCRIPTION, apply, options, repository_execution_allowed, saved_decision, startup_trust,
};

#[cfg(test)]
use adapter::{load_legacy as load_from, save_to};
#[cfg(test)]
use std::{fs, path::PathBuf};

#[cfg(test)]
mod tests;
