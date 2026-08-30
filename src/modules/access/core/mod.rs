pub(crate) mod approval;
pub(super) mod network;
pub(super) mod policy;

pub(super) use super::{AccessPolicy, FilesystemAccess, NetworkAccess, NetworkConfiguration};

pub(crate) trait PolicyValidator: Send + Sync {
    fn validate(&self, policy: &[u8]) -> Result<(), String>;
}
