use super::*;
use std::path::Path;

impl RuntimeSnapshot {
    pub(super) fn session_facts(&self) -> SessionFacts {
        SessionFacts {
            metrics: self.stats.clone(),
            goal: self.session_goal.clone(),
            outcome: self.last_outcome,
            cache_usage: self.conversation.cache_usage(),
            service_tier: self.observed_service_tier.clone(),
        }
    }

    pub(super) fn restore_session_facts(&mut self, facts: SessionFacts) {
        self.stats = facts.metrics;
        self.session_goal = facts.goal;
        self.last_outcome = facts.outcome;
        if facts.cache_usage.prompt_tokens > self.conversation.cache_usage().prompt_tokens {
            conversation_mut(self).restore_cache_usage(facts.cache_usage);
        }
        self.observed_service_tier = facts.service_tier.clone();
        self.prefill_service_tier = facts.service_tier;
    }
}

impl RuntimeOwner {
    pub(super) fn session_facts(&self, path: &Path) -> SessionFacts {
        let mut saved = match self.state.as_ref().zip(self.harness) {
            Some((state, harness)) => state
                .with(|store| store.load_session_facts(harness, path))
                .unwrap_or_else(|error| {
                    zlog::warn!("Load session facts: {error}");
                    SessionFacts::default()
                }),
            None => SessionFacts::default(),
        };
        if self.snapshot.selected_session.as_deref() == Some(path) {
            let mut observed = self.snapshot.session_facts();
            observed.metrics =
                projection::stable_session_stats(&saved.metrics, observed.metrics, false);
            if self.snapshot.document_ready {
                return observed;
            }
            saved.metrics = observed.metrics;
        }
        saved
    }
}

#[cfg(test)]
#[path = "facts_tests.rs"]
mod tests;
