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

#[test]
fn profile_limits_are_independent_and_include_pending_slots() {
    let concurrency = WorkerConcurrency::new(1);
    concurrency
        .set_profile_limits([("smartest".into(), 1), ("light".into(), 2)])
        .unwrap();
    let expensive = concurrency.reserve_profile("smartest").unwrap();
    assert!(concurrency.reserve_profile("smartest").is_err());
    let first = concurrency.reserve_profile("light").unwrap();
    let second = concurrency.reserve_profile("light").unwrap();
    assert!(concurrency.reserve_profile("light").is_err());
    first.release();
    assert!(concurrency.reserve_profile("light").is_ok());
    expensive.release();
    assert!(concurrency.reserve_profile("smartest").is_ok());
    drop(second);
}
