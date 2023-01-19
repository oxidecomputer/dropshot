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
use std::convert::TryFrom;
use std::future::Future;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::ReadBuf;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{server::TlsStream, TlsAcceptor};
use uuid::Uuid;

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
    pub request_body_max_bytes: usize,
    /// maximum size of any page of results
    pub page_max_nitems: NonZeroU32,
    /// default size for a page of results
    pub page_default_nitems: NonZeroU32,
}

/// A thin wrapper around a Hyper Server object that exposes some interfaces that
/// we find useful.
pub struct HttpServerStarter<C: ServerContext> {
    app_state: Arc<DropshotState<C>>,
    local_addr: SocketAddr,
    wrapped: WrappedHttpServerStarter<C>,
}

impl<C: ServerContext> HttpServerStarter<C> {
    pub fn new(
        config: &ConfigDropshot,
        api: ApiDescription<C>,
        private: C,
        log: &Logger,
    ) -> Result<HttpServerStarter<C>, GenericError> {
        let server_config = ServerConfig {
            // We start aggressively to ensure test coverage.
            request_body_max_bytes: config.request_body_max_bytes,
            page_max_nitems: NonZeroU32::new(10000).unwrap(),
            page_default_nitems: NonZeroU32::new(100).unwrap(),
        };

        let starter = match config.tls {
            Some(_) => {
                let (starter, app_state, local_addr) =
                    InnerHttpsServerStarter::new(
                        config,
                        server_config,
                        api,
                        private,
                        log,
                    )?;
                HttpServerStarter {
                    app_state,
                    local_addr,
                    wrapped: WrappedHttpServerStarter::Https(starter),
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
                    )?;
                HttpServerStarter {
                    app_state,
                    local_addr,
                    wrapped: WrappedHttpServerStarter::Http(starter),
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

        tokio::spawn(async { graceful.await })
    }

    /// Set up an HTTP server bound on the specified address that runs registered
    /// handlers.  You must invoke `start()` on the returned instance of
    /// `HttpServerStarter` (and await the result) to actually start the server.
    ///
    /// TODO-cleanup We should be able to take a reference to the ApiDescription.
    /// We currently can't because we need to hang onto the router.
    fn new(
        config: &ConfigDropshot,
        server_config: ServerConfig,
        api: ApiDescription<C>,
        private: C,
        log: &Logger,
    ) -> Result<InnerHttpServerStarterNewReturn<C>, hyper::Error> {
        let incoming = AddrIncoming::bind(&config.bind_address)?;
        let local_addr = incoming.local_addr();

        // TODO-cleanup too many Arcs?
        let app_state = Arc::new(DropshotState {
            private,
            config: server_config,
            router: api.into_router(),
            log: log.new(o!("local_addr" => local_addr)),
            local_addr,
            tls_acceptor: None,
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
// Eventually we may want to change the APIs to allow users to pass
// a rustls::ServerConfig themselves
impl TryFrom<&ConfigTls> for rustls::ServerConfig {
    type Error = std::io::Error;

    fn try_from(config: &ConfigTls) -> std::io::Result<Self> {
        let certs = load_certs(&config)?;
        let private_key = load_private_key(&config)?;
        let mut cfg = rustls::ServerConfig::builder()
            // TODO: We may want to expose protocol configuration in our
            // config
            .with_safe_default_cipher_suites()
            .with_safe_default_kx_groups()
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_client_cert_verifier(rustls::server::NoClientAuth::new())
            .with_single_cert(certs, private_key)
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

        tokio::spawn(async { graceful.await })
    }

    fn new(
        config: &ConfigDropshot,
        server_config: ServerConfig,
        api: ApiDescription<C>,
        private: C,
        log: &Logger,
    ) -> Result<InnerHttpsServerStarterNewReturn<C>, GenericError> {
        let acceptor = Arc::new(Mutex::new(TlsAcceptor::from(Arc::new(
            // Unwrap is safe here because we cannot enter this code path
            // without a TLS configuration
            rustls::ServerConfig::try_from(config.tls.as_ref().unwrap())?,
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

pub type SharedBoxFuture<T> = Shared<Pin<Box<dyn Future<Output = T> + Send>>>;

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
    /// To trigger a shutdown, Call [HttpServer::close] (which also awaits shutdown).
    pub fn wait_for_shutdown(&self) -> SharedBoxFuture<Result<(), String>> {
        self.join_future.clone()
    }

    /// Signals the currently running server to stop and waits for it to exit.
    pub async fn close(mut self) -> Result<(), String> {
        self.closer
            .close_channel
            .take()
            .expect("cannot close twice")
            .send(())
            .expect("failed to send close signal");
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
            c.send(()).expect("failed to send close signal")
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
    let request_id = generate_request_id();
    let request_log = server.log.new(o!(
        "remote_addr" => remote_addr,
        "req_id" => request_id.clone(),
        "method" => request.method().as_str().to_string(),
        "uri" => format!("{}", request.uri()),
    ));
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

    let maybe_response = http_request_handle(
        server,
        request,
        &request_id,
        request_log.new(o!()),
    )
    .await;

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
                "response_code" => r.status().as_str().to_string(),
                "error_message_internal" => message_internal,
                "error_message_external" => message_external,
            );

            r
        }

        Ok(response) => {
            // TODO-debug: add request and response headers here
            info!(request_log, "request completed";
                "response_code" => response.status().as_str().to_string()
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
) -> Result<Response<Body>, HttpError> {
    // TODO-hardening: is it correct to (and do we correctly) read the entire
    // request body even if we decide it's too large and are going to send a 400
    // response?
    // TODO-hardening: add a request read timeout as well so that we don't allow
    // this to take forever.
    // TODO-correctness: Do we need to dump the body on errors?
    let method = request.method();
    let uri = request.uri();
    let lookup_result =
        server.router.lookup_route(&method, uri.path().into())?;
    let rqctx = RequestContext {
        server: Arc::clone(&server),
        request: RequestInfo::from(&request),
        path_variables: lookup_result.variables,
        body_content_type: lookup_result.body_content_type,
        request_id: request_id.to_string(),
        log: request_log,
    };
    let mut response =
        lookup_result.handler.handle_request(rqctx, request).await?;
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

// Load public certificate from config.
fn load_certs(config: &ConfigTls) -> std::io::Result<Vec<rustls::Certificate>> {
    let mut reader = config.cert_reader()?;

    // Load and return certificate.
    rustls_pemfile::certs(&mut reader)
        .map_err(|err| io_error(format!("failed to load certificate: {err}")))
        .map(|mut chain| chain.drain(..).map(rustls::Certificate).collect())
}

// Load private key from config.
fn load_private_key(config: &ConfigTls) -> std::io::Result<rustls::PrivateKey> {
    let mut reader = config.key_reader()?;

    // Load and return a single private key.
    let keys =
        rustls_pemfile::pkcs8_private_keys(&mut reader).map_err(|err| {
            io_error(format!("failed to load private key: {err}"))
        })?;
    if keys.len() != 1 {
        return Err(io_error("expected a single private key".into()));
    }
    Ok(rustls::PrivateKey(keys[0].clone()))
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
