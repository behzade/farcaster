//! Composer state, completion, and submission coordination.

use super::*;

pub(in crate::app) mod completion;
pub(in crate::app) mod file_mentions;
pub(in crate::app) mod images;
pub(in crate::app) mod pastes;
pub(crate) mod prompt_fragments;
pub(crate) mod sessions;
#[cfg(test)]
mod sessions_tests;
pub(in crate::app) mod slash_commands;
mod state;
pub(in crate::app) mod submissions;
pub(crate) mod user_invocations;

pub(crate) use images::ComposerImage;
pub(crate) use pastes::ComposerPaste;
