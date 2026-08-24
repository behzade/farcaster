//! App orchestration for repository filesystem watchers.

use std::{path::PathBuf, time::Duration};

use gpui::Context;

use super::{super::PiApp, RepositoryLocation};
use crate::repository::watcher::{RepositoryWatchEvent, RepositoryWatcher};

const WATCH_DEBOUNCE: Duration = Duration::from_millis(150);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum WatchBinding {
    Discovery(PathBuf),
    Repository(RepositoryLocation),
}

impl PiApp {
    pub(super) fn install_repository_watcher(
        &mut self,
        location: RepositoryLocation,
        cx: &mut Context<Self>,
    ) -> bool {
        let binding = WatchBinding::Repository(location.clone());
        self.install_watcher(binding, || RepositoryWatcher::start(&location), cx)
    }

    pub(super) fn install_repository_discovery_watcher(&mut self, cx: &mut Context<Self>) -> bool {
        let project = self.repository.project.clone();
        let binding = WatchBinding::Discovery(project.clone());
        self.install_watcher(binding, || RepositoryWatcher::start_discovery(&project), cx)
    }

    fn install_watcher(
        &mut self,
        binding: WatchBinding,
        start: impl FnOnce() -> Result<
            (
                RepositoryWatcher,
                async_channel::Receiver<RepositoryWatchEvent>,
            ),
            String,
        >,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.repository.watcher_binding.as_ref() == Some(&binding)
            && self.repository.watcher.is_some()
        {
            return false;
        }
        self.repository.watcher = None;
        self.repository.watcher_binding = None;
        self.repository.watcher_generation = self.repository.watcher_generation.saturating_add(1);
        let generation = self.repository.watcher_generation;
        let (watcher, events) = match start() {
            Ok(watcher) => watcher,
            Err(error) => {
                let changed = self.repository.watcher_error.as_deref() != Some(error.as_str());
                self.repository.watcher_error = Some(error);
                return changed;
            }
        };
        self.repository.watcher = Some(watcher);
        self.repository.watcher_binding = Some(binding);
        let changed = self.repository.watcher_error.take().is_some();
        cx.spawn(async move |weak, cx| {
            while let Ok(first) = events.recv().await {
                let (mut changed, mut error) = match first {
                    RepositoryWatchEvent::Changed => (true, None),
                    RepositoryWatchEvent::Failed(error) => (false, Some(error)),
                };
                loop {
                    cx.background_executor().timer(WATCH_DEBOUNCE).await;
                    let mut received = false;
                    while let Ok(event) = events.try_recv() {
                        received = true;
                        match event {
                            RepositoryWatchEvent::Changed => changed = true,
                            RepositoryWatchEvent::Failed(next) => error = Some(next),
                        }
                    }
                    if !received {
                        break;
                    }
                }
                if weak
                    .update(cx, |this, cx| {
                        if this.repository.watcher_generation != generation {
                            return;
                        }
                        if let Some(error) = error {
                            this.repository.watcher_error = Some(error);
                            this.repository.watcher = None;
                            this.repository.watcher_binding = None;
                            this.repository.watcher_generation =
                                this.repository.watcher_generation.saturating_add(1);
                            this.notify_run_panel(cx);
                            return;
                        }
                        if changed {
                            this.request_repository_refresh(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        changed
    }
}
