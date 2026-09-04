use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub(crate) struct WorkerConcurrency {
    maximum: usize,
    active: Arc<Mutex<usize>>,
}

impl WorkerConcurrency {
    pub(crate) fn new(maximum: usize) -> Self {
        Self {
            maximum,
            active: Arc::new(Mutex::new(0)),
        }
    }

    pub(crate) fn reserve(&self) -> Result<WorkerSlot, String> {
        let slot = WorkerSlot(Arc::new(SlotState {
            concurrency: self.clone(),
            active: Mutex::new(false),
        }));
        if !slot.try_activate() {
            return Err(format!(
                "worker pool is full (maximum {} active workers)",
                self.maximum
            ));
        }
        Ok(slot)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WorkerSlot(Arc<SlotState>);

#[derive(Debug)]
struct SlotState {
    concurrency: WorkerConcurrency,
    active: Mutex<bool>,
}

impl WorkerSlot {
    pub(crate) fn try_activate(&self) -> bool {
        let Ok(mut active) = self.0.active.lock() else {
            return false;
        };
        if *active {
            return true;
        }
        let Ok(mut count) = self.0.concurrency.active.lock() else {
            return false;
        };
        if *count >= self.0.concurrency.maximum {
            return false;
        }
        *count += 1;
        *active = true;
        true
    }

    pub(crate) fn release(&self) {
        if let Ok(mut active) = self.0.active.lock()
            && *active
            && let Ok(mut count) = self.0.concurrency.active.lock()
        {
            *count -= 1;
            *active = false;
        }
    }
}

impl Drop for SlotState {
    fn drop(&mut self) {
        if self.active.get_mut().is_ok_and(|active| *active)
            && let Ok(mut count) = self.concurrency.active.lock()
        {
            *count -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_reactivation_admits_only_one_worker() {
        let concurrency = WorkerConcurrency::new(1);
        let slots = (0..8)
            .map(|_| {
                let slot = concurrency.reserve().expect("slot");
                slot.release();
                slot
            })
            .collect::<Vec<_>>();
        let barrier = Arc::new(std::sync::Barrier::new(slots.len()));
        let threads = slots
            .iter()
            .cloned()
            .map(|slot| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    slot.try_activate()
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            threads
                .into_iter()
                .map(|thread| usize::from(thread.join().expect("worker")))
                .sum::<usize>(),
            1
        );
    }
}
