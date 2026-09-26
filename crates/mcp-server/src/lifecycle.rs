use std::{
    future::IntoFuture as _,
    net::TcpListener,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread::JoinHandle,
};

use rmcp::transport::streamable_http_server::{
    StreamableHttpService, session::local::LocalSessionManager,
};
use tokio::sync::oneshot;

#[cfg(test)]
use super::notices;
use super::{BIND_ADDRESS, FarcasterMcp, MCP_PATH, server_config};

static SERVER: Mutex<Option<ServerState>> = Mutex::new(None);
// Keep the reservation even while MCP is disabled or startup fails.
static INSTALLED_LISTENER: Mutex<Option<TcpListener>> = Mutex::new(None);

/// Install a reserved endpoint before starting the server or launching agents.
/// The socket stays reserved for this process, including while MCP is disabled.
pub fn install_listener(listener: TcpListener) -> Result<(), String> {
    let current = SERVER
        .lock()
        .map_err(|_| "MCP server state is unavailable")?;
    if current.is_some() {
        return Err("MCP server is already initialized".into());
    }
    let mut installed = INSTALLED_LISTENER
        .lock()
        .map_err(|_| "MCP listener is unavailable")?;
    if installed.is_some() {
        return Err("MCP listener is already installed".into());
    }
    let address = listener
        .local_addr()
        .map_err(|error| format!("read MCP address: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("configure MCP listener: {error}"))?;
    crate::builtin_mcp::set_endpoint(address)?;
    *installed = Some(listener);
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
static TEST_WORKER_POOL: Mutex<Option<crate::agents::WorkerPool>> = Mutex::new(None);

#[cfg(any(test, feature = "test-support"))]
static TEST_WORKER_POOL_LOCK: Mutex<()> = Mutex::new(());

pub struct McpServer;

struct ServerState {
    service: FarcasterMcp,
    running: Option<RunningServer>,
    reserved: Option<TcpListener>,
}

struct RunningServer {
    shutdown: oneshot::Sender<()>,
    thread: JoinHandle<()>,
}

pub fn start(
    store: Arc<Mutex<crate::storage::StateStore>>,
    workers: crate::agents::WorkerPool,
    updates: async_channel::Sender<()>,
    notices: crate::notice_board::NoticeBoard,
) -> Result<McpServer, String> {
    let mut current = SERVER
        .lock()
        .map_err(|_| "MCP server state is unavailable")?;
    if current.is_some() {
        return Err("MCP server is already initialized".into());
    }
    super::with_store(&store, |store| {
        store.with_connection(|connection| {
            workgraph::SqliteAdapter::initialize_connection(connection)
                .map_err(|error| error.to_string())
        })
    })?;
    let reserved = INSTALLED_LISTENER
        .lock()
        .map_err(|_| "MCP listener is unavailable")?
        .as_ref()
        .map(TcpListener::try_clone)
        .transpose()
        .map_err(|error| format!("clone MCP listener: {error}"))?;
    let server = ServerState::with_listener(
        FarcasterMcp::new(store.clone(), workers, updates, notices),
        crate::builtin_mcp::enabled(),
        BIND_ADDRESS,
        reserved,
    )?;
    let family_store = store.clone();
    crate::agents::CallerRegistry::shared().set_family_sink(Some(Arc::new(move |link| {
        super::with_store(&family_store, |store| store.save_worker_family(link))
    })));
    let binding_store = store.clone();
    let execution_store = store;
    crate::agents::CallerRegistry::shared().set_execution_sinks(
        Some(Arc::new(move |caller| {
            super::with_store(&binding_store, |store| {
                store.register_caller_session(caller)
            })
        })),
        Some(Arc::new(move |caller, execution| {
            super::with_store(&execution_store, |store| {
                store.register_execution_for_caller(caller, execution)
            })
        })),
    );
    *current = Some(server);
    Ok(McpServer)
}

pub fn set_enabled(
    enabled: bool,
    save_setting: impl FnOnce(bool) -> Result<(), String>,
) -> Result<(), String> {
    let mut current = SERVER
        .lock()
        .map_err(|_| "MCP server state is unavailable")?;
    let server = current.as_mut().ok_or("MCP server is not initialized")?;
    let was_running = server.running.is_some();
    if enabled {
        server.enable(BIND_ADDRESS)?;
    }
    if let Err(error) = save_setting(enabled) {
        if !was_running {
            server.disable();
        }
        return Err(error);
    }
    if !enabled {
        server.disable();
    }
    crate::builtin_mcp::set_enabled(enabled);
    Ok(())
}

pub fn set_worker_app_proxy(proxy: Option<String>) -> Result<(), String> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(workers) = TEST_WORKER_POOL
        .lock()
        .map_err(|_| "test worker pool is unavailable")?
        .as_ref()
        .cloned()
    {
        return workers.set_app_proxy(proxy);
    }
    let current = SERVER
        .lock()
        .map_err(|_| "MCP server state is unavailable")?;
    let Some(server) = current.as_ref() else {
        return Ok(());
    };
    server.service.workers.set_app_proxy(proxy)
}

pub fn stop_session_family_workers(
    project: &std::path::Path,
    sessions: &[(crate::agents::Backend, PathBuf)],
) -> Result<usize, String> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(workers) = TEST_WORKER_POOL
        .lock()
        .map_err(|_| "test worker pool is unavailable")?
        .as_ref()
        .cloned()
    {
        return workers.stop_session_family(project, sessions);
    }
    let current = SERVER
        .lock()
        .map_err(|_| "MCP server state is unavailable")?;
    let Some(server) = current.as_ref() else {
        return Ok(0);
    };
    server
        .service
        .workers
        .stop_session_family(project, sessions)
}

pub fn finish_session_family_worker_stop(
    project: &std::path::Path,
    sessions: &[(crate::agents::Backend, PathBuf)],
) -> Result<(), String> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(workers) = TEST_WORKER_POOL
        .lock()
        .map_err(|_| "test worker pool is unavailable")?
        .as_ref()
        .cloned()
    {
        return workers.finish_session_family_stop(project, sessions);
    }
    let current = SERVER
        .lock()
        .map_err(|_| "MCP server state is unavailable")?;
    let Some(server) = current.as_ref() else {
        return Ok(());
    };
    server
        .service
        .workers
        .finish_session_family_stop(project, sessions)
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_test_worker_pool<T>(workers: crate::agents::WorkerPool, test: impl FnOnce() -> T) -> T {
    let _serial = TEST_WORKER_POOL_LOCK.lock().expect("test worker pool lock");
    *TEST_WORKER_POOL.lock().expect("test worker pool") = Some(workers);
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            *TEST_WORKER_POOL.lock().expect("test worker pool") = None;
        }
    }
    let _clear = Clear;
    test()
}

pub fn worker_snapshots() -> Result<Vec<crate::agents::WorkerSnapshot>, String> {
    let current = SERVER
        .lock()
        .map_err(|_| "MCP server state is unavailable")?;
    let Some(server) = current.as_ref() else {
        return Ok(Vec::new());
    };
    server.service.workers.snapshots()
}

impl Drop for McpServer {
    fn drop(&mut self) {
        if let Ok(mut current) = SERVER.lock() {
            drop(current.take());
            crate::agents::CallerRegistry::shared().set_family_sink(None);
            crate::agents::CallerRegistry::shared().set_execution_sinks(None, None);
        }
    }
}

impl ServerState {
    #[cfg(test)]
    fn new(service: FarcasterMcp, enabled: bool, address: &str) -> Result<Self, String> {
        Self::with_listener(service, enabled, address, None)
    }

    fn with_listener(
        service: FarcasterMcp,
        enabled: bool,
        address: &str,
        reserved: Option<TcpListener>,
    ) -> Result<Self, String> {
        let mut server = Self {
            service,
            running: None,
            reserved,
        };
        if enabled {
            server.enable(address)?;
        }
        Ok(server)
    }

    fn enable(&mut self, address: &str) -> Result<(), String> {
        if self.running.is_some() {
            return Ok(());
        }
        let listener = match &self.reserved {
            Some(listener) => listener
                .try_clone()
                .map_err(|error| format!("clone MCP listener: {error}"))?,
            None => TcpListener::bind(address)
                .map_err(|error| format!("bind http://{address}{MCP_PATH}: {error}"))?,
        };
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("configure MCP listener: {error}"))?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("create MCP runtime: {error}"))?;
        let listener = {
            let _entered = runtime.enter();
            tokio::net::TcpListener::from_std(listener)
                .map_err(|error| format!("open MCP listener: {error}"))?
        };
        let handler = self.service.clone();
        let (shutdown, stopped) = oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("farcaster-mcp".into())
            .spawn(move || {
                runtime.block_on(serve(listener, handler, stopped));
                runtime.shutdown_background();
            })
            .map_err(|error| format!("spawn MCP server: {error}"))?;
        self.running = Some(RunningServer { shutdown, thread });
        Ok(())
    }

    fn disable(&mut self) {
        if let Some(running) = self.running.take() {
            let _ = running.shutdown.send(());
            let _ = running.thread.join();
        }
    }
}

impl Drop for ServerState {
    fn drop(&mut self) {
        self.disable();
    }
}

async fn serve(
    listener: tokio::net::TcpListener,
    handler: FarcasterMcp,
    stopped: oneshot::Receiver<()>,
) {
    let service = StreamableHttpService::new(
        move || Ok(handler.clone()),
        LocalSessionManager::default().into(),
        server_config(),
    );
    let router = axum::Router::new().nest_service(MCP_PATH, service);
    tokio::select! {
        result = axum::serve(listener, router).into_future() => {
            if let Err(error) = result {
                log::error!("MCP server stopped: {error}");
            }
        }
        _ = stopped => {}
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "pi_handshake_tests.rs"]
mod pi_handshake_tests;
