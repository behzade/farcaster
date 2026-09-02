mod adapter;
mod contract;
mod core;
mod trust;

pub(crate) use adapter::{is_temporary_project, load_legacy};
pub(crate) use contract::{
    AppliedTrust, DraftSession, Registry, StartupTrust, TrustChoice, TrustOption,
};
pub(crate) use core::{
    ProjectStore, add_unique, add_visible, allocate_session_id, load_registry, remove, restore,
    save_registry, select,
};
pub(crate) use trust::{
    apply, options, repository_execution_allowed, saved_decision, startup_trust,
};

#[cfg(test)]
use adapter::{load_legacy as load_from, save_to};
#[cfg(test)]
use std::{fs, path::PathBuf};

#[cfg(test)]
mod tests;
