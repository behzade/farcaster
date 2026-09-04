use std::{
    future::IntoFuture as _, net::TcpListener, path::PathBuf, sync::Mutex, thread::JoinHandle,
};

use rmcp::transport::streamable_http_server::{
    StreamableHttpService, session::local::LocalSessionManager,
};
use tokio::sync::oneshot;

use super::{BIND_ADDRESS, FarcasterMcp, MCP_PATH, notices, server_config};

static SERVER: Mutex<Option<ServerState>> = Mutex::new(None);

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
        FarcasterMcp::new(database, workers, updates, notices::NoticeBoard::default()),
        crate::builtin_mcp::enabled(),
        BIND_ADDRESS,
    )?;
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

impl Drop for McpServer {
    fn drop(&mut self) {
        if let Ok(mut current) = SERVER.lock() {
            drop(current.take());
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
mod tests {
    use super::*;

    #[test]
    fn disabled_server_leaves_the_port_free_and_can_be_reenabled() {
        let project = tempfile::tempdir().expect("project");
        let (factories, backend) =
            crate::agents::worker_factories(crate::agents::AgentLaunchConfig::default());
        let workers =
            crate::agents::WorkerPool::new(factories, backend, project.path().to_owned(), 1)
                .expect("workers");
        let (updates, _) = async_channel::bounded(1);
        let service = FarcasterMcp::new(
            project.path().join("state.db"),
            workers,
            updates,
            notices::NoticeBoard::default(),
        );
        let occupied = TcpListener::bind("127.0.0.1:0").expect("reserve port");
        let address = occupied.local_addr().expect("address").to_string();
        assert!(ServerState::new(service.clone(), true, &address).is_err());
        let mut server = ServerState::new(service, false, &address)
            .expect("disabled startup ignores occupied port");
        server.disable();
        assert!(server.enable(&address).is_err());
        assert!(server.running.is_none());
        drop(occupied);
        for _ in 0..2 {
            server.enable(&address).expect("enable server");
            assert!(
                TcpListener::bind(&address).is_err(),
                "enabled server owns the port"
            );
            server.disable();
            let probe = TcpListener::bind(&address).expect("disabled server releases port");
            drop(probe);
        }
    }
}
