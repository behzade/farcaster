use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(any(test, feature = "test-support"))]
use std::sync::{Mutex, MutexGuard};

static ENABLED: AtomicBool = AtomicBool::new(true);

#[cfg(any(test, feature = "test-support"))]
static EXCLUSIVE: Mutex<()> = Mutex::new(());

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

#[cfg(any(test, feature = "test-support"))]
pub fn exclusive_for_test() -> MutexGuard<'static, ()> {
    EXCLUSIVE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(any(test, feature = "test-support"))]
pub struct McpDisabledForTest {
    _exclusive: MutexGuard<'static, ()>,
    previous: bool,
}

#[cfg(any(test, feature = "test-support"))]
impl McpDisabledForTest {
    pub fn new() -> Self {
        let exclusive = exclusive_for_test();
        let previous = enabled();
        set_enabled(false);
        Self {
            _exclusive: exclusive,
            previous,
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Default for McpDisabledForTest {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for McpDisabledForTest {
    fn drop(&mut self) {
        set_enabled(self.previous);
    }
}
