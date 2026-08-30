pub(crate) mod approval;
pub(super) mod policy;

pub(super) use super::{AccessPolicy, FilesystemAccess, NetworkAccess};

pub(crate) trait PolicyValidator: Send + Sync {
    fn validate(&self, policy: &[u8]) -> Result<(), String>;
}
