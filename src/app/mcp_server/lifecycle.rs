use std::{
    future::IntoFuture as _, net::TcpListener, path::PathBuf, sync::Mutex, thread::JoinHandle,
};

use rmcp::transport::streamable_http_server::{
    StreamableHttpService, session::local::LocalSessionManager,
};
use tokio::sync::oneshot;

use super::{BIND_ADDRESS, FarcasterMcp, MCP_PATH, notices, server_config};

static SERVER: Mutex<Option<ServerState>> = Mutex::new(None);

#[cfg(test)]
static TEST_WORKER_POOL: Mutex<Option<crate::agents::WorkerPool>> = Mutex::new(None);

#[cfg(test)]
static TEST_WORKER_POOL_LOCK: Mutex<()> = Mutex::new(());

pub(crate) struct McpServer;

struct ServerState {
    service: FarcasterMcp,
    running: Option<RunningServer>,
}

struct RunningServer {
    shutdown: oneshot::Sender<()>,
    thread: JoinHandle<()>,
}

pub(crate) fn start(
    database: PathBuf,
    workers: crate::agents::WorkerPool,
    updates: async_channel::Sender<()>,
) -> Result<McpServer, String> {
    let mut current = SERVER
        .lock()
        .map_err(|_| "MCP server state is unavailable")?;
    if current.is_some() {
        return Err("MCP server is already initialized".into());
    }
    let server = ServerState::new(
        FarcasterMcp::new(
            database.clone(),
            workers,
            updates,
            notices::NoticeBoard::default(),
        ),
        crate::builtin_mcp::enabled(),
        BIND_ADDRESS,
    )?;
    crate::agents::CallerRegistry::shared().set_family_sink(Some(std::sync::Arc::new(
        move |link| {
            crate::app::persistence::StateStore::open_at(&database)?.save_worker_family(link)
        },
    )));
    *current = Some(server);
    Ok(McpServer)
}

pub(crate) fn set_enabled(enabled: bool) -> Result<(), String> {
    let store = crate::app::persistence::StateStore::open()?;
    let mut current = SERVER
        .lock()
        .map_err(|_| "MCP server state is unavailable")?;
    let server = current.as_mut().ok_or("MCP server is not initialized")?;
    let was_running = server.running.is_some();
    if enabled {
        server.enable(BIND_ADDRESS)?;
    }
    if let Err(error) = store.save_builtin_mcp_enabled(enabled) {
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

pub(crate) fn set_worker_app_proxy(proxy: Option<String>) -> Result<(), String> {
    #[cfg(test)]
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

pub(crate) fn stop_session_family_workers(
    project: &std::path::Path,
    sessions: &[(String, PathBuf)],
) -> Result<usize, String> {
    #[cfg(test)]
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

pub(crate) fn finish_session_family_worker_stop(
    project: &std::path::Path,
    sessions: &[(String, PathBuf)],
) -> Result<(), String> {
    #[cfg(test)]
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

#[cfg(test)]
pub(crate) fn with_test_worker_pool<T>(
    workers: crate::agents::WorkerPool,
    test: impl FnOnce() -> T,
) -> T {
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

pub(crate) fn worker_snapshots() -> Result<Vec<crate::agents::WorkerSnapshot>, String> {
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
        }
    }
}

impl ServerState {
    fn new(service: FarcasterMcp, enabled: bool, address: &str) -> Result<Self, String> {
        let mut server = Self {
            service,
            running: None,
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
        let listener = TcpListener::bind(address)
            .map_err(|error| format!("bind http://{address}{MCP_PATH}: {error}"))?;
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
                zlog::error!("MCP server stopped: {error}");
            }
        }
        _ = stopped => {}
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
