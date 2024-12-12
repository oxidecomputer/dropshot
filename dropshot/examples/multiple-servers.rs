// Copyright 2023 Oxide Computer Company

//! Example use of Dropshot with multiple servers sharing context.
//!
//! This example initially starts two servers named "A" and "B" listening on
//! `127.0.0.1:12345` and `127.0.0.1:12346`. On either address, a client can
//! query the list of currently-running servers:
//!
//! ```text
//! sh$ curl -X GET http://127.0.0.1:12345/servers | jq
//! [
//!   {
//!     "name": "B",
//!     "bind_addr": "127.0.0.1:12346"
//!   },
//!   {
//!     "name": "A",
//!     "bind_addr": "127.0.0.1:12345"
//!   }
//! ]
//! ```
//!
//! start a new server, as long as the name and bind address aren't already in
//! use:
//!
//! ```text
//! sh$ curl -X POST -H 'Content-Type: application/json' http://127.0.0.1:12345/servers/C -d '"127.0.0.1:12347"' | jq
//! {
//!   "name": "C",
//!   "bind_addr": "127.0.0.1:12347"
//! }
//! sh$ % curl -X GET http://127.0.0.1:12345/servers | jq
//! [
//!   {
//!     "name": "B",
//!     "bind_addr": "127.0.0.1:12346"
//!   },
//!   {
//!     "name": "C",
//!     "bind_addr": "127.0.0.1:12347"
//!   },
//!   {
//!     "name": "A",
//!     "bind_addr": "127.0.0.1:12345"
//!   }
//! ]
//! ```
//!
//! or stop a running server by name:
//!
//! ```text
//! sh$ % curl -X DELETE http://127.0.0.1:12347/servers/B
//! sh$ curl -X GET http://127.0.0.1:12345/servers | jq
//! [
//!   {
//!     "name": "C",
//!     "bind_addr": "127.0.0.1:12347"
//!   },
//!   {
//!     "name": "A",
//!     "bind_addr": "127.0.0.1:12345"
//!   }
//! ]
//! ```
//!
//! The final example shows deleting server "B" via server C's address, and then
//! querying server "A" to show that "B" is gone. The logfiles of the running
//! process will also note the shutdown of server B.

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::HttpResponseCreated;
use dropshot::HttpResponseDeleted;
use dropshot::HttpResponseOk;
use dropshot::HttpServer;
use dropshot::Path;
use dropshot::RequestContext;
use dropshot::ServerBuilder;
use dropshot::TypedBody;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use futures::StreamExt;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use slog::info;
use slog::Logger;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::mem;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> Result<(), String> {
    // XXX Is there interest in adding the optional integration into some of the existing examples?
    #[cfg(feature = "otel-tracing")]
    let _otel_guard = equinix_otel_tools::init(env!("CARGO_CRATE_NAME"));

    // Initial set of servers to start. Once they're running, we may add or
    // remove servers based on client requests.
    let initial_servers = [("A", "127.0.0.1:12345"), ("B", "127.0.0.1:12346")];

    // We keep the set of running servers in a `FuturesUnordered` to allow us to
    // drive them all concurrently.
    let mut running_servers = FuturesUnordered::new();
    let (running_servers_tx, mut running_servers_rx) = mpsc::channel(8);

    let shared_context =
        Arc::new(SharedMultiServerContext::new(running_servers_tx));
    for (name, bind_address) in initial_servers {
        let bind_address = bind_address.parse().unwrap();
        shared_context.start_server(name, bind_address).await?;
    }

    // Explicitly drop `shared_context` so we can detect when all servers are
    // gone via `running_servers_rx` (which returns `None` when all transmitters
    // are dropped).
    mem::drop(shared_context);

    // Loop until all servers are shut down.
    //
    // If we receive a new server on `running_servers_rx`, we added it to
    // `running_servers`. If `running_servers_rx` indicates the channel is
    // closed, we know all server contexts have been dropped and no servers
    // remain.
    loop {
        tokio::select! {
            maybe_new_server = running_servers_rx.recv() => {
                match maybe_new_server {
                    Some(server) => running_servers.push(server),
                    None => return Ok(()),
                }
            }

            maybe_result = running_servers.next() => {
                if let Some(result) = maybe_result {
                    // Fail if any server failed to shut down.
                    result?;
                }
            }
        }
    }
}

type ServerShutdownFuture = BoxFuture<'static, Result<(), String>>;

/// Application-specific server context (state shared by handler functions)
struct MultiServerContext {
    // All running servers have the same underlying `shared` context.
    shared: Arc<SharedMultiServerContext>,

    // `name` and `log` are unique to each running server, allowing them to log
    // their own name when they shut down.
    name: String,
    log: Logger,
}

impl Drop for MultiServerContext {
    fn drop(&mut self) {
        info!(self.log, "shut down server {:?}", self.name);
    }
}

/// Context shared by all running servers.
struct SharedMultiServerContext {
    servers: Mutex<HashMap<String, HttpServer<MultiServerContext>>>,
    started_server_shutdown_handles: mpsc::Sender<ServerShutdownFuture>,
}

impl SharedMultiServerContext {
    fn new(
        started_server_shutdown_handles: mpsc::Sender<ServerShutdownFuture>,
    ) -> Self {
        Self { servers: Mutex::default(), started_server_shutdown_handles }
    }

    async fn start_server(
        self: &Arc<Self>,
        name: &str,
        bind_address: SocketAddr,
    ) -> Result<(), String> {
        let mut servers = self.servers.lock().await;
        let slot = match servers.entry(name.to_string()) {
            Entry::Occupied(_) => {
                return Err(format!("already running a server named {name:?}",))
            }
            Entry::Vacant(slot) => slot,
        };

        // See dropshot/examples/basic.rs for more details on most of these pieces.
        let config_logging =
            ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
        let log = config_logging
            .to_logger(format!("example-multiserver-{name}"))
            .map_err(|error| format!("failed to create logger: {}", error))?;

        // TODO: Could `ApiDescription` implement `Clone`, or could we pass an
        // `Arc<ApiDescription>` instead?
        let mut api = ApiDescription::new();
        api.register(api_get_servers).unwrap();
        api.register(api_start_server).unwrap();
        api.register(api_stop_server).unwrap();

        // Configure the server with the requested bind address.
        let config_dropshot =
            ConfigDropshot { bind_address, ..Default::default() };

        // Set up the server.
        let context = MultiServerContext {
            shared: Arc::clone(self),
            name: name.to_string(),
            log: log.clone(),
        };
        let server = ServerBuilder::new(api, context, log)
            .config(config_dropshot)
            .start()
            .map_err(|error| format!("failed to create server: {}", error))?;
        let shutdown_handle = server.wait_for_shutdown();

        slot.insert(server);

        // Explicitly drop `servers`, releasing the lock, before we potentially
        // block waiting to tell `main()` about this new server.
        mem::drop(servers);

        // Tell `main()` about this new running server, allowing it to wait for
        // its shutdown.
        //
        // Ignore the result of this `send()`: we can't unwrap due to missing
        // `Debug` impls, but if `main()` is gone we don't care.
        _ = self
            .started_server_shutdown_handles
            .send(shutdown_handle.boxed())
            .await;

        Ok(())
    }
}

// HTTP API interface

#[derive(Debug, Serialize, JsonSchema)]
struct ServerDescription {
    name: String,
    bind_addr: SocketAddr,
}

/// Fetch the current list of running servers.
#[endpoint {
    method = GET,
    path = "/servers",
}]
async fn api_get_servers(
    rqctx: RequestContext<MultiServerContext>,
) -> Result<HttpResponseOk<Vec<ServerDescription>>, HttpError> {
    let api_context = rqctx.context();

    let servers = api_context.shared.servers.lock().await;
    let servers = servers
        .iter()
        .map(|(name, server)| ServerDescription {
            name: name.clone(),
            bind_addr: server.local_addr(),
        })
        .collect();

    Ok(HttpResponseOk(servers))
}

#[derive(Deserialize, JsonSchema)]
struct PathName {
    name: String,
}

/// Start a new running server.
#[endpoint {
    method = POST,
    path = "/servers/{name}",
}]
async fn api_start_server(
    rqctx: RequestContext<MultiServerContext>,
    path: Path<PathName>,
    body: TypedBody<SocketAddr>,
) -> Result<HttpResponseCreated<ServerDescription>, HttpError> {
    let api_context = rqctx.context();
    let name = path.into_inner().name;
    let bind_addr = body.into_inner();

    api_context.shared.start_server(&name, bind_addr).await.map_err(|err| {
        // `for_bad_request` _might_ not be right (e.g., we might have some
        // spurious OS error starting the server), but it's likely right (the
        // most likely cause for failure is a duplicate name or already-in-use
        // bind address), and we're being lazy with errors in this example.
        HttpError::for_bad_request(
            Some("StartServerFailed".to_string()),
            format!("failed to start server {name:?}: {err}"),
        )
    })?;

    Ok(HttpResponseCreated(ServerDescription { name, bind_addr }))
}

/// Stop a running server by name.
#[endpoint {
    method = DELETE,
    path = "/servers/{name}",
}]
async fn api_stop_server(
    rqctx: RequestContext<MultiServerContext>,
    path: Path<PathName>,
) -> Result<HttpResponseDeleted, HttpError> {
    let api_context = rqctx.context();
    let name = path.into_inner().name;

    let mut servers = api_context.shared.servers.lock().await;
    let server = servers.remove(&name).ok_or_else(|| {
        HttpError::for_bad_request(
            Some("InvalidServerName".to_string()),
            format!("no server named {name:?}"),
        )
    })?;

    // We want to shut down `server`, but it might be the very server handling
    // this request! Move the shutdown onto a background task to allow this
    // request to complete; otherwise, we deadlock.
    //
    // We can safely discard the result of `close()` because `main()` also gets
    // it via the shutdown handle it received when `server` was created.
    tokio::spawn(server.close());

    Ok(HttpResponseDeleted())
}
