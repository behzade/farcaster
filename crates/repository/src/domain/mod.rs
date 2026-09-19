use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SnapshotToken {
    Git(Arc<[u8]>),
    Jujutsu(Arc<str>),
}
