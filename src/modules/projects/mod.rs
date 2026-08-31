mod adapter;
mod contract;
mod core;

pub(crate) use adapter::load_legacy;
pub(crate) use contract::{DraftSession, Registry};
pub(crate) use core::{add_unique, add_visible, remove, restore, select};

#[cfg(test)]
use adapter::{load_legacy as load_from, save_to};
#[cfg(test)]
use std::{fs, path::PathBuf};

#[cfg(test)]
mod tests;
