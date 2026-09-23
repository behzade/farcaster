use super::*;

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
        while notice_updates.recv().await.is_ok() {
            if weak.update(cx, |_, cx| cx.notify()).is_err() {
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
