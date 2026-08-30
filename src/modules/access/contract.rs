#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FilesystemAccess {
    ReadOnly,
    Sandboxed,
    Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NetworkAccess {
    Sandboxed,
    Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AccessPolicy {
    pub(crate) filesystem: FilesystemAccess,
    pub(crate) network: NetworkAccess,
}

impl AccessPolicy {
    pub(crate) const fn unrestricted(self) -> bool {
        matches!(self.filesystem, FilesystemAccess::Full)
            && matches!(self.network, NetworkAccess::Full)
    }
}
