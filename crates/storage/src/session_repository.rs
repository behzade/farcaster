use super::*;

pub struct ObservedSession<'a> {
    pub harness: Backend,
    pub project: &'a Path,
    pub path: &'a Path,
    pub profile_id: Option<&'a str>,
    pub draft: Option<&'a str>,
}

/// Keeps prompt submission, backend registration and catalog writes on the same session.
pub trait SessionRepository {
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
}

impl SessionRepository for SharedStateStore {
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
}
