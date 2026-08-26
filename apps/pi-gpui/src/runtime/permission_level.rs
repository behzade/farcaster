//! Session process permission level: OS sandbox vs unsandboxed host tools.

use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum PermissionLevel {
    #[default]
    Sandboxed,
    FullAccess,
}

impl PermissionLevel {
    pub(crate) fn all() -> [Self; 2] {
        [Self::Sandboxed, Self::FullAccess]
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Sandboxed => "Sandboxed",
            Self::FullAccess => "Full access",
        }
    }

    pub(crate) fn from_sandbox_disabled(disabled: bool) -> Self {
        if disabled {
            Self::FullAccess
        } else {
            Self::Sandboxed
        }
    }
}

impl RuntimeOwner {
    pub(super) fn set_permission_level(&mut self, level: PermissionLevel) {
        let sandbox_disabled = matches!(level, PermissionLevel::FullAccess);
        if self.process_command.sandbox_disabled == sandbox_disabled {
            return;
        }
        self.process_command.sandbox_disabled = sandbox_disabled;
        let session = self
            .snapshot
            .selected_session
            .clone()
            .or_else(|| self.active_session.clone());
        self.start_process(session);
    }
}
