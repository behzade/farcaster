use super::*;

use futures::future::{Either, select};
use std::future::Future;

const NOTICE_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

struct NoticeRefresh {
    active: bool,
}

impl NoticeRefresh {
    fn update(&mut self, active: bool) -> bool {
        let redraw = self.active || active;
        self.active = active;
        redraw
    }
}

pub(super) struct BootstrapTasks {
    pub(super) runtime_events: Task<()>,
    pub(super) workgraph_updates: Task<()>,
    pub(super) worker_updates: Task<()>,
    pub(super) worker_notices: Task<()>,
}

pub(super) struct PerformanceState {
    pub(super) monitor: Option<crate::app::infrastructure::performance::PerformanceMonitor>,
    pub(super) task: Option<Task<()>>,
}

pub(super) fn spawn(
    runtime: &RuntimeHandle,
    workgraph_updates: async_channel::Receiver<()>,
    worker_updates: async_channel::Receiver<()>,
    notice_updates: async_channel::Receiver<()>,
    cx: &mut Context<FarcasterApp>,
) -> BootstrapTasks {
    let runtime_wake = runtime.wake_receiver();
    let runtime_events = cx.spawn(async move |weak, cx| {
        while runtime_wake.recv().await.is_ok() {
            if weak.update(cx, |this, cx| this.drain_runtime(cx)).is_err() {
                break;
            }
        }
    });
    let workgraph_updates = cx.spawn(async move |weak, cx| {
        while workgraph_updates.recv().await.is_ok() {
            if weak
                .update(cx, |this, cx| {
                    this.views
                        .workgraph_sidebar
                        .update(cx, |view, cx| view.invalidate_and_refresh(cx));
                    this.views.workgraph.update(cx, |view, cx| view.refresh(cx));
                })
                .is_err()
            {
                break;
            }
        }
    });
    let worker_updates = cx.spawn(async move |weak, cx| {
        while worker_updates.recv().await.is_ok() {
            if weak
                .update(cx, |this, cx| {
                    this.send(RuntimeCommand::RefreshSessions, cx);
                })
                .is_err()
            {
                break;
            }
        }
    });
    let worker_notices = cx.spawn(async move |weak, cx| {
        let mut refresh = match weak.update(cx, |this, _| NoticeRefresh {
            active: !this.worker_notices.snapshot(&this.project.path).is_empty(),
        }) {
            Ok(refresh) => refresh,
            Err(_) => return,
        };
        loop {
            if !wait_for_notice_refresh(
                &notice_updates,
                cx.background_executor().timer(NOTICE_REFRESH_INTERVAL),
            )
            .await
            {
                break;
            }
            if weak
                .update(cx, |this, cx| {
                    let active = !this.worker_notices.snapshot(&this.project.path).is_empty();
                    if refresh.update(active) {
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
        }
    });

    BootstrapTasks {
        runtime_events,
        workgraph_updates,
        worker_updates,
        worker_notices,
    }
}

async fn wait_for_notice_refresh(
    updates: &async_channel::Receiver<()>,
    timer: impl Future<Output = ()>,
) -> bool {
    let update = updates.recv();
    futures::pin_mut!(update, timer);
    match select(update, timer).await {
        Either::Left((result, _)) => result.is_ok(),
        Either::Right(((), _)) => true,
    }
}

#[cfg(test)]
#[path = "tasks_tests.rs"]
mod tests;

pub(super) fn start_performance_monitor(
    window: &Window,
    cx: &mut Context<FarcasterApp>,
) -> PerformanceState {
    let debug = std::env::var("DEBUG").ok().as_deref() == Some("true");
    let monitor = Some(
        crate::app::infrastructure::performance::PerformanceMonitor::new(
            window.window_handle().window_id(),
            debug,
        ),
    );
    let task = Some(cx.spawn(async move |weak, cx| {
        loop {
            cx.background_executor()
                .timer(crate::app::infrastructure::performance::sample_interval())
                .await;
            if weak
                .update(cx, |this, cx| {
                    if this.lifecycle.performance_monitor.as_mut().is_some_and(
                        crate::app::infrastructure::performance::PerformanceMonitor::sample_if_due,
                    ) {
                        this.notify_run_panel(cx);
                    }
                })
                .is_err()
            {
                break;
            }
        }
    }));

    PerformanceState { monitor, task }
}
