use super::*;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

struct WakeProbe {
    thread: Option<thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    received: mpsc::Receiver<()>,
}

impl WakeProbe {
    fn new() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let (sent, received) = mpsc::channel();
        let thread = thread::spawn(move || {
            thread::park();
            if !stopped.load(Ordering::Acquire) {
                let _ = sent.send(());
            }
        });
        Self {
            thread: Some(thread),
            stop,
            received,
        }
    }

    fn wake(&self) -> thread::Thread {
        self.thread.as_ref().unwrap().thread().clone()
    }
}

impl Drop for WakeProbe {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.wake().unpark();
        self.thread.take().unwrap().join().unwrap();
    }
}

#[gpui::test]
fn shared_scheduler_wakes_at_deadline_and_cancelled_tasks_do_not_wake(
    cx: &mut gpui::TestAppContext,
) {
    let executor = cx.background_executor.clone();
    let host = host(executor.clone());
    let due = WakeProbe::new();
    let cancelled = WakeProbe::new();
    let _task = host.schedule_wake(Instant::now() + Duration::from_secs(1), due.wake());
    let cancelled_task =
        host.schedule_wake(Instant::now() + Duration::from_secs(1), cancelled.wake());
    executor.run_until_parked();
    drop(cancelled_task);
    executor.advance_clock(Duration::from_millis(500));
    executor.run_until_parked();
    assert!(due.received.try_recv().is_err());
    executor.advance_clock(Duration::from_secs(1));
    executor.run_until_parked();
    due.received
        .recv_timeout(Duration::from_secs(2))
        .expect("scheduled wake");
    assert!(matches!(
        cancelled.received.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
}
