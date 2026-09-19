use serde_json::Value;

use super::WorkerSessionTransport;
use crate::{
    SessionContextUsage, SessionEvent, SessionOperation, SessionResponse,
    SessionResponsePayload as Payload, SessionUsage, SessionUsageTokens,
    extensions::{Model, SessionState},
};

#[derive(Clone, Copy)]
pub(super) enum CatalogQuery {
    Models,
    Commands,
    Modes,
}

impl CatalogQuery {
    fn operation(self) -> SessionOperation {
        match self {
            Self::Models => SessionOperation::ListModels,
            Self::Commands => SessionOperation::ListCommands,
            Self::Modes => SessionOperation::ListModes,
        }
    }
}

impl WorkerSessionTransport {
    pub(super) fn response(&mut self, id: Option<String>, payload: Payload) {
        self.pending
            .push_back(SessionEvent::Response(SessionResponse::success(
                id, payload,
            )));
    }

    pub(super) fn catalog_response(&mut self, id: Option<String>, query: CatalogQuery) {
        let result = match query {
            CatalogQuery::Models => {
                serde_json::from_value(Value::Array(self.metadata.models.clone()))
                    .map(Payload::ListModels)
            }
            CatalogQuery::Commands => {
                serde_json::from_value(Value::Array(self.metadata.commands.clone()))
                    .map(Payload::ListCommands)
            }
            CatalogQuery::Modes => {
                serde_json::from_value(Value::Array(self.metadata.modes.clone())).map(|modes| {
                    Payload::ListModes {
                        modes,
                        selected: self.selected_mode.clone(),
                    }
                })
            }
        };
        match result {
            Ok(payload) => self.response(id, payload),
            Err(error) => {
                let operation = query.operation();
                self.pending
                    .push_back(SessionEvent::Response(SessionResponse::failure(
                        id,
                        operation,
                        format!("invalid {} {operation:?} catalog: {error}", self.harness),
                    )));
            }
        }
    }

    pub(super) fn sync_model_selection(&mut self) {
        if let Some(selection) = self.worker.model_selection() {
            self.model = selection.model;
            self.effort = selection.effort;
        }
    }

    pub(super) fn catalog_model(&self, provider: &str, id: &str) -> Model {
        let mut model: Model = self
            .metadata
            .models
            .iter()
            .find(|model| model["provider"] == provider && model["id"] == id)
            .and_then(|model| serde_json::from_value(model.clone()).ok())
            .unwrap_or_else(|| Model {
                id: id.into(),
                name: id.into(),
                provider: provider.into(),
                context_window: self.usage.context_window,
                reasoning: true,
                efforts: None,
                resolved_model: None,
                access_modes: None,
            });
        if model.context_window == 0 {
            // Some adapters report the live limit separately from their catalog.
            model.context_window = self.usage.context_window;
        }
        model
    }

    pub(super) fn reasoning_levels(&self) -> Vec<String> {
        self.model.as_ref().map_or_else(
            || self.metadata.efforts.clone(),
            |(provider, id)| {
                crate::model_efforts(&self.catalog_model(provider, id), &self.metadata.efforts)
            },
        )
    }

    pub(super) fn state(&self) -> SessionState {
        let model = self
            .model
            .as_ref()
            .map(|(provider, id)| self.catalog_model(provider, id));
        SessionState {
            model,
            service_tier: self.metadata.service_tier.clone(),
            service_tiers: self.metadata.service_tiers.clone(),
            thinking_level: self.effort.clone(),
            is_streaming: self.running,
            is_compacting: false,
            session_file: Some(self.path.to_string_lossy().into_owned()),
            session_id: self.locator.clone(),
            session_name: self.metadata.session_name.clone(),
            auto_compaction_enabled: true,
            message_count: self.message_count,
            pending_message_count: 0,
        }
    }

    pub(super) fn session_usage(&self) -> SessionUsage {
        SessionUsage {
            context_usage: Some(SessionContextUsage {
                tokens: Some(self.usage.turn.total()),
                context_window: self.usage.context_window,
                percent: Some(if self.usage.context_window > 0 {
                    self.usage.turn.total() as f64 * 100.0 / self.usage.context_window as f64
                } else {
                    0.0
                }),
            }),
            tokens: SessionUsageTokens {
                input: self.usage.session.input,
                output: self.usage.session.output,
                cache_read: self.usage.session.cache_read,
                cache_write: self.usage.session.cache_write,
                total_tokens: self.usage.session.total(),
            },
            total_cost: self.usage.cost,
        }
    }
}
