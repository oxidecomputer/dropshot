// Copyright 2025 Oxide Computer Company
//! Generic server-wide state and facilities

use super::api_description::ApiDescription;
use super::body::Body;
use super::config::{ConfigDropshot, ConfigTls};
#[cfg(feature = "usdt-probes")]
use super::dtrace::probes;
use super::error::HttpError;
use super::handler::HandlerError;
use super::handler::RequestContext;
use super::http_util::HEADER_REQUEST_ID;
use super::router::HttpRouter;
use super::versioning::VersionPolicy;
use super::ProbeRegistration;

use async_stream::stream;
use debug_ignore::DebugIgnore;
use futures::future::{
    BoxFuture, FusedFuture, FutureExt, Shared, TryFutureExt,
};
use futures::lock::Mutex;
use futures::stream::{Stream, StreamExt};
use hyper::service::Service;
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
use thiserror::Error;

// TODO Remove when we can remove `HttpServerStarter`
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
    /// specifies how incoming requests are mapped to handlers based on versions
    pub(crate) version_policy: VersionPolicy,
}

impl<C: ServerContext> DropshotState<C> {
    pub fn using_tls(&self) -> bool {
        self.tls_acceptor.is_some()
    }
}

/// Stores static configuration associated with the server
/// TODO-cleanup merge with ConfigDropshot
#[derive(Debug)]
pub struct ServerConfig {
    /// maximum allowed size of a request body
    pub default_request_body_max_bytes: usize,
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

/// See [`ServerBuilder`] instead.
// It would be nice to remove this structure altogether once we've got
// confidence that no consumers actually need to distinguish between the
// configuration and start steps.
pub struct HttpServerStarter<C: ServerContext> {
    app_state: Arc<DropshotState<C>>,
    local_addr: SocketAddr,
    handler_waitgroup: WaitGroup,
    http_acceptor: HttpAcceptor,
    tls_acceptor: Option<Arc<Mutex<TlsAcceptor>>>,
}

impl<C: ServerContext> HttpServerStarter<C> {
    /// Make an `HttpServerStarter` to start an `HttpServer`
    ///
    /// This function exists for backwards compatibility.  You should use
    /// [`ServerBuilder`] instead.
    pub fn new(
        config: &ConfigDropshot,
        api: ApiDescription<C>,
        private: C,
        log: &Logger,
    ) -> Result<HttpServerStarter<C>, GenericError> {
        HttpServerStarter::new_with_tls(config, api, private, log, None)
    }

    /// Make an `HttpServerStarter` to start an `HttpServer`
    ///
    /// This function exists for backwards compatibility.  You should use
    /// [`ServerBuilder`] instead.
    pub fn new_with_tls(
        config: &ConfigDropshot,
        api: ApiDescription<C>,
        private: C,
        log: &Logger,
        tls: Option<ConfigTls>,
    ) -> Result<HttpServerStarter<C>, GenericError> {
        ServerBuilder::new(api, private, log.clone())
            .config(config.clone())
            .tls(tls)
            .build_starter()
            .map_err(|e| Box::new(e) as GenericError)
    }

    fn new_internal(
        config: &ConfigDropshot,
        api: ApiDescription<C>,
        private: C,
        log: &Logger,
        tls: Option<ConfigTls>,
        version_policy: VersionPolicy,
    ) -> Result<HttpServerStarter<C>, BuildError> {
        let tcp = {
            let std_listener = std::net::TcpListener::bind(
                &config.bind_address,
            )
            .map_err(|e| BuildError::bind_error(e, config.bind_address))?;
            std_listener.set_nonblocking(true).map_err(|e| {
                BuildError::generic_system(e, "setting non-blocking")
            })?;
            // We use `from_std` instead of just calling `bind` here directly
            // to avoid invoking an async function.
            TcpListener::from_std(std_listener).map_err(|e| {
                BuildError::generic_system(e, "creating TCP listener")
            })?
        };

        let local_addr = tcp.local_addr().map_err(|e| {
            BuildError::generic_system(e, "getting local TCP address")
        })?;

        let log = log.new(o!("local_addr" => local_addr));

        let server_config = ServerConfig {
            // We start aggressively to ensure test coverage.
            default_request_body_max_bytes: config
                .default_request_body_max_bytes,
            page_max_nitems: NonZeroU32::new(10000).unwrap(),
            page_default_nitems: NonZeroU32::new(100).unwrap(),
            default_handler_task_mode: config.default_handler_task_mode,
            log_headers: config.log_headers.clone(),
        };

        let tls_acceptor = tls
            .as_ref()
            .map(|tls| {
                Ok(Arc::new(Mutex::new(TlsAcceptor::from(Arc::new(
                    rustls::ServerConfig::try_from(tls)?,
                )))))
            })
            .transpose()?;
        let handler_waitgroup = WaitGroup::new();

        let router = api.into_router();
        if let VersionPolicy::Unversioned = version_policy {
            if router.has_versioned_routes() {
                return Err(BuildError::UnversionedServerHasVersionedRoutes);
            }
        }

        let app_state = Arc::new(DropshotState {
            private,
            config: server_config,
            router,
            log: log.clone(),
            local_addr,
            tls_acceptor: tls_acceptor.clone(),
            handler_waitgroup_worker: DebugIgnore(handler_waitgroup.worker()),
            version_policy,
        });

        for (path, method, endpoint) in app_state.router.endpoints(None) {
            debug!(&log, "registered endpoint";
                "method" => &method,
                "path" => &path,
                "versions" => &endpoint.versions,
            );
        }

        let http_acceptor = HttpAcceptor { tcp, log: log.clone() };

        Ok(HttpServerStarter {
            app_state,
            local_addr,
            handler_waitgroup,
            http_acceptor,
            tls_acceptor,
        })
    }

    pub fn start(self) -> HttpServer<C> {
        let HttpServerStarter {
            app_state,
            local_addr,
            handler_waitgroup,
            tls_acceptor,
            http_acceptor,
        } = self;

        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        let make_service = ServerConnectionHandler::new(Arc::clone(&app_state));
        let log = &app_state.log;
        let log_close = log.clone();
        let join_handle = tokio::spawn(async move {
            use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
            use hyper_util::server::conn::auto;

            let mut builder = auto::Builder::new(TokioExecutor::new());
            // http/1 settings
            builder.http1().timer(TokioTimer::new());
            // http/2 settings
            builder.http2().timer(TokioTimer::new());

            // Use a graceful watcher to keep track of all existing connections,
            // and when the close_signal is trigger, force all known conns
            // to start a graceful shutdown.
            let graceful =
                hyper_util::server::graceful::GracefulShutdown::new();

            // The following code looks superficially similar between the HTTP
            // and HTTPS paths.  However, the concrete types of various objects
            // are different and so it's not easy to actually share the code.
            let log = log_close;
            match tls_acceptor {
                Some(tls_acceptor) => {
                    let mut https_acceptor = HttpsAcceptor::new(
                        log.clone(),
                        tls_acceptor,
                        http_acceptor,
                    );
                    loop {
                        tokio::select! {
                            Some(Ok(sock)) = https_acceptor.accept() => {
                                let remote_addr = sock.remote_addr();
                                let handler = make_service
                                    .make_http_request_handler(remote_addr);
                                let fut = builder
                                    .serve_connection_with_upgrades(
                                        TokioIo::new(sock),
                                        handler,
                                    );
                                let fut = graceful.watch(fut.into_owned());
                                tokio::spawn(fut);
                            },

                            _ = &mut rx => {
                                info!(log, "beginning graceful shutdown");
                                break;
                            }
                        }
                    }
                }
                None => loop {
                    tokio::select! {
                        (sock, remote_addr) = http_acceptor.accept() => {
                            let handler = make_service
                                .make_http_request_handler(remote_addr);
                            let fut = builder
                                .serve_connection_with_upgrades(
                                    TokioIo::new(sock),
                                    handler,
                                );
                            let fut = graceful.watch(fut.into_owned());
                            tokio::spawn(fut);
                        },

                        _ = &mut rx => {
                            info!(log, "beginning graceful shutdown");
                            break;
                        }
                    }
                },
            };

            // optional: could use another select on a timeout
            graceful.shutdown().await
        });

        info!(log, "listening");

        let join_handle = async move {
            // After the server shuts down, we also want to wait for any
            // detached handler futures to complete.
            () = join_handle
                .await
                .map_err(|e| format!("server stopped: {e}"))?;
            () = handler_waitgroup.wait().await;
            Ok(())
        };

        #[cfg(feature = "usdt-probes")]
        let probe_registration = match usdt::register_probes() {
            Ok(_) => {
                debug!(&log, "successfully registered DTrace USDT probes");
                ProbeRegistration::Succeeded
            }
            Err(e) => {
                let msg = e.to_string();
                error!(&log, "failed to register DTrace USDT probes: {}", msg);
                ProbeRegistration::Failed(msg)
            }
        };
        #[cfg(not(feature = "usdt-probes"))]
        let probe_registration = {
            debug!(&log, "DTrace USDT probes compiled out, not registering");
            ProbeRegistration::Disabled
        };

        HttpServer {
            probe_registration,
            app_state,
            local_addr,
            closer: CloseHandle { close_channel: Some(tx) },
            join_future: join_handle.boxed().shared(),
        }
    }
}

/// Accepts TCP connections like a `TcpListener`, but ignores transient errors
/// rather than propagating them to the caller
struct HttpAcceptor {
    tcp: TcpListener,
    log: slog::Logger,
}

impl HttpAcceptor {
    async fn accept(&self) -> (TcpStream, SocketAddr) {
        loop {
            match self.tcp.accept().await {
                Ok((socket, addr)) => return (socket, addr),
                Err(e) => match e.kind() {
                    // These are errors on the individual socket that we
                    // tried to accept, and so can be ignored.
                    std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::ConnectionReset => (),

                    // This could EMFILE implying resource exhaustion.
                    // Sleep a little bit and try again.
                    _ => {
                        warn!(self.log, "accept error"; "error" => e);
                        tokio::time::sleep(std::time::Duration::from_millis(
                            100,
                        ))
                        .await;
                    }
                },
            }
        }
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
        http_acceptor: HttpAcceptor,
    ) -> HttpsAcceptor {
        HttpsAcceptor {
            stream: Box::new(Box::pin(Self::new_stream(
                log,
                tls_acceptor,
                http_acceptor,
            ))),
        }
    }

    async fn accept(&mut self) -> Option<std::io::Result<TlsConn>> {
        self.stream.next().await
    }

    fn new_stream(
        log: slog::Logger,
        tls_acceptor: Arc<Mutex<TlsAcceptor>>,
        http_acceptor: HttpAcceptor,
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
                    (socket, addr) = http_acceptor.accept() => {
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

/// Create a TLS configuration from the Dropshot config structure.
impl TryFrom<&ConfigTls> for rustls::ServerConfig {
    type Error = BuildError;

    fn try_from(config: &ConfigTls) -> Result<Self, Self::Error> {
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
                        BuildError::generic_system(
                            e,
                            format!("opening {}", cert_file.display()),
                        )
                    })?,
                ));
                let keyfile = Box::new(std::io::BufReader::new(
                    std::fs::File::open(key_file).map_err(|e| {
                        BuildError::generic_system(
                            e,
                            format!("opening {}", key_file.display()),
                        )
                    })?,
                ));
                (certfile, keyfile)
            }
        };

        let certs = rustls_pemfile::certs(&mut cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| {
                BuildError::generic_system(err, "loading TLS certificates")
            })?;
        let keys = rustls_pemfile::pkcs8_private_keys(&mut key_reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| {
                BuildError::generic_system(err, "loading TLS private key")
            })?;
        let mut keys_iter = keys.into_iter();
        let (Some(private_key), None) = (keys_iter.next(), keys_iter.next())
        else {
            return Err(BuildError::NotOnePrivateKey);
        };

        let mut cfg = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, private_key.into())
            .expect("bad certificate/key");
        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Ok(cfg)
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

/// Initial entry point for handling a new request to the HTTP server.  This is
/// invoked by Hyper when a new request is received.  This function returns a
/// Result that either represents a valid HTTP response or an error (which will
/// also get turned into an HTTP response).
async fn http_request_handle_wrap<C: ServerContext>(
    server: Arc<DropshotState<C>>,
    remote_addr: SocketAddr,
    request: Request<hyper::body::Incoming>,
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
            {
                let status = error.status_code();
                let message_external = error.external_message();
                let message_internal = error.internal_message();

                #[cfg(feature = "usdt-probes")]
                probes::request__done!(|| {
                    crate::dtrace::ResponseInfo {
                        id: request_id.clone(),
                        local_addr,
                        remote_addr,
                        status_code: status.as_u16(),
                        message: message_external
                            .cloned()
                            .unwrap_or_else(|| message_internal.clone()),
                    }
                });

                // TODO-debug: add request and response headers here
                info!(request_log, "request completed";
                    "response_code" => status.as_u16(),
                    "latency_us" => latency_us,
                    "error_message_internal" => message_internal,
                    "error_message_external" => message_external,
                );
            };
            error.into_response(&request_id)
        }

        Ok(response) => {
            // TODO-debug: add request and response headers here
            info!(request_log, "request completed";
                "response_code" => response.status().as_u16(),
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
    request: Request<hyper::body::Incoming>,
    request_id: &str,
    request_log: Logger,
    remote_addr: std::net::SocketAddr,
) -> Result<Response<Body>, HandlerError> {
    // TODO-hardening: is it correct to (and do we correctly) read the entire
    // request body even if we decide it's too large and are going to send a 400
    // response?
    // TODO-hardening: add a request read timeout as well so that we don't allow
    // this to take forever.
    // TODO-correctness: Do we need to dump the body on errors?
    let request = request.map(crate::Body::wrap);
    let method = request.method();
    let uri = request.uri();
    let found_version =
        server.version_policy.request_version(&request, &request_log)?;
    let lookup_result = server.router.lookup_route(
        &method,
        uri.path().into(),
        found_version.as_ref(),
    )?;
    let rqctx = RequestContext {
        server: Arc::clone(&server),
        request: RequestInfo::new(&request, remote_addr),
        endpoint: lookup_result.endpoint,
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
                            "response_code" => r.status().as_u16(),
                        ),
                        Err(error) => {
                            warn!(request_log, "request completed after handler was already cancelled";
                                "response_code" => error.status_code().as_u16(),
                                "error_message_internal" => error.internal_message(),
                                "error_message_external" => error.external_message(),,
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
                    error!(
                        request_log,
                        "handler panicked; returning 500 error"
                    );

                    // To get the panic, we now need to await `handler_task`; we
                    // know it is complete _and_ it failed, because it has
                    // dropped `tx` without sending us a result, which is only
                    // possible if it panicked.
                    let task_err = handler_task.await.expect_err(
                        "task failed to send result but didn't panic",
                    );

                    // Extract panic message if possible
                    let panic_msg = if task_err.is_panic() {
                        match task_err.into_panic().downcast::<&'static str>() {
                            Ok(s) => s.to_string(),
                            Err(panic_any) => {
                                match panic_any.downcast::<String>() {
                                    Ok(s) => *s,
                                    Err(_) => "handler panicked".to_string(),
                                }
                            }
                        }
                    } else {
                        task_err.to_string()
                    };

                    // Instead of propagating the panic, return a 500 error
                    // This ensures proper status code reporting (500 instead of 499)
                    return Err(HandlerError::Dropshot(
                        HttpError::for_internal_error(panic_msg),
                    ));
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

    /// Initial entry point for handling a new connection to the HTTP server.
    /// This is invoked by Hyper when a new connection is accepted.  This function
    /// must return a Hyper Service object that will handle requests for this
    /// connection.
    fn make_http_request_handler(
        &self,
        remote_addr: SocketAddr,
    ) -> ServerRequestHandler<C> {
        info!(self.server.log, "accepted connection"; "remote_addr" => %remote_addr);
        ServerRequestHandler::new(self.server.clone(), remote_addr)
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

impl<C: ServerContext> Service<Request<hyper::body::Incoming>>
    for ServerRequestHandler<C>
{
    type Response = Response<Body>;
    type Error = GenericError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn call(&self, req: Request<hyper::body::Incoming>) -> Self::Future {
        Box::pin(http_request_handle_wrap(
            Arc::clone(&self.server),
            self.remote_addr,
            req,
        ))
    }
}

/// Errors encountered while configuring a Dropshot server
#[derive(Debug, Error)]
pub enum BuildError {
    #[error("failed to bind to {address}")]
    BindError {
        address: SocketAddr,
        #[source]
        error: std::io::Error,
    },
    #[error("expected exactly one TLS private key")]
    NotOnePrivateKey,
    #[error("{context}")]
    SystemError {
        context: String,
        #[source]
        error: std::io::Error,
    },
    #[error(
        "unversioned servers cannot have endpoints with specific versions"
    )]
    UnversionedServerHasVersionedRoutes,
}

impl BuildError {
    /// Generate an error for failure to bind to `address`
    fn bind_error(error: std::io::Error, address: SocketAddr) -> BuildError {
        BuildError::BindError { address, error }
    }

    /// Generate an error for any kind of `std::io::Error`
    ///
    /// `context` describes more about what we were trying to do that generated
    /// the error.
    fn generic_system<S: Into<String>>(
        error: std::io::Error,
        context: S,
    ) -> BuildError {
        BuildError::SystemError { context: context.into(), error }
    }
}

/// Start configuring a Dropshot server
#[derive(Debug)]
pub struct ServerBuilder<C: ServerContext> {
    // required caller-provided values
    private: C,
    log: Logger,
    api: DebugIgnore<ApiDescription<C>>,

    // optional caller-provided values
    config: ConfigDropshot,
    version_policy: VersionPolicy,
    tls: Option<ConfigTls>,
}

impl<C: ServerContext> ServerBuilder<C> {
    /// Start configuring a new Dropshot server
    ///
    /// * `api`: the API to be hosted on this server
    /// * `private`: your private data that will be made available in
    ///   `RequestContext`
    /// * `log`: a slog logger for all server events
    pub fn new(
        api: ApiDescription<C>,
        private: C,
        log: Logger,
    ) -> ServerBuilder<C> {
        ServerBuilder {
            private,
            log,
            api: DebugIgnore(api),
            config: Default::default(),
            version_policy: VersionPolicy::Unversioned,
            tls: Default::default(),
        }
    }

    /// Specify the server configuration
    pub fn config(mut self, config: ConfigDropshot) -> Self {
        self.config = config;
        self
    }

    /// Specify the TLS configuration, if any
    ///
    /// `None` (the default) means no TLS.  The server will listen for plain
    /// HTTP.
    pub fn tls(mut self, tls: Option<ConfigTls>) -> Self {
        self.tls = tls;
        self
    }

    /// Specifies whether and how this server determines the API version to use
    /// for incoming requests
    ///
    /// All the interfaces related to [`VersionPolicy`] are considered
    /// experimental and may change in an upcoming release.
    pub fn version_policy(mut self, version_policy: VersionPolicy) -> Self {
        self.version_policy = version_policy;
        self
    }

    /// Start the server
    ///
    /// # Errors
    ///
    /// See [`ServerBuilder::build_starter()`].
    pub fn start(self) -> Result<HttpServer<C>, BuildError> {
        Ok(self.build_starter()?.start())
    }

    /// Build an `HttpServerStarter` that can be used to start the server
    ///
    /// Most consumers probably want to use `start()` instead.
    ///
    /// # Errors
    ///
    /// This fails if:
    ///
    /// * We could not bind to the requested IP address and TCP port
    /// * The provided `tls` configuration was not valid
    /// * The `version_policy` is `VersionPolicy::Unversioned` and `api` (the
    ///   `ApiDescription`) contains any endpoints that are version-restricted
    ///   (i.e., have "versions" set to anything other than
    ///   `ApiEndpointVersions::All)`.  Versioned routes are not supported with
    ///   unversioned servers.
    pub fn build_starter(self) -> Result<HttpServerStarter<C>, BuildError> {
        HttpServerStarter::new_internal(
            &self.config,
            self.api.0,
            self.private,
            &self.log,
            self.tls,
            self.version_policy,
        )
    }
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

    #[tokio::test]
    async fn test_http_acceptor_happy_path() {
        const TOTAL: usize = 100;
        let tcp =
            tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = tcp.local_addr().expect("local_addr");
        let acceptor =
            HttpAcceptor { log: slog::Logger::root(slog::Discard, o!()), tcp };

        let t1 = tokio::spawn(async move {
            for _ in 0..TOTAL {
                let _ = acceptor.accept().await;
            }
        });

        let t2 = tokio::spawn(async move {
            for _ in 0..TOTAL {
                tokio::net::TcpStream::connect(&addr).await.expect("connect");
            }
        });

        t1.await.expect("task 1");
        t2.await.expect("task 2");
    }
}
