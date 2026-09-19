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
