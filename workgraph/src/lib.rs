mod adapter;
mod application;
mod contract;
mod core;

pub use adapter::{SqliteAdapter, SqliteTransaction};
pub use application::{add_node, create_plan, link_session, load_plan};
pub use contract::{
    CompletionRequirement, Edge, EditAction, EditRequest, EditResult, Evidence, EvidenceKind,
    IdempotencyReceipt, Node, NodeDraft, Outcome, Plan, PlanSnapshot, ProjectGraph,
    ProjectSelection, SearchRequest, SearchResult, SessionLink, StoredProject, TaskCompletion,
    TaskOwner, TaskState, Walk, WalkStep,
};
pub use core::{
    Persistence, PersistenceError, TransactionMode, WorkGraph, WorkGraphError, WorkGraphTransaction,
};

#[cfg(test)]
mod adapter_tests;
#[cfg(test)]
mod application_tests;
#[cfg(test)]
mod core_tests;
