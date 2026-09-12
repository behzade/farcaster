use serde::Deserialize;
use serde_json::Value;

use crate::agents::{SessionHistory, SessionOperation, SessionResponsePayload};

/// Decode known response bodies before they leave the Pi adapter. A malformed
/// body is a correlated command failure, not a successful empty catalog.
pub(super) fn decode(
    operation: SessionOperation,
    data: Value,
) -> Result<SessionResponsePayload, String> {
    use SessionResponsePayload as Payload;
    let result = (|| -> Result<Payload, serde_json::Error> {
        Ok(match operation {
            SessionOperation::LoadState => Payload::LoadState(serde_json::from_value(data)?),
            SessionOperation::LoadHistory => {
                #[derive(Deserialize)]
                struct History {
                    entries: Vec<Value>,
                }
                let history: History = serde_json::from_value(data)?;
                Payload::LoadHistory(SessionHistory::Replace(
                    super::session_files::project_display_history(&history.entries),
                ))
            }
            SessionOperation::LoadUsage => Payload::LoadUsage(serde_json::from_value(data)?),
            SessionOperation::ListModels => {
                #[derive(Deserialize)]
                struct Models {
                    models: Vec<crate::agents::extensions::Model>,
                }
                Payload::ListModels(serde_json::from_value::<Models>(data)?.models)
            }
            SessionOperation::ListReasoningLevels => {
                #[derive(Deserialize)]
                struct Levels {
                    levels: Vec<String>,
                }
                Payload::ListReasoningLevels(serde_json::from_value::<Levels>(data)?.levels)
            }
            SessionOperation::ListModes => {
                #[derive(Deserialize)]
                struct Modes {
                    modes: Vec<crate::agents::extensions::AgentMode>,
                    selected: Option<String>,
                }
                let modes: Modes = serde_json::from_value(data)?;
                Payload::ListModes {
                    modes: modes.modes,
                    selected: modes.selected,
                }
            }
            SessionOperation::ListCommands => {
                #[derive(Deserialize)]
                struct Commands {
                    commands: Vec<crate::agents::extensions::SlashCommand>,
                }
                Payload::ListCommands(serde_json::from_value::<Commands>(data)?.commands)
            }
            SessionOperation::SelectModel => Payload::SelectModel(serde_json::from_value(data)?),
            SessionOperation::ExportHtml => {
                #[derive(Deserialize)]
                struct Export {
                    path: String,
                }
                Payload::ExportHtml {
                    path: serde_json::from_value::<Export>(data)?.path,
                }
            }
            SessionOperation::ForkAt => {
                #[derive(Deserialize)]
                struct Fork {
                    cancelled: bool,
                }
                let fork: Fork = serde_json::from_value(data)?;
                Payload::ForkAt {
                    cancelled: fork.cancelled,
                }
            }
            SessionOperation::ConfigureSteering => Payload::ConfigureSteering,
            SessionOperation::ApplySteering => Payload::ApplySteering,
            SessionOperation::Prompt(mode) => Payload::Prompt(mode),
            SessionOperation::Abort => Payload::Abort,
            SessionOperation::Compact => Payload::Compact,
            SessionOperation::Rename => Payload::Rename,
            SessionOperation::SelectReasoning => Payload::SelectReasoning,
            SessionOperation::SelectServiceTier => Payload::SelectServiceTier,
            SessionOperation::SelectMode => Payload::SelectMode,
            SessionOperation::Other => Payload::Other,
        })
    })();
    result.map_err(|error| format!("invalid Pi {operation:?} response: {error}"))
}
