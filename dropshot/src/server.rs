// Copyright 2023 Oxide Computer Company
//! Generic server-wide state and facilities

use super::api_description::ApiDescription;
use super::config::{ConfigDropshot, ConfigTls};
#[cfg(feature = "usdt-probes")]
use super::dtrace::probes;
use super::error::HttpError;
use super::handler::RequestContext;
use super::http_util::HEADER_REQUEST_ID;
use super::router::HttpRouter;
use super::ProbeRegistration;

use async_stream::stream;
use debug_ignore::DebugIgnore;
use futures::future::{
    BoxFuture, FusedFuture, FutureExt, Shared, TryFutureExt,
};
use futures::lock::Mutex;
use futures::stream::{Stream, StreamExt};
use hyper::server::{
    conn::{AddrIncoming, AddrStream},
    Server,
};
use hyper::service::Service;
use hyper::Body;
use hyper::Request;
use hyper::Response;
use rustls;
use scopeguard::{guard, ScopeGuard};
use std::convert::TryFrom;
use std::future::Future;
use std::mem;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::panic;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::ReadBuf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_rustls::{server::TlsStream, TlsAcceptor};
use uuid::Uuid;
use waitgroup::WaitGroup;

use crate::config::HandlerTaskMode;
use crate::RequestInfo;
use slog::Logger;

// TODO Replace this with something else?
type GenericError = Box<dyn std::error::Error + Send + Sync>;

/// Endpoint-accessible context associated with a server.
///
/// Automatically implemented for all Send + Sync types.
pub trait ServerContext: Send + Sync + 'static {}

impl<T: 'static> ServerContext for T where T: Send + Sync {}

/// Stores shared state used by the Dropshot server.
#[derive(Debug)]
pub struct DropshotState<C: ServerContext> {
    /// caller-specific state
    pub private: C,
    /// static server configuration parameters
    pub config: ServerConfig,
    /// request router
    pub router: HttpRouter<C>,
    /// server-wide log handle
    pub log: Logger,
    /// bound local address for the server.
    pub local_addr: SocketAddr,
    /// Identifies how to accept TLS connections
    pub(crate) tls_acceptor: Option<Arc<Mutex<TlsAcceptor>>>,
    /// Worker for the handler_waitgroup associated with this server, allowing
    /// graceful shutdown to wait for all handlers to complete.
    pub(crate) handler_waitgroup_worker: DebugIgnore<waitgroup::Worker>,
    pub(crate) version_policy: Arc<dyn VersionPolicy>,
}

impl<C: ServerContext> DropshotState<C> {
    pub fn using_tls(&self) -> bool {
        self.tls_acceptor.is_some()
    }
}

// XXX-dap should be a semer of some kind
// XXX-dap if we want to make it so that people don't have to deal with versions
// if they don't care about it, then we probably want a VersionPolicy whose
// returned default is some sentinel value that matches only ApiVersions::All.
pub type Version = String;

pub trait VersionPolicy: std::fmt::Debug + Send + Sync {
    fn version_allowed(&self, version: &Version) -> bool;
    fn default_version(&self) -> DefaultVersion;
}

pub enum DefaultVersion {
    Version(Version),
    ClientMustSpecify,
    DontCare,
}

#[derive(Debug)]
pub struct Unversioned;
impl VersionPolicy for Unversioned {
    fn version_allowed(&self, version: &Version) -> bool {
        // XXX-dap this shouldn't be callable actually
        true
    }

    fn default_version(&self) -> DefaultVersion {
        DefaultVersion::DontCare
    }
}

/// Stores static configuration associated with the server
/// TODO-cleanup merge with ConfigDropshot
#[derive(Debug)]
pub struct ServerConfig {
    /// maximum allowed size of a request body
    pub request_body_max_bytes: usize,
    /// maximum size of any page of results
    pub page_max_nitems: NonZeroU32,
    /// default size for a page of results
    pub page_default_nitems: NonZeroU32,
    /// Default behavior for HTTP handler functions with respect to clients
    /// disconnecting early.
    pub default_handler_task_mode: HandlerTaskMode,
    /// A list of header names to include as extra properties in the log
    /// messages emitted by the per-request logger.  Each header will, if
    /// present, be included in the output with a "hdr_"-prefixed property name
    /// in lower case that has all hyphens replaced with underscores; e.g.,
    /// "X-Forwarded-For" will be included as "hdr_x_forwarded_for".  No attempt
    /// is made to deal with headers that appear multiple times in a single
    /// request.
    pub log_headers: Vec<String>,
}

pub struct HttpServerStarter<C: ServerContext> {
    app_state: Arc<DropshotState<C>>,
    local_addr: SocketAddr,
    wrapped: WrappedHttpServerStarter<C>,
    handler_waitgroup: WaitGroup,
}

impl<C: ServerContext> HttpServerStarter<C> {
    pub fn new(
        config: &ConfigDropshot,
        api: ApiDescription<C>,
        private: C,
        log: &Logger,
    ) -> Result<HttpServerStarter<C>, GenericError> {
        Self::new_with_tls(
            config,
            api,
            private,
            log,
            None,
            Arc::new(Unversioned),
        )
    }

    pub fn new_with_tls(
        config: &ConfigDropshot,
        api: ApiDescription<C>,
        private: C,
        log: &Logger,
        tls: Option<ConfigTls>,
        version_policy: Arc<dyn VersionPolicy>,
    ) -> Result<HttpServerStarter<C>, GenericError> {
        let server_config = ServerConfig {
            // We start aggressively to ensure test coverage.
            request_body_max_bytes: config.request_body_max_bytes,
            page_max_nitems: NonZeroU32::new(10000).unwrap(),
            page_default_nitems: NonZeroU32::new(100).unwrap(),
            default_handler_task_mode: config.default_handler_task_mode,
            log_headers: config.log_headers.clone(),
        };

        let handler_waitgroup = WaitGroup::new();
        let starter = match &tls {
            Some(tls) => {
                let (starter, app_state, local_addr) =
                    InnerHttpsServerStarter::new(
                        config,
                        server_config,
                        api,
                        private,
                        log,
                        tls,
                        handler_waitgroup.worker(),
                        version_policy,
                    )?;
                HttpServerStarter {
                    app_state,
                    local_addr,
                    wrapped: WrappedHttpServerStarter::Https(starter),
                    handler_waitgroup,
                }
            }
            None => {
                let (starter, app_state, local_addr) =
                    InnerHttpServerStarter::new(
                        config,
                        server_config,
                        api,
                        private,
                        log,
                        handler_waitgroup.worker(),
                        version_policy,
                    )?;
                HttpServerStarter {
                    app_state,
                    local_addr,
                    wrapped: WrappedHttpServerStarter::Http(starter),
                    handler_waitgroup,
                }
            }
        };

        for (path, method, _) in &starter.app_state.router {
            debug!(starter.app_state.log, "registered endpoint";
                "method" => &method,
                "path" => &path
            );
        }

        Ok(starter)
    }

    pub fn start(self) -> HttpServer<C> {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let log_close = self.app_state.log.new(o!());
        let join_handle = match self.wrapped {
            WrappedHttpServerStarter::Http(http) => http.start(rx, log_close),
            WrappedHttpServerStarter::Https(https) => {
                https.start(rx, log_close)
            }
        }
        .map(|r| {
            r.map_err(|e| format!("waiting for server: {e}"))?
                .map_err(|e| format!("server stopped: {e}"))
        });
        info!(self.app_state.log, "listening");

        let handler_waitgroup = self.handler_waitgroup;
        let join_handle = async move {
            // After the server shuts down, we also want to wait for any
            // detached handler futures to complete.
            () = join_handle.await?;
            () = handler_waitgroup.wait().await;
            Ok(())
        };

        #[cfg(feature = "usdt-probes")]
        let probe_registration = match usdt::register_probes() {
            Ok(_) => {
                debug!(
                    self.app_state.log,
                    "successfully registered DTrace USDT probes"
                );
                ProbeRegistration::Succeeded
            }
            Err(e) => {
                let msg = e.to_string();
                error!(
                    self.app_state.log,
                    "failed to register DTrace USDT probes: {}", msg
                );
                ProbeRegistration::Failed(msg)
            }
        };
        #[cfg(not(feature = "usdt-probes"))]
        let probe_registration = {
            debug!(
                self.app_state.log,
                "DTrace USDT probes compiled out, not registering"
            );
            ProbeRegistration::Disabled
        };

        HttpServer {
            probe_registration,
            app_state: self.app_state,
            local_addr: self.local_addr,
            closer: CloseHandle { close_channel: Some(tx) },
            join_future: join_handle.boxed().shared(),
        }
    }
}

enum WrappedHttpServerStarter<C: ServerContext> {
    Http(InnerHttpServerStarter<C>),
    Https(InnerHttpsServerStarter<C>),
}

struct InnerHttpServerStarter<C: ServerContext>(
    Server<AddrIncoming, ServerConnectionHandler<C>>,
);

type InnerHttpServerStarterNewReturn<C> =
    (InnerHttpServerStarter<C>, Arc<DropshotState<C>>, SocketAddr);

impl<C: ServerContext> InnerHttpServerStarter<C> {
    /// Begins execution of the underlying Http server.
    fn start(
        self,
        close_signal: tokio::sync::oneshot::Receiver<()>,
        log_close: Logger,
    ) -> tokio::task::JoinHandle<Result<(), hyper::Error>> {
        let graceful = self.0.with_graceful_shutdown(async move {
            close_signal.await.expect(
                "dropshot server shutting down without invoking close()",
            );
            info!(log_close, "received request to begin graceful shutdown");
        });

        tokio::spawn(graceful)
    }

    /// Set up an HTTP server bound on the specified address that runs
    /// registered handlers.  You must invoke `start()` on the returned instance
    /// of `HttpServerStarter` (and await the result) to actually start the
    /// server.
    fn new(
        config: &ConfigDropshot,
        server_config: ServerConfig,
        api: ApiDescription<C>,
        private: C,
        log: &Logger,
        handler_waitgroup_worker: waitgroup::Worker,
        version_policy: Arc<dyn VersionPolicy>,
    ) -> Result<InnerHttpServerStarterNewReturn<C>, hyper::Error> {
        let incoming = AddrIncoming::bind(&config.bind_address)?;
        let local_addr = incoming.local_addr();

        let app_state = Arc::new(DropshotState {
            private,
            config: server_config,
            router: api.into_router(),
            log: log.new(o!("local_addr" => local_addr)),
            local_addr,
            tls_acceptor: None,
            handler_waitgroup_worker: DebugIgnore(handler_waitgroup_worker),
            version_policy,
        });

        let make_service = ServerConnectionHandler::new(app_state.clone());
        let builder = hyper::Server::builder(incoming);
        let server = builder.serve(make_service);
        Ok((InnerHttpServerStarter(server), app_state, local_addr))
    }
}

/// Wrapper for TlsStream<TcpStream> that also carries the remote SocketAddr
#[derive(Debug)]
struct TlsConn {
    stream: TlsStream<TcpStream>,
    remote_addr: SocketAddr,
}

impl TlsConn {
    fn new(stream: TlsStream<TcpStream>, remote_addr: SocketAddr) -> TlsConn {
        TlsConn { stream, remote_addr }
    }

    fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }
}

/// Forward AsyncRead to the underlying stream
impl tokio::io::AsyncRead for TlsConn {
    fn poll_read(
        mut self: Pin<&mut Self>,
        ctx: &mut core::task::Context,
        buf: &mut ReadBuf,
    ) -> Poll<std::io::Result<()>> {
        let pinned = Pin::new(&mut self.stream);
        pinned.poll_read(ctx, buf)
    }
}

/// Forward AsyncWrite to the underlying stream
impl tokio::io::AsyncWrite for TlsConn {
    fn poll_write(
        mut self: Pin<&mut Self>,
        ctx: &mut core::task::Context,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let pinned = Pin::new(&mut self.stream);
        pinned.poll_write(ctx, data)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        ctx: &mut core::task::Context,
    ) -> Poll<std::io::Result<()>> {
        let pinned = Pin::new(&mut self.stream);
        pinned.poll_flush(ctx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        ctx: &mut core::task::Context,
    ) -> Poll<std::io::Result<()>> {
        let pinned = Pin::new(&mut self.stream);
        pinned.poll_shutdown(ctx)
    }
}

/// This is our bridge between tokio-rustls and hyper. It implements
/// `hyper::server::accept::Accept` interface, producing TLS-over-TCP
/// connections.
///
/// Internally, it creates a stream that produces fully negotiated TLS
/// connections as they come in from a TCP listen socket.  This stream allows
/// for multiple TLS connections to be negotiated concurrently with new
/// connections being accepted.
struct HttpsAcceptor {
    stream: Box<dyn Stream<Item = std::io::Result<TlsConn>> + Send + Unpin>,
}

impl HttpsAcceptor {
    pub fn new(
        log: slog::Logger,
        tls_acceptor: Arc<Mutex<TlsAcceptor>>,
        tcp_listener: TcpListener,
    ) -> HttpsAcceptor {
        HttpsAcceptor {
            stream: Box::new(Box::pin(Self::new_stream(
                log,
                tls_acceptor,
                tcp_listener,
            ))),
        }
    }

    fn new_stream(
        log: slog::Logger,
        tls_acceptor: Arc<Mutex<TlsAcceptor>>,
        tcp_listener: TcpListener,
    ) -> impl Stream<Item = std::io::Result<TlsConn>> {
        stream! {
            let mut tls_negotiations = futures::stream::FuturesUnordered::new();
            loop {
                tokio::select! {
                    Some(negotiation) = tls_negotiations.next(), if
                            !tls_negotiations.is_empty() => {

                        match negotiation {
                            Ok(conn) => yield Ok(conn),
                            Err(e) => {
                                // If TLS negotiation fails, log the cause but
                                // don't forward it along. Yielding an error
                                // from here will terminate the server.
                                // These failures may be a fatal TLS alert
                                // message, or a client disconnection during
                                // negotiation, or other issues.
                                // TODO: We may want to export a counter for
                                // different error types, since this may contain
                                // useful things like "your certificate is
                                // invalid"
                                warn!(log, "tls accept err: {}", e);
                            },
                        }
                    },
                    accept_result = tcp_listener.accept() => {
                        let (socket, addr) = match accept_result {
                            Ok(v) => v,
                            Err(e) => {
                                match e.kind() {
                                    std::io::ErrorKind::ConnectionAborted => {
                                        continue;
                                    },
                                    // The other errors that can be returned
                                    // under POSIX are all programming errors or
                                    // resource exhaustion. For now, handle
                                    // these by no longer accepting new
                                    // connections.
                                    // TODO-robustness: Consider handling these
                                    // more gracefully.
                                    _ => {
                                        yield Err(e);
                                        break;
                                    }
                                }
                            }
                        };

                        let tls_negotiation = tls_acceptor
                            .lock()
                            .await
                            .accept(socket)
                            .map_ok(move |stream| TlsConn::new(stream, addr));
                        tls_negotiations.push(tls_negotiation);
                    },
                    else => break,
                }
            }
        }
    }
}

impl hyper::server::accept::Accept for HttpsAcceptor {
    type Conn = TlsConn;
    type Error = std::io::Error;

    fn poll_accept(
        mut self: Pin<&mut Self>,
        ctx: &mut core::task::Context,
    ) -> core::task::Poll<Option<Result<Self::Conn, Self::Error>>> {
        let pinned = Pin::new(&mut self.stream);
        pinned.poll_next(ctx)
    }
}

struct InnerHttpsServerStarter<C: ServerContext>(
    Server<HttpsAcceptor, ServerConnectionHandler<C>>,
);

/// Create a TLS configuration from the Dropshot config structure.
impl TryFrom<&ConfigTls> for rustls::ServerConfig {
    type Error = std::io::Error;

    fn try_from(config: &ConfigTls) -> std::io::Result<Self> {
        let (mut cert_reader, mut key_reader): (
            Box<dyn std::io::BufRead>,
            Box<dyn std::io::BufRead>,
        ) = match config {
            ConfigTls::Dynamic(raw) => {
                return Ok(raw.clone());
            }
            ConfigTls::AsBytes { certs, key } => (
                Box::new(std::io::BufReader::new(certs.as_slice())),
                Box::new(std::io::BufReader::new(key.as_slice())),
            ),
            ConfigTls::AsFile { cert_file, key_file } => {
                let certfile = Box::new(std::io::BufReader::new(
                    std::fs::File::open(cert_file).map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!(
                                "failed to open {}: {}",
                                cert_file.display(),
                                e
                            ),
                        )
                    })?,
                ));
                let keyfile = Box::new(std::io::BufReader::new(
                    std::fs::File::open(key_file).map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!(
                                "failed to open {}: {}",
                                key_file.display(),
                                e
                            ),
                        )
                    })?,
                ));
                (certfile, keyfile)
            }
        };

        let certs = rustls_pemfile::certs(&mut cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| {
                io_error(format!("failed to load certificate: {err}"))
            })?;
        let keys = rustls_pemfile::pkcs8_private_keys(&mut key_reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| {
                io_error(format!("failed to load private key: {err}"))
            })?;
        let mut keys_iter = keys.into_iter();
        let (Some(private_key), None) = (keys_iter.next(), keys_iter.next())
        else {
            return Err(io_error("expected a single private key".into()));
        };

        let mut cfg = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, private_key.into())
            .expect("bad certificate/key");
        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Ok(cfg)
    }
}

type InnerHttpsServerStarterNewReturn<C> =
    (InnerHttpsServerStarter<C>, Arc<DropshotState<C>>, SocketAddr);

impl<C: ServerContext> InnerHttpsServerStarter<C> {
    /// Begins execution of the underlying Http server.
    fn start(
        self,
        close_signal: tokio::sync::oneshot::Receiver<()>,
        log_close: Logger,
    ) -> tokio::task::JoinHandle<Result<(), hyper::Error>> {
        let graceful = self.0.with_graceful_shutdown(async move {
            close_signal.await.expect(
                "dropshot server shutting down without invoking close()",
            );
            info!(log_close, "received request to begin graceful shutdown");
        });

        tokio::spawn(graceful)
    }

    fn new(
        config: &ConfigDropshot,
        server_config: ServerConfig,
        api: ApiDescription<C>,
        private: C,
        log: &Logger,
        tls: &ConfigTls,
        handler_waitgroup_worker: waitgroup::Worker,
        version_policy: Arc<dyn VersionPolicy>,
    ) -> Result<InnerHttpsServerStarterNewReturn<C>, GenericError> {
        let acceptor = Arc::new(Mutex::new(TlsAcceptor::from(Arc::new(
            rustls::ServerConfig::try_from(tls)?,
        ))));

        let tcp = {
            let listener = std::net::TcpListener::bind(&config.bind_address)?;
            listener.set_nonblocking(true)?;
            // We use `from_std` instead of just calling `bind` here directly
            // to avoid invoking an async function, to match the interface
            // provided by `HttpServerStarter::new`.
            TcpListener::from_std(listener)?
        };

        let local_addr = tcp.local_addr()?;
        let logger = log.new(o!("local_addr" => local_addr));
        let https_acceptor =
            HttpsAcceptor::new(logger.clone(), acceptor.clone(), tcp);

        let app_state = Arc::new(DropshotState {
            private,
            config: server_config,
            router: api.into_router(),
            log: logger,
            local_addr,
            tls_acceptor: Some(acceptor),
            handler_waitgroup_worker: DebugIgnore(handler_waitgroup_worker),
            version_policy,
        });

        let make_service = ServerConnectionHandler::new(Arc::clone(&app_state));
        let server = Server::builder(https_acceptor).serve(make_service);

        Ok((InnerHttpsServerStarter(server), app_state, local_addr))
    }
}

impl<C: ServerContext> Service<&TlsConn> for ServerConnectionHandler<C> {
    type Response = ServerRequestHandler<C>;
    type Error = GenericError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        _ctx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, conn: &TlsConn) -> Self::Future {
        let server = Arc::clone(&self.server);
        let remote_addr = conn.remote_addr();
        Box::pin(http_connection_handle(server, remote_addr))
    }
}

type SharedBoxFuture<T> = Shared<Pin<Box<dyn Future<Output = T> + Send>>>;

/// Future returned by [`HttpServer::wait_for_shutdown()`].
pub struct ShutdownWaitFuture(SharedBoxFuture<Result<(), String>>);

impl Future for ShutdownWaitFuture {
    type Output = Result<(), String>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.get_mut().0).poll(cx)
    }
}

impl FusedFuture for ShutdownWaitFuture {
    fn is_terminated(&self) -> bool {
        self.0.is_terminated()
    }
}

/// A running Dropshot HTTP server.
///
/// The generic traits represent the following:
/// - C: Caller-supplied server context
pub struct HttpServer<C: ServerContext> {
    probe_registration: ProbeRegistration,
    app_state: Arc<DropshotState<C>>,
    local_addr: SocketAddr,
    closer: CloseHandle,
    join_future: SharedBoxFuture<Result<(), String>>,
}

// Handle used to trigger the shutdown of an [HttpServer].
struct CloseHandle {
    close_channel: Option<tokio::sync::oneshot::Sender<()>>,
}

impl<C: ServerContext> HttpServer<C> {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn app_private(&self) -> &C {
        &self.app_state.private
    }

    pub fn using_tls(&self) -> bool {
        self.app_state.using_tls()
    }

    /// Update TLS certificates for a running HTTPS server.
    pub async fn refresh_tls(&self, config: &ConfigTls) -> Result<(), String> {
        let acceptor = &self
            .app_state
            .tls_acceptor
            .as_ref()
            .ok_or_else(|| "Not configured for TLS".to_string())?;

        *acceptor.lock().await = TlsAcceptor::from(Arc::new(
            rustls::ServerConfig::try_from(config).unwrap(),
        ));
        Ok(())
    }

    /// Return the result of registering the server's DTrace USDT probes.
    ///
    /// See [`ProbeRegistration`] for details.
    pub fn probe_registration(&self) -> &ProbeRegistration {
        &self.probe_registration
    }

    /// Returns a future which completes when the server has shut down.
    ///
    /// This function does not cause the server to shut down. It just waits for
    /// the shutdown to happen.
    ///
    /// To trigger a shutdown, Call [HttpServer::close] (which also awaits
    /// shutdown).
    pub fn wait_for_shutdown(&self) -> ShutdownWaitFuture {
        ShutdownWaitFuture(self.join_future.clone())
    }

    /// Signals the currently running server to stop and waits for it to exit.
    pub async fn close(mut self) -> Result<(), String> {
        self.closer
            .close_channel
            .take()
            .expect("cannot close twice")
            .send(())
            .expect("failed to send close signal");

        // We _must_ explicitly drop our app state before awaiting join_future.
        // If we are running handlers in `Detached` mode, our `app_state` has a
        // `waitgroup::Worker` that they all clone, and `join_future` will await
        // all of them being dropped. That means we must drop our "primary"
        // clone of it, too!
        mem::drop(self.app_state);

        self.join_future.await
    }
}

// For graceful termination, the `close()` function is preferred, as it can
// report errors and wait for termination to complete.  However, we impl
// `Drop` to attempt to shut down the server to handle less clean shutdowns
// (e.g., from failing tests).
impl Drop for CloseHandle {
    fn drop(&mut self) {
        if let Some(c) = self.close_channel.take() {
            // The other side of this channel is owned by a separate tokio task
            // that's running the hyper server.  We do not expect that to be
            // cancelled.  But it can happen if the executor itself is shutting
            // down and that task happens to get cleaned up before this one.
            let _ = c.send(());
        }
    }
}

impl<C: ServerContext> Future for HttpServer<C> {
    type Output = Result<(), String>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let server = Pin::into_inner(self);
        let join_future = Pin::new(&mut server.join_future);
        join_future.poll(cx)
    }
}

impl<C: ServerContext> FusedFuture for HttpServer<C> {
    fn is_terminated(&self) -> bool {
        self.join_future.is_terminated()
    }
}

/// Initial entry point for handling a new connection to the HTTP server.
/// This is invoked by Hyper when a new connection is accepted.  This function
/// must return a Hyper Service object that will handle requests for this
/// connection.
async fn http_connection_handle<C: ServerContext>(
    server: Arc<DropshotState<C>>,
    remote_addr: SocketAddr,
) -> Result<ServerRequestHandler<C>, GenericError> {
    info!(server.log, "accepted connection"; "remote_addr" => %remote_addr);
    Ok(ServerRequestHandler::new(server, remote_addr))
}

/// Initial entry point for handling a new request to the HTTP server.  This is
/// invoked by Hyper when a new request is received.  This function returns a
/// Result that either represents a valid HTTP response or an error (which will
/// also get turned into an HTTP response).
async fn http_request_handle_wrap<C: ServerContext>(
    server: Arc<DropshotState<C>>,
    remote_addr: SocketAddr,
    request: Request<Body>,
) -> Result<Response<Body>, GenericError> {
    // This extra level of indirection makes error handling much more
    // straightforward, since the request handling code can simply return early
    // with an error and we'll treat it like an error from any of the endpoints
    // themselves.
    let start_time = std::time::Instant::now();
    let request_id = generate_request_id();

    let mut request_log = server.log.new(o!(
        "remote_addr" => remote_addr,
        "req_id" => request_id.clone(),
        "method" => request.method().as_str().to_string(),
        "uri" => format!("{}", request.uri()),
    ));
    // If we have been asked to include any headers from the request in the
    // log messages, do so here:
    for name in server.config.log_headers.iter() {
        let v = request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok().map(str::to_string));

        if let Some(v) = v {
            // This is unfortunate in at least two ways: first, we would like to
            // just construct _one_ key value map, but OwnedKV is opaque and can
            // only be constructed with the o!() macro, so the only way to layer
            // on a dynamic set of additional properties is by creating a chain
            // of child loggers to add each one; second, we would like to be
            // able to include all header values under a single map-valued
            // "header" property, but slog only allows us a single-level
            // property hierarchy.  Alas!
            //
            // We also replace the hyphens with underscores to make it easier to
            // refer to the generated properties in dynamic languages used for
            // filtering like rhai.
            let k = format!("hdr_{}", name.to_lowercase().replace('-', "_"));
            request_log = request_log.new(o!(k => v));
        }
    }

    trace!(request_log, "incoming request");
    #[cfg(feature = "usdt-probes")]
    probes::request__start!(|| {
        let uri = request.uri();
        crate::dtrace::RequestInfo {
            id: request_id.clone(),
            local_addr: server.local_addr,
            remote_addr,
            method: request.method().to_string(),
            path: uri.path().to_string(),
            query: uri.query().map(|x| x.to_string()),
        }
    });

    // Copy local address to report later during the finish probe, as the
    // server is passed by value to the request handler function.
    #[cfg(feature = "usdt-probes")]
    let local_addr = server.local_addr;

    // In the case the client disconnects early, the scopeguard allows us
    // to perform extra housekeeping before this task is dropped.
    let on_disconnect = guard((), |_| {
        let latency_us = start_time.elapsed().as_micros();

        warn!(request_log, "request handling cancelled (client disconnected)";
            "latency_us" => latency_us,
        );

        #[cfg(feature = "usdt-probes")]
        probes::request__done!(|| {
            crate::dtrace::ResponseInfo {
                id: request_id.clone(),
                local_addr,
                remote_addr,
                // 499 is a non-standard code popularized by nginx to mean "client disconnected".
                status_code: 499,
                message: String::from(
                    "client disconnected before response returned",
                ),
            }
        });
    });

    let maybe_response = http_request_handle(
        server,
        request,
        &request_id,
        request_log.new(o!()),
        remote_addr,
    )
    .await;

    // If `http_request_handle` completed, it means the request wasn't
    // cancelled and we can safely "defuse" the scopeguard.
    let _ = ScopeGuard::into_inner(on_disconnect);

    let latency_us = start_time.elapsed().as_micros();
    let response = match maybe_response {
        Err(error) => {
            let message_external = error.external_message.clone();
            let message_internal = error.internal_message.clone();
            let r = error.into_response(&request_id);

            #[cfg(feature = "usdt-probes")]
            probes::request__done!(|| {
                crate::dtrace::ResponseInfo {
                    id: request_id.clone(),
                    local_addr,
                    remote_addr,
                    status_code: r.status().as_u16(),
                    message: message_external.clone(),
                }
            });

            // TODO-debug: add request and response headers here
            info!(request_log, "request completed";
                "response_code" => r.status().as_str(),
                "latency_us" => latency_us,
                "error_message_internal" => message_internal,
                "error_message_external" => message_external,
            );

            r
        }

        Ok(response) => {
            // TODO-debug: add request and response headers here
            info!(request_log, "request completed";
                "response_code" => response.status().as_str(),
                "latency_us" => latency_us,
            );

            #[cfg(feature = "usdt-probes")]
            probes::request__done!(|| {
                crate::dtrace::ResponseInfo {
                    id: request_id.parse().unwrap(),
                    local_addr,
                    remote_addr,
                    status_code: response.status().as_u16(),
                    message: "".to_string(),
                }
            });

            response
        }
    };

    Ok(response)
}

async fn http_request_handle<C: ServerContext>(
    server: Arc<DropshotState<C>>,
    request: Request<Body>,
    request_id: &str,
    request_log: Logger,
    remote_addr: std::net::SocketAddr,
) -> Result<Response<Body>, HttpError> {
    // TODO-hardening: is it correct to (and do we correctly) read the entire
    // request body even if we decide it's too large and are going to send a 400
    // response?
    // TODO-hardening: add a request read timeout as well so that we don't allow
    // this to take forever.
    // TODO-correctness: Do we need to dump the body on errors?
    let method = request.method();
    let uri = request.uri();
    let version = String::from("dummy"); // XXX-dap
    if !server.version_policy.version_allowed(&version) {
        return Err(HttpError::for_not_found(
            None,
            String::from("version is disallowed by policy"),
        ));
    }
    let lookup_result =
        server.router.lookup_route(&method, uri.path().into(), &version)?;
    let rqctx = RequestContext {
        server: Arc::clone(&server),
        request: RequestInfo::new(&request, remote_addr),
        path_variables: lookup_result.variables,
        body_content_type: lookup_result.body_content_type,
        operation_id: lookup_result.operation_id,
        request_id: request_id.to_string(),
        log: request_log,
    };
    let handler = lookup_result.handler;

    let mut response = match server.config.default_handler_task_mode {
        HandlerTaskMode::CancelOnDisconnect => {
            // For CancelOnDisconnect, we run the request handler directly: if
            // the client disconnects, we will be cancelled, and therefore this
            // future will too.
            handler.handle_request(rqctx, request).await?
        }
        HandlerTaskMode::Detached => {
            // Spawn the handler so if we're cancelled, the handler still runs
            // to completion.
            let (tx, rx) = oneshot::channel();
            let request_log = rqctx.log.clone();
            let worker = server.handler_waitgroup_worker.clone();
            let handler_task = tokio::spawn(async move {
                let request_log = rqctx.log.clone();
                let result = handler.handle_request(rqctx, request).await;

                // If this send fails, our spawning task has been cancelled in
                // the `rx.await` below; log such a result.
                if let Err(result) = tx.send(result) {
                    match result {
                        Ok(r) => warn!(
                            request_log, "request completed after handler was already cancelled";
                            "response_code" => r.status().as_str(),
                        ),
                        Err(error) => {
                            warn!(request_log, "request completed after handler was already cancelled";
                                "response_code" => error.status_code.as_str(),
                                "error_message_internal" => error.external_message,
                                "error_message_external" => error.internal_message,
                            );
                        }
                    }
                }

                // Drop our waitgroup worker, allowing graceful shutdown to
                // complete (if it's waiting on us).
                mem::drop(worker);
            });

            // The only way we can fail to receive on `rx` is if `tx` is
            // dropped before a result is sent, which can only happen if
            // `handle_request` panics. We will propogate such a panic here,
            // just as we would have in `CancelOnDisconnect` mode above (where
            // we call the handler directly).
            match rx.await {
                Ok(result) => result?,
                Err(_) => {
                    error!(request_log, "handler panicked; propogating panic");

                    // To get the panic, we now need to await `handler_task`; we
                    // know it is complete _and_ it failed, because it has
                    // dropped `tx` without sending us a result, which is only
                    // possible if it panicked.
                    let task_err = handler_task.await.expect_err(
                        "task failed to send result but didn't panic",
                    );
                    panic::resume_unwind(task_err.into_panic());
                }
            }
        }
    };
    response.headers_mut().insert(
        HEADER_REQUEST_ID,
        http::header::HeaderValue::from_str(&request_id).unwrap(),
    );
    Ok(response)
}

// This function should probably be parametrized by some name of the service
// that is expected to be unique within an organization.  That way, it would be
// possible to determine from a given request id which service it was from.
// TODO should we encode more information here?  Service?  Instance?  Time up to
// the hour?
fn generate_request_id() -> String {
    format!("{}", Uuid::new_v4())
}

/// ServerConnectionHandler is a Hyper Service implementation that forwards
/// incoming connections to `http_connection_handle()`, providing the server
/// state object as an additional argument.  We could use `make_service_fn` here
/// using a closure to capture the state object, but the resulting code is a bit
/// simpler without it.
pub struct ServerConnectionHandler<C: ServerContext> {
    /// backend state that will be made available to the connection handler
    server: Arc<DropshotState<C>>,
}

impl<C: ServerContext> ServerConnectionHandler<C> {
    /// Create an ServerConnectionHandler with the given state object that
    /// will be made available to the handler.
    fn new(server: Arc<DropshotState<C>>) -> Self {
        ServerConnectionHandler { server }
    }
}

impl<T: ServerContext> Service<&AddrStream> for ServerConnectionHandler<T> {
    // Recall that a Service in this context is just something that takes a
    // request (which could be anything) and produces a response (which could be
    // anything).  This being a connection handler, the request type is an
    // AddrStream (which wraps a TCP connection) and the response type is
    // another Service: one that accepts HTTP requests and produces HTTP
    // responses.
    type Response = ServerRequestHandler<T>;
    type Error = GenericError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        // TODO is this right?
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, conn: &AddrStream) -> Self::Future {
        // We're given a borrowed reference to the AddrStream, but our interface
        // is async (which is good, so that we can support time-consuming
        // operations as part of receiving requests).  To avoid having to ensure
        // that conn's lifetime exceeds that of this async operation, we simply
        // copy the only useful information out of the conn: the SocketAddr.  We
        // may want to create our own connection type to encapsulate the socket
        // address and any other per-connection state that we want to keep.
        let server = Arc::clone(&self.server);
        let remote_addr = conn.remote_addr();
        Box::pin(http_connection_handle(server, remote_addr))
    }
}

/// ServerRequestHandler is a Hyper Service implementation that forwards
/// incoming requests to `http_request_handle_wrap()`, including as an argument
/// the backend server state object.  We could use `service_fn` here using a
/// closure to capture the server state object, but the resulting code is a bit
/// simpler without all that.
pub struct ServerRequestHandler<C: ServerContext> {
    /// backend state that will be made available to the request handler
    server: Arc<DropshotState<C>>,
    remote_addr: SocketAddr,
}

impl<C: ServerContext> ServerRequestHandler<C> {
    /// Create a ServerRequestHandler object with the given state object that
    /// will be provided to the handler function.
    fn new(server: Arc<DropshotState<C>>, remote_addr: SocketAddr) -> Self {
        ServerRequestHandler { server, remote_addr }
    }
}

impl<C: ServerContext> Service<Request<Body>> for ServerRequestHandler<C> {
    type Response = Response<Body>;
    type Error = GenericError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        // TODO is this right?
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        Box::pin(http_request_handle_wrap(
            Arc::clone(&self.server),
            self.remote_addr,
            req,
        ))
    }
}

fn io_error(err: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, err)
}

#[cfg(test)]
mod test {
    use super::*;
    // Referring to the current crate as "dropshot::" instead of "crate::"
    // helps the endpoint macro with module lookup.
    use crate as dropshot;
    use dropshot::endpoint;
    use dropshot::test_util::ClientTestContext;
    use dropshot::test_util::LogContext;
    use dropshot::ConfigLogging;
    use dropshot::ConfigLoggingLevel;
    use dropshot::HttpError;
    use dropshot::HttpResponseOk;
    use dropshot::RequestContext;
    use http::StatusCode;
    use hyper::Method;

    use futures::future::FusedFuture;

    #[endpoint {
        method = GET,
        path = "/handler",
    }]
    async fn handler(
        _rqctx: RequestContext<i32>,
    ) -> Result<HttpResponseOk<u64>, HttpError> {
        Ok(HttpResponseOk(3))
    }

    struct TestConfig {
        log_context: LogContext,
    }

    impl TestConfig {
        fn log(&self) -> &slog::Logger {
            &self.log_context.log
        }
    }

    fn create_test_server() -> (HttpServer<i32>, TestConfig) {
        let config_dropshot = ConfigDropshot::default();

        let mut api = ApiDescription::new();
        api.register(handler).unwrap();

        let config_logging =
            ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Warn };
        let log_context = LogContext::new("test server", &config_logging);
        let log = &log_context.log;

        let server = HttpServerStarter::new(&config_dropshot, api, 0, log)
            .unwrap()
            .start();

        (server, TestConfig { log_context })
    }

    async fn single_client_request(addr: SocketAddr, log: &slog::Logger) {
        let client_log = log.new(o!("http_client" => "dropshot test suite"));
        let client_testctx = ClientTestContext::new(addr, client_log);
        tokio::task::spawn(async move {
            let response = client_testctx
                .make_request(
                    Method::GET,
                    "/handler",
                    None as Option<()>,
                    StatusCode::OK,
                )
                .await;

            assert!(response.is_ok());
        })
        .await
        .expect("client request failed");
    }

    #[tokio::test]
    async fn test_server_run_then_close() {
        let (mut server, config) = create_test_server();
        let client = single_client_request(server.local_addr(), config.log());

        futures::select! {
            _ = client.fuse() => {},
            r = server => panic!("Server unexpectedly terminated: {:?}", r),
        }

        assert!(!server.is_terminated());
        assert!(server.close().await.is_ok());
    }

    #[tokio::test]
    async fn test_drop_server_without_close_okay() {
        let (server, _) = create_test_server();
        std::mem::drop(server);
    }
}
