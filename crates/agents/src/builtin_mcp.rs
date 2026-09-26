use std::sync::atomic::{AtomicBool, Ordering};
use std::{net::SocketAddr, sync::OnceLock};

#[cfg(any(test, feature = "test-support"))]
use std::sync::{Mutex, MutexGuard};

const DEFAULT_URL: &str = "http://127.0.0.1:8765/mcp";
static ENDPOINT: OnceLock<String> = OnceLock::new();

/// Configure the host endpoint once, before any agents launch.
pub fn set_endpoint(address: SocketAddr) -> Result<(), String> {
    ENDPOINT
        .set(format!("http://{address}/mcp"))
        .map_err(|_| "MCP endpoint is already configured".to_owned())
}

pub fn url() -> String {
    #[cfg(test)]
    if let Some(url) = TEST_URL.with(|url| url.borrow().clone()) {
        return url;
    }
    ENDPOINT
        .get()
        .map(String::as_str)
        .unwrap_or(DEFAULT_URL)
        .to_owned()
}

#[cfg(test)]
thread_local! {
    static TEST_URL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_url_for_test<T>(url: &str, test: impl FnOnce() -> T) -> T {
    struct Restore(Option<String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_URL.with(|url| *url.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(TEST_URL.with(|value| value.replace(Some(url.to_owned()))));
    test()
}

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
/// Hold across operations and assertions that require a stable MCP setting.
/// Do not acquire this lock while holding another MCP test guard.
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

#[cfg(test)]
#[path = "builtin_mcp_tests.rs"]
mod tests;
