use super::*;

mod app_state;
pub(in crate::app) use app_state::AppLifecycle;

pub(crate) mod launch;
#[cfg(target_os = "macos")]
mod menus;
pub(crate) mod neovim_launch;
pub(crate) mod paths;
pub(crate) mod performance;
pub(crate) mod persistence;
#[cfg(test)]
mod persistence_tests;
pub(super) mod quit;
pub(crate) mod shell_environment;
