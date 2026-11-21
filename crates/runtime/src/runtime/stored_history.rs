use super::*;

const FIRST_PAGE_MESSAGES: usize = 64;

pub(super) struct HistoryRequest<'a> {
    pub generation: u64,
    pub path: &'a std::path::Path,
    pub project: &'a std::path::Path,
    pub kind: HistoryLoadKind,
    pub sender: &'a mpsc::Sender<HistoryUpdate>,
    pub wake: &'a thread::Thread,
}

impl HistoryRequest<'_> {
    fn send(&self, update: HistoryUpdate) {
        if self.sender.send(update).is_ok() {
            self.wake.unpark();
        }
    }

    fn result(&self, result: Result<LoadedHistory, String>) -> HistoryResult {
        HistoryResult {
            generation: self.generation,
            kind: self.kind,
            path: self.path.to_owned(),
            project: self.project.to_owned(),
            result,
        }
    }

    pub(super) fn finish(&self, result: Result<LoadedHistory, String>) {
        self.send(HistoryUpdate::Finished(self.result(result)));
    }

    fn tail(&self, page: agents::HistoryPage) {
        self.send(HistoryUpdate::Tail {
            generation: self.generation,
            page,
        });
    }

    pub(super) fn read(
        &self,
        config: &AgentLaunchConfig,
        harness: Backend,
        repository: Option<&SharedStateStore>,
    ) -> Result<Option<LoadedHistory>, String> {
        let session = TranscriptSession {
            harness,
            path: self.path,
            project: self.project,
            profile_id: config.profile_id.as_deref(),
        };
        let selection = self.kind == HistoryLoadKind::Selection;
        let mut expected_revision = Some(0);
        let mut published_local = false;
        if let Some(repo) = repository {
            let local = if selection {
                repo.load_transcript_tail(session, FIRST_PAGE_MESSAGES)
            } else {
                repo.load_transcript(session, Some(0))
            };
            match local {
                Ok(Some(page)) => {
                    expected_revision = Some(page.revision);
                    let source_matches = |stored: &StoredTranscript| {
                        stored.complete_from_start
                            && stored
                                .metadata
                                .get("sourceRevision")
                                .filter(|v| !v.is_null())
                                .is_some_and(|saved| {
                                    agents::local_history_revision(
                                        config,
                                        harness,
                                        self.path,
                                        self.project,
                                    )
                                    .as_ref()
                                        == Some(saved)
                                })
                    };
                    // A catalog refresh can be metadata-only. If its native source is
                    // unchanged, keep the provider out of the document read as well.
                    if selection || source_matches(&page) {
                        let full = if page.messages.len() == page.total {
                            Ok(Some(page))
                        } else {
                            if selection {
                                self.tail(agents::HistoryPage {
                                    content: page.messages,
                                    coverage: agents::HistoryCoverage::Suffix,
                                });
                            }
                            repo.load_transcript(session, None)
                        };
                        match full {
                            Ok(Some(stored)) => {
                                expected_revision = Some(stored.revision);
                                let current = source_matches(&stored);
                                self.send(HistoryUpdate::Saved {
                                    complete: stored.complete,
                                    result: self.result(Ok(saved_history(stored))),
                                    refreshing: !current,
                                });
                                if current {
                                    return Ok(None);
                                }
                                published_local = true;
                            }
                            Ok(None) => {}
                            Err(error) => {
                                zlog::warn!("Read full saved conversation: {error}");
                            }
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    expected_revision = None; // A failed read cannot authorize a replacement.
                    zlog::warn!("Read saved conversation: {error}");
                }
            }
        }
        let mut publish_page = |page| self.tail(page);
        let preview = selection && !published_local;
        let (mut history, stable_revision) = if repository.is_some() {
            agents::load_session_history_for_import(
                config,
                harness,
                self.path,
                self.project,
                preview.then_some(&mut publish_page as &mut dyn FnMut(agents::HistoryPage)),
            )?
        } else if preview {
            (
                agents::load_session_history_with_page(
                    config,
                    harness,
                    self.path,
                    self.project,
                    &mut publish_page,
                )?,
                None,
            )
        } else {
            (
                agents::load_session_history_for_profile(config, harness, self.path, self.project)?,
                None,
            )
        };
        if let (Some(repo), Some(expected)) = (repository, expected_revision) {
            let metadata = json!({
                "sourceRevision": stable_revision,
                "model": history.model,
                "thinkingLevel": history.thinking_level,
                "pendingQuestion": history.pending_question,
            });
            match repo.replace_transcript(session, expected, &history.messages, &metadata) {
                Ok(true) => {
                    repo.restore_transcript_links(session, &mut history.messages)?;
                }
                Ok(false) => {
                    // A live writer or a newer import won while this backend was reading.
                    if let Some(newer) = repo.load_transcript(session, None)? {
                        self.send(HistoryUpdate::Saved {
                            complete: newer.complete,
                            result: self.result(Ok(saved_history(newer))),
                            refreshing: false,
                        });
                        return Ok(None);
                    }
                }
                Err(error) => {
                    zlog::warn!("Save imported conversation: {error}");
                }
            }
        }
        Ok(Some(history))
    }
}

fn saved_history(stored: StoredTranscript) -> LoadedHistory {
    LoadedHistory {
        messages: Arc::new(stored.messages),
        model: stored
            .metadata
            .get("model")
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
        thinking_level: stored
            .metadata
            .get("thinkingLevel")
            .and_then(Value::as_str)
            .map(str::to_owned),
        pending_question: stored
            .metadata
            .get("pendingQuestion")
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
        prompt_deliveries: None,
    }
}
