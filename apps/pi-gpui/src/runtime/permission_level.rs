//! Independent filesystem and network access modes for the Pi child process.

use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum FileAccessMode {
    ReadOnly,
    #[default]
    Sandboxed,
    Full,
}

impl FileAccessMode {
    pub(crate) fn all() -> [Self; 3] {
        [Self::ReadOnly, Self::Sandboxed, Self::Full]
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "Read-only",
            Self::Sandboxed => "Sandboxed",
            Self::Full => "Full",
        }
    }

    pub(crate) fn flag_value(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Sandboxed => "sandboxed",
            Self::Full => "full",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum NetworkAccessMode {
    #[default]
    Sandboxed,
    Full,
}

impl NetworkAccessMode {
    pub(crate) fn all() -> [Self; 2] {
        [Self::Sandboxed, Self::Full]
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Sandboxed => "Sandboxed",
            Self::Full => "Full",
        }
    }

    pub(crate) fn flag_value(self) -> &'static str {
        match self {
            Self::Sandboxed => "sandboxed",
            Self::Full => "full",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PermissionLevel {
    pub(crate) files: FileAccessMode,
    pub(crate) network: NetworkAccessMode,
}

impl PermissionLevel {
    pub(crate) fn label(self) -> String {
        format!(
            "Files: {} · Network: {}",
            self.files.label(),
            self.network.label()
        )
    }

    pub(crate) fn with_files(self, files: FileAccessMode) -> Self {
        Self { files, ..self }
    }

    pub(crate) fn with_network(self, network: NetworkAccessMode) -> Self {
        Self { network, ..self }
    }
}

impl RuntimeOwner {
    pub(super) fn set_permission_level(&mut self, level: PermissionLevel) {
        if self.process_command.permission_level == level {
            return;
        }
        self.process_command.permission_level = level;
        let session = self
            .snapshot
            .selected_session
            .clone()
            .or_else(|| self.active_session.clone());
        self.start_process(session);
    }
}
