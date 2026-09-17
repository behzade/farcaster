#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CodexNotificationTier {
    Skills,
    Telemetry,
    Global,
    Thread,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CodexMethod<'a> {
    SkillsChanged,
    McpServerStartupStatusUpdated,
    AccountRateLimitsUpdated,
    Warning,
    ConfigWarning,
    RemoteControlStatusChanged,
    ThreadStarted,
    TurnStarted,
    AgentMessageDelta,
    PlanDelta,
    ReasoningTextDelta,
    ItemStarted,
    CommandExecutionOutputDelta,
    ItemCompleted,
    AutoApprovalReviewStarted,
    AutoApprovalReviewCompleted,
    GuardianWarning,
    TokenUsageUpdated,
    GoalUpdated,
    GoalCleared,
    TurnCompleted,
    ReasoningSummaryPartAdded,
    Error,
    ThreadSettingsUpdated,
    ThreadNameUpdated,
    ThreadStatusChanged,
    TurnDiffUpdated,
    TurnPlanUpdated,
    ServerRequestResolved,
    TerminalInteraction,
    FileChangeOutputDelta,
    CommandApproval,
    FileChangeApproval,
    PermissionsApproval,
    Unknown(&'a str),
}

impl<'a> CodexMethod<'a> {
    pub(super) fn parse(method: &'a str) -> Self {
        match method {
            "skills/changed" => Self::SkillsChanged,
            "mcpServer/startupStatus/updated" => Self::McpServerStartupStatusUpdated,
            "account/rateLimits/updated" => Self::AccountRateLimitsUpdated,
            "warning" => Self::Warning,
            "configWarning" => Self::ConfigWarning,
            "remoteControl/status/changed" => Self::RemoteControlStatusChanged,
            "thread/started" => Self::ThreadStarted,
            "turn/started" => Self::TurnStarted,
            "item/agentMessage/delta" => Self::AgentMessageDelta,
            "item/plan/delta" => Self::PlanDelta,
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                Self::ReasoningTextDelta
            }
            "item/started" => Self::ItemStarted,
            "item/commandExecution/outputDelta" => Self::CommandExecutionOutputDelta,
            "item/completed" => Self::ItemCompleted,
            "item/autoApprovalReview/started" => Self::AutoApprovalReviewStarted,
            "item/autoApprovalReview/completed" => Self::AutoApprovalReviewCompleted,
            "guardianWarning" => Self::GuardianWarning,
            "thread/tokenUsage/updated" => Self::TokenUsageUpdated,
            "thread/goal/updated" => Self::GoalUpdated,
            "thread/goal/cleared" => Self::GoalCleared,
            "turn/completed" => Self::TurnCompleted,
            "item/reasoning/summaryPartAdded" => Self::ReasoningSummaryPartAdded,
            "error" => Self::Error,
            "thread/settings/updated" => Self::ThreadSettingsUpdated,
            "thread/name/updated" => Self::ThreadNameUpdated,
            "thread/status/changed" => Self::ThreadStatusChanged,
            "turn/diff/updated" => Self::TurnDiffUpdated,
            "turn/plan/updated" => Self::TurnPlanUpdated,
            "serverRequest/resolved" => Self::ServerRequestResolved,
            "item/commandExecution/terminalInteraction" => Self::TerminalInteraction,
            "item/fileChange/outputDelta" => Self::FileChangeOutputDelta,
            "item/commandExecution/requestApproval" | "execCommandApproval" => {
                Self::CommandApproval
            }
            "item/fileChange/requestApproval" | "applyPatchApproval" => Self::FileChangeApproval,
            "item/permissions/requestApproval" => Self::PermissionsApproval,
            _ => Self::Unknown(method),
        }
    }

    pub(super) const fn tier(self) -> CodexNotificationTier {
        match self {
            Self::SkillsChanged => CodexNotificationTier::Skills,
            Self::McpServerStartupStatusUpdated | Self::AccountRateLimitsUpdated => {
                CodexNotificationTier::Telemetry
            }
            Self::Warning | Self::ConfigWarning | Self::RemoteControlStatusChanged => {
                CodexNotificationTier::Global
            }
            _ => CodexNotificationTier::Thread,
        }
    }

    pub(super) const fn is_approval_request(self) -> bool {
        matches!(
            self,
            Self::CommandApproval | Self::FileChangeApproval | Self::PermissionsApproval
        )
    }
}

#[cfg(test)]
#[path = "notification_tests.rs"]
mod tests;
