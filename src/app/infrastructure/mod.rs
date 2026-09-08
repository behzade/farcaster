use super::*;

pub(crate) mod launch;
#[cfg(target_os = "macos")]
mod menus;
pub(crate) mod paths;
pub(crate) mod performance;
pub(crate) mod persistence;
#[cfg(test)]
mod persistence_tests;
mod quit;
pub(crate) mod shell_environment;
