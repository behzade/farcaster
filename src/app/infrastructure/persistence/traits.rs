use super::*;

impl crate::access::NetworkSettingsStore for StateStore {
    fn load_proxy(&self) -> Result<Option<String>, String> {
        self.load_network_proxy()
    }

    fn save_proxy(&self, proxy: Option<&str>) -> Result<(), String> {
        self.save_network_proxy(proxy)
    }
}

impl crate::agents::PromptStore for StateStore {
    fn has_queued_for(&self, paths: &[PathBuf]) -> Result<bool, String> {
        self.has_queued_prompts_for(paths)
    }

    fn enqueue(
        &self,
        target: &str,
        harness: &str,
        project: &Path,
        session: Option<&Path>,
        mode: PromptMode,
        message: &str,
        images: &[PromptImage],
    ) -> Result<i64, String> {
        self.enqueue_prompt(target, harness, project, session, mode, message, images)
    }

    fn enqueue_with_presentation(
        &self,
        target: &str,
        harness: &str,
        project: &Path,
        session: Option<&Path>,
        mode: PromptMode,
        message: &str,
        display_message: Option<&str>,
        invocation: Option<&str>,
        images: &[PromptImage],
    ) -> Result<i64, String> {
        self.enqueue_prompt_with_presentation(
            target,
            harness,
            project,
            session,
            mode,
            message,
            display_message,
            invocation,
            images,
        )
    }

    fn queued(&self) -> Result<Vec<QueuedPrompt>, String> {
        self.queued_prompts()
    }

    fn complete(&mut self, id: i64, target: &str, session: Option<&Path>) -> Result<(), String> {
        self.complete_prompt(id, target, session)
    }

    fn begin(&self, id: i64) -> Result<(), String> {
        self.begin_prompt(id)
    }

    fn fail(&self, id: i64, error: &str) -> Result<(), String> {
        self.fail_prompt(id, error)
    }
}

impl crate::sessions::SessionStore for StateStore {
    fn cached(&self, query: &str) -> Result<Vec<SessionSummary>, String> {
        self.cached_sessions(query)
    }

    fn index(&mut self, sessions: &[SessionSummary], prune_missing: bool) -> Result<(), String> {
        self.index_sessions(sessions, prune_missing)
    }

    fn relocate(
        &mut self,
        paths: &[(PathBuf, PathBuf)],
        target_project: &Path,
    ) -> Result<(), String> {
        self.relocate_session_paths(paths, target_project)
    }

    fn delete(&mut self, paths: &[PathBuf]) -> Result<(), String> {
        self.delete_session_state(paths)
    }

    fn set_archived(&self, path: &Path, archived: bool) -> Result<(), String> {
        self.set_session_archived(path, archived)
    }
}

impl crate::projects::ProjectStore for StateStore {
    fn allocate_session_id(&mut self, draft_id: &str, created_ms: u64) -> Result<i64, String> {
        self.allocate_app_session_id(draft_id, created_ms)
    }

    fn load_registry(&self) -> Result<Registry, String> {
        StateStore::load_registry(self)
    }

    fn save_registry(&mut self, registry: &Registry) -> Result<(), String> {
        StateStore::save_registry(self, registry)
    }
}

impl crate::repository::PreferenceStore for StateStore {
    fn load(&self) -> Result<BTreeMap<PathBuf, crate::repository::BackendPreference>, String> {
        self.load_repository_backend_preferences()?
            .into_iter()
            .map(|(project, preference)| preference.parse().map(|preference| (project, preference)))
            .collect()
    }

    fn save(
        &self,
        preferences: &BTreeMap<PathBuf, crate::repository::BackendPreference>,
    ) -> Result<(), String> {
        let preferences = preferences
            .iter()
            .map(|(project, preference)| (project.clone(), preference.as_str().to_owned()))
            .collect();
        self.save_repository_backend_preferences(&preferences)
    }
}
