mod adapter;
mod contract;
mod core;

pub use adapter::{SqliteAdapter, SqliteTransaction};
pub use contract::{
    CompletionRequirement, Edge, EditAction, EditRequest, EditResult, Evidence, EvidenceKind,
    IdempotencyReceipt, Node, NodeDraft, Outcome, Plan, PlanSnapshot, ProjectGraph, SearchRequest,
    SearchResult, SessionLink, StoredProject, Walk, WalkStep,
};
pub use core::{
    Persistence, PersistenceError, TransactionMode, WorkGraph, WorkGraphError, WorkGraphTransaction,
};

#[cfg(test)]
mod adapter_tests;
#[cfg(test)]
mod core_tests;
