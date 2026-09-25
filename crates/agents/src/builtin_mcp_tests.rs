use super::*;
use std::{
    sync::{TryLockError, mpsc},
    thread,
    time::Duration,
};

#[test]
fn disabled_guard_restores_enabled_state() {
    let guard = McpDisabledForTest::new();
    assert!(!enabled());
    drop(guard);

    let _exclusive = exclusive_for_test();
    assert!(enabled());
}

#[test]
fn disabled_guard_restores_disabled_state() {
    let _outer = McpDisabledForTest::new();
    // Keep the global lock through the assertion and final restoration. Give
    // the inner guard its own lock to test Drop without nesting global locks.
    static INNER: Mutex<()> = Mutex::new(());
    let guard = McpDisabledForTest {
        _exclusive: INNER.lock().expect("inner guard lock"),
        previous: enabled(),
    };
    set_enabled(true);
    drop(guard);
    assert!(!enabled());
}

#[test]
fn disabled_guard_restores_state_after_panic() {
    let result = std::panic::catch_unwind(|| {
        let _guard = McpDisabledForTest::new();
        assert!(!enabled());
        panic!("exercise MCP guard cleanup");
    });
    assert!(result.is_err());

    let _exclusive = exclusive_for_test();
    assert!(enabled());
}

#[test]
fn disabled_guards_serialize_across_threads() {
    let first = McpDisabledForTest::new();
    let (attempted_tx, attempted_rx) = mpsc::channel();
    let (entered_tx, entered_rx) = mpsc::channel();
    let second = thread::spawn(move || {
        let locked = matches!(EXCLUSIVE.try_lock(), Err(TryLockError::WouldBlock));
        attempted_tx.send(locked).expect("report lock state");
        let _guard = McpDisabledForTest::new();
        entered_tx.send(enabled()).expect("report MCP state");
    });

    assert!(
        attempted_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("second thread attempted lock")
    );
    assert!(matches!(
        entered_rx.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    assert!(!enabled());
    drop(first);

    assert!(
        !entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("second guard acquired released lock")
    );
    second.join().expect("second guard completed");
    let _exclusive = exclusive_for_test();
    assert!(enabled());
}
