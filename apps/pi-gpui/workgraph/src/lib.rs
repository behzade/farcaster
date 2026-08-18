//! Reusable durable work-graph module.
//!
//! `contract` owns shared values, `core` owns graph behavior behind a
//! persistence interface, and `adapter` contains the SQLite implementation.
//! Applications choose and assemble the adapter with the core.

pub mod adapter;
mod adapter_rows;
pub mod contract;
pub mod core;

#[cfg(test)]
mod adapter_tests;
#[cfg(test)]
mod core_tests;
