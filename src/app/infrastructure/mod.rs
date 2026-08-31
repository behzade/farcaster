//! Process, persistence, and application lifecycle adapters.

use super::*;

pub(crate) mod launch;
pub(crate) mod paths;
pub(crate) mod performance;
pub(crate) mod persistence;
#[cfg(test)]
mod persistence_tests;
mod quit;
pub(crate) mod shell_environment;
