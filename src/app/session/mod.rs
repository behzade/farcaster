//! Session-facing application coordination.

use super::*;

pub(super) mod archive;
pub(super) mod deletion;
pub(in crate::app) mod drafts;
mod expiries;
pub(in crate::app) mod lifecycle;
mod titles;
