use super::*;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::Instant,
};

#[test]
fn concurrent_sends_share_one_profile_choice_and_its_result() {
    let model = agents::WorkerExecution {
        harness: agents::Backend::Pi,
        provider: "openai".into(),
        model: "chosen".into(),
        effort: None,
        service_tier: None,
    };
    check_burst("selected", Ok(model));
    check_burst("cancelled", Err("worker creation cancelled".into()));
}

fn check_burst(parent: &str, expected: Selection) {
    let key = (parent.to_owned(), "light".to_owned());
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let (started_tx, started_rx) = mpsc::channel();
    let choices = Arc::new(AtomicUsize::new(0));
    let mut threads = Vec::new();
    for _ in 0..20 {
        let key = key.clone();
        let choices = choices.clone();
        let release_rx = release_rx.clone();
        let started_tx = started_tx.clone();
        let expected = expected.clone();
        let handle = thread::spawn(move || {
            select_once(key, || {
                choices.fetch_add(1, Ordering::SeqCst);
                started_tx.send(()).unwrap();
                release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
                expected
            })
        });
        threads.push(handle);
        // The first caller must be inside its selection before the rest join.
        if threads.len() == 1 {
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while flights()
        .lock()
        .unwrap()
        .get(&key)
        .is_none_or(|flight| Arc::strong_count(flight) < 21)
    {
        assert!(
            Instant::now() < deadline,
            "all sends should join one selection"
        );
        thread::yield_now();
    }
    release_tx.send(()).unwrap();
    for handle in threads {
        assert_eq!(handle.join().unwrap(), expected);
    }
    assert_eq!(choices.load(Ordering::SeqCst), 1);
    assert!(!flights().lock().unwrap().contains_key(&key));
}
