use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TranscriptStamp {
    pub session_id: i64,
    pub revision: i64,
    pub prompt_event_seq: i64,
}

impl StateStore {
    pub(super) fn transcript_stamp_for_id(
        &self,
        id: i64,
    ) -> rusqlite::Result<Option<TranscriptStamp>> {
        use rusqlite::OptionalExtension;
        let mut statement = self.connection.prepare_cached(&format!(
            "SELECT revision, ({}) FROM transcript_documents WHERE session_id=?1",
            crate::transcript_viewport::PRESENTATION_STAMP,
        ))?;
        statement
            .query_row([id], |row| {
                Ok(TranscriptStamp {
                    session_id: id,
                    revision: row.get(0)?,
                    prompt_event_seq: row.get(1)?,
                })
            })
            .optional()
    }
}

pub struct ObservedSession<'a> {
    pub harness: Backend,
    pub project: &'a Path,
    pub path: &'a Path,
    pub profile_id: Option<&'a str>,
    pub draft: Option<&'a str>,
}

/// Keeps prompt submission, backend registration and catalog writes on the same session.
pub trait SessionRepository {
    fn transcript_stamp(
        &self,
        session: TranscriptSession<'_>,
    ) -> Result<Option<TranscriptStamp>, String>;
    fn replace_transcript_with_stamp(
        &self,
        session: TranscriptSession<'_>,
        expected_revision: i64,
        messages: &[serde_json::Value],
        metadata: &serde_json::Value,
    ) -> Result<Option<TranscriptStamp>, String>;
    fn load_transcript_display(
        &self,
        session: TranscriptSession<'_>,
        full_document: bool,
    ) -> Result<Option<StoredViewport>, String>;
    fn restore_transcript_links(
        &self,
        session: TranscriptSession<'_>,
        messages: &mut Arc<Vec<serde_json::Value>>,
    ) -> Result<(), String>;
    /// Includes a user-turn boundary when a bounded viewport page can reach it.
    fn load_transcript_tail(
        &self,
        session: TranscriptSession<'_>,
        minimum: usize,
    ) -> Result<Option<StoredTranscript>, String>;
    fn load_transcript(
        &self,
        session: TranscriptSession<'_>,
        limit: Option<usize>,
    ) -> Result<Option<StoredTranscript>, String>;
    fn replace_transcript(
        &self,
        session: TranscriptSession<'_>,
        expected_revision: i64,
        messages: &[serde_json::Value],
        metadata: &serde_json::Value,
    ) -> Result<bool, String>;
    fn upsert_transcript_items(
        &self,
        session: TranscriptSession<'_>,
        items: &[(String, serde_json::Value)],
        complete_from_start: bool,
    ) -> Result<(), String>;
    fn submit_prompt(&self, submission: PromptSubmission<'_>) -> Result<EnqueuedPrompt, String>;
    fn pending_prompt_target(
        &self,
        outbox_id: i64,
    ) -> Result<Option<sessions::SessionTarget>, String>;
    fn register_caller_session(
        &self,
        caller: &agents::CallerContext,
        family: Option<&agents::WorkerFamilyLink>,
    ) -> Result<i64, String>;
    fn register_execution_for_caller(
        &self,
        caller: &agents::CallerContext,
        execution: &agents::ExecutionBinding,
        family: Option<&agents::WorkerFamilyLink>,
    ) -> Result<i64, String>;
    fn record_session_facts(
        &self,
        session: ObservedSession<'_>,
        facts: &SessionFacts,
    ) -> Result<(), String>;
    fn read_session_facts(&self, harness: Backend, session: &Path) -> Result<SessionFacts, String>;
    fn record_session_metadata(
        &self,
        update: &agents::SessionMetadata,
    ) -> Result<SessionSummary, String>;
    fn transcript_prompt_overlay(
        &self,
        session: TranscriptSession<'_>,
        evidence: Option<&sessions::PromptDeliveryReconciliation>,
    ) -> Result<PromptHistoryOverlay, String>;
    fn dismiss_transcript_receipt(
        &self,
        session: TranscriptSession<'_>,
        receipt_id: &str,
    ) -> Result<(), String>;
    fn can_display_transcript_tail(&self, session: TranscriptSession<'_>) -> Result<bool, String>;
}

impl SessionRepository for SharedStateStore {
    fn transcript_stamp(
        &self,
        session: TranscriptSession<'_>,
    ) -> Result<Option<TranscriptStamp>, String> {
        self.with_transcript_connection(|store| {
            let tx = store
                .connection
                .unchecked_transaction()
                .map_err(|e| e.to_string())?;
            match session.find(&tx).map_err(|e| e.to_string())? {
                Some(id) => store.transcript_stamp_for_id(id).map_err(|e| e.to_string()),
                None => Ok(None),
            }
        })
    }
    fn replace_transcript_with_stamp(
        &self,
        session: TranscriptSession<'_>,
        expected_revision: i64,
        messages: &[serde_json::Value],
        metadata: &serde_json::Value,
    ) -> Result<Option<TranscriptStamp>, String> {
        self.with_transcript_connection(|store| {
            store.replace_conversation(session, expected_revision, messages, metadata)
        })
    }
    fn load_transcript_display(
        &self,
        session: TranscriptSession<'_>,
        full_document: bool,
    ) -> Result<Option<StoredViewport>, String> {
        self.with_transcript_connection(|store| {
            store
                .read_display(session, full_document)
                .map_err(|e| format!("read prepared viewport: {e}"))
        })
    }
    fn restore_transcript_links(
        &self,
        session: TranscriptSession<'_>,
        messages: &mut Arc<Vec<serde_json::Value>>,
    ) -> Result<(), String> {
        self.with_transcript_connection(|store| {
            store
                .restore_links(session, messages)
                .map_err(|error| format!("restore saved transcript links: {error}"))
        })
    }
    fn load_transcript_tail(
        &self,
        session: TranscriptSession<'_>,
        minimum: usize,
    ) -> Result<Option<StoredTranscript>, String> {
        self.with_transcript_connection(|store| {
            store.read_conversation(session, Some(minimum), true)
        })
    }
    fn load_transcript(
        &self,
        session: TranscriptSession<'_>,
        limit: Option<usize>,
    ) -> Result<Option<StoredTranscript>, String> {
        self.with_transcript_connection(|store| store.read_conversation(session, limit, false))
    }
    fn replace_transcript(
        &self,
        session: TranscriptSession<'_>,
        expected_revision: i64,
        messages: &[serde_json::Value],
        metadata: &serde_json::Value,
    ) -> Result<bool, String> {
        self.replace_transcript_with_stamp(session, expected_revision, messages, metadata)
            .map(|stamp| stamp.is_some())
    }
    fn upsert_transcript_items(
        &self,
        session: TranscriptSession<'_>,
        items: &[(String, serde_json::Value)],
        complete_from_start: bool,
    ) -> Result<(), String> {
        self.with_transcript_connection(|store| {
            store.write_conversation_items(session, items, complete_from_start)
        })
    }
    fn submit_prompt(&self, submission: PromptSubmission<'_>) -> Result<EnqueuedPrompt, String> {
        self.with(|store| store.insert_prompt(submission))
    }

    fn pending_prompt_target(
        &self,
        outbox_id: i64,
    ) -> Result<Option<sessions::SessionTarget>, String> {
        self.with(|store| store.pending_prompt_target(outbox_id))
    }

    fn register_caller_session(
        &self,
        caller: &agents::CallerContext,
        family: Option<&agents::WorkerFamilyLink>,
    ) -> Result<i64, String> {
        self.with(|store| store.register_caller_session(caller, family))
    }

    fn register_execution_for_caller(
        &self,
        caller: &agents::CallerContext,
        execution: &agents::ExecutionBinding,
        family: Option<&agents::WorkerFamilyLink>,
    ) -> Result<i64, String> {
        self.with(|store| store.register_execution_for_caller(caller, execution, family))
    }

    fn record_session_facts(
        &self,
        session: ObservedSession<'_>,
        facts: &SessionFacts,
    ) -> Result<(), String> {
        self.with(|store| store.record_session_facts(session, facts))
    }

    fn read_session_facts(&self, harness: Backend, session: &Path) -> Result<SessionFacts, String> {
        self.with(|store| store.load_session_facts(harness, session))
    }

    fn record_session_metadata(
        &self,
        update: &agents::SessionMetadata,
    ) -> Result<SessionSummary, String> {
        self.with(|store| store.update_session_metadata(update))
    }

    fn transcript_prompt_overlay(
        &self,
        session: TranscriptSession<'_>,
        evidence: Option<&sessions::PromptDeliveryReconciliation>,
    ) -> Result<PromptHistoryOverlay, String> {
        self.with_transcript_connection(|store| store.transcript_prompt_overlay(session, evidence))
    }

    fn dismiss_transcript_receipt(
        &self,
        session: TranscriptSession<'_>,
        receipt_id: &str,
    ) -> Result<(), String> {
        self.with_transcript_connection(|store| {
            if let Some(id) = session.find(&store.connection).map_err(|e| e.to_string())? {
                store.dismiss_prompt_receipt(id, receipt_id)?;
            }
            Ok(())
        })
    }

    fn can_display_transcript_tail(&self, session: TranscriptSession<'_>) -> Result<bool, String> {
        self.with_transcript_connection(|store| {
            let id = session.find(&store.connection).map_err(|e| e.to_string())?;
            store.can_display_tail_for_id(id)
        })
    }
}
