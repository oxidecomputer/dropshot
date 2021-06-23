// Copyright 2020 Oxide Computer Company
/*!
 * Generic server-wide state and facilities
 */

use super::api_description::ApiDescription;
use super::config::ConfigDropshot;
use super::error::HttpError;
use super::handler::RequestContext;
use super::http_util::HEADER_REQUEST_ID;
use super::router::HttpRouter;

use futures::future::BoxFuture;
use futures::future::FusedFuture;
use futures::future::FutureExt;
use futures::lock::Mutex;
use hyper::server::{
    conn::{AddrIncoming, AddrStream},
    Server,
};
use hyper::service::Service;
use hyper::Body;
use hyper::Request;
use hyper::Response;
use std::future::Future;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use uuid::Uuid;

use slog::Logger;

/* TODO Replace this with something else? */
type GenericError = Box<dyn std::error::Error + Send + Sync>;

/**
 * Endpoint-accessible context associated with a server.
 *
 * Automatically implemented for all Send + Sync types.
 */
pub trait ServerContext: Send + Sync + 'static {}

impl<T: 'static> ServerContext for T where T: Send + Sync {}

/**
 * Stores shared state used by the Dropshot server.
 */
pub struct DropshotState<C: ServerContext> {
    /** caller-specific state */
    pub private: C,
    /** static server configuration parameters */
    pub config: ServerConfig,
    /** request router */
    pub router: HttpRouter<C>,
    /** server-wide log handle */
    pub log: Logger,
}

/**
 * Stores static configuration associated with the server
 * TODO-cleanup merge with ConfigDropshot
 */
pub struct ServerConfig {
    /** maximum allowed size of a request body */
    pub request_body_max_bytes: usize,
    /** maximum size of any page of results */
    pub page_max_nitems: NonZeroU32,
    /** default size for a page of results */
    pub page_default_nitems: NonZeroU32,
}

/**
 * A thin wrapper around a Hyper Server object that exposes some interfaces that
 * we find useful.
 */
pub struct HttpServerStarter<C: ServerContext> {
    app_state: Arc<DropshotState<C>>,
    server: Server<AddrIncoming, ServerConnectionHandler<C>>,
    local_addr: SocketAddr,
}

impl<C: ServerContext> HttpServerStarter<C> {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /**
     * Begins execution of the underlying Http server.
     */
    pub fn start(self) -> HttpServer<C> {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let log_close = self.app_state.log.new(o!());
        let graceful = self.server.with_graceful_shutdown(async move {
            rx.await.expect(
                "dropshot server shutting down without invoking close()",
            );
            info!(log_close, "received request to begin graceful shutdown");
        });

        let join_handle = tokio::spawn(async { graceful.await });

        HttpServer {
            app_state: self.app_state,
            local_addr: self.local_addr,
            join_handle: Some(join_handle),
            close_channel: Some(tx),
        }
    }

    /**
     * Set up an HTTP server bound on the specified address that runs registered
     * handlers.  You must invoke `start()` on the returned instance of
     * `HttpServerStarter` (and await the result) to actually start the server.
     *
     * TODO-cleanup We should be able to take a reference to the ApiDescription.
     * We currently can't because we need to hang onto the router.
     */
    pub fn new(
        config: &ConfigDropshot,
        api: ApiDescription<C>,
        private: C,
        log: &Logger,
    ) -> Result<HttpServerStarter<C>, hyper::Error> {
        /* TODO-cleanup too many Arcs? */
        let app_state = Arc::new(DropshotState {
            private,
            config: ServerConfig {
                /* We start aggressively to ensure test coverage. */
                request_body_max_bytes: config.request_body_max_bytes,
                page_max_nitems: NonZeroU32::new(10000).unwrap(),
                page_default_nitems: NonZeroU32::new(100).unwrap(),
            },
            router: api.into_router(),
            log: log.new(o!()),
        });

        for (path, method, _) in &app_state.router {
            debug!(app_state.log, "registered endpoint";
                "method" => &method,
                "path" => &path
            );
        }

        let make_service = ServerConnectionHandler::new(Arc::clone(&app_state));
        let builder = hyper::Server::try_bind(&config.bind_address)?;
        let server = builder.serve(make_service);
        let local_addr = server.local_addr();
        info!(app_state.log, "listening"; "local_addr" => %local_addr);

        Ok(HttpServerStarter {
            app_state,
            server,
            local_addr,
        })
    }

    pub fn app_private(&self) -> &C {
        &self.app_state.private
    }
}

/**
 * A running Dropshot HTTP server.
 *
 * # Panics
 *
 * Panics if dropped without invoking `close`.
 */
pub struct HttpServer<C: ServerContext> {
    app_state: Arc<DropshotState<C>>,
    local_addr: SocketAddr,
    join_handle: Option<tokio::task::JoinHandle<Result<(), hyper::Error>>>,
    close_channel: Option<tokio::sync::oneshot::Sender<()>>,
}

impl<C: ServerContext> HttpServer<C> {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn app_private(&self) -> &C {
        &self.app_state.private
    }

    /**
     * Signals the currently running server to stop and waits for it to exit.
     */
    pub async fn close(mut self) -> Result<(), String> {
        self.close_channel
            .take()
            .expect("cannot close twice")
            .send(())
            .expect("failed to send close signal");
        if let Some(handle) = self.join_handle.take() {
            handle
                .await
                .map_err(|error| format!("waiting for server: {}", error))?
                .map_err(|error| format!("server stopped: {}", error))
        } else {
            Ok(())
        }
    }
}

/*
 * For graceful termination, the `close()` function is preferred, as it can
 * report errors and wait for termination to complete.  However, we impl
 * `Drop` to attempt to shut down the server to handle less clean shutdowns
 * (e.g., from failing tests).
 */
impl<C: ServerContext> Drop for HttpServer<C> {
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
        let mut handle = server
            .join_handle
            .take()
            .expect("polling a server future which has already completed");
        let poll = handle.poll_unpin(cx).map(|result| {
            result
                .map_err(|error| format!("waiting for server: {}", error))?
                .map_err(|error| format!("server stopped: {}", error))
        });

        if poll.is_pending() {
            server.join_handle.replace(handle);
        }
        return poll;
    }
}

impl<C: ServerContext> FusedFuture for HttpServer<C> {
    fn is_terminated(&self) -> bool {
        self.join_handle.is_none()
    }
}

/**
 * Initial entry point for handling a new connection to the HTTP server.
 * This is invoked by Hyper when a new connection is accepted.  This function
 * must return a Hyper Service object that will handle requests for this
 * connection.
 */
async fn http_connection_handle<C: ServerContext>(
    server: Arc<DropshotState<C>>,
    remote_addr: SocketAddr,
) -> Result<ServerRequestHandler<C>, GenericError> {
    info!(server.log, "accepted connection"; "remote_addr" => %remote_addr);
    Ok(ServerRequestHandler::new(server))
}

/**
 * Initial entry point for handling a new request to the HTTP server.  This is
 * invoked by Hyper when a new request is received.  This function returns a
 * Result that either represents a valid HTTP response or an error (which will
 * also get turned into an HTTP response).
 */
async fn http_request_handle_wrap<C: ServerContext>(
    server: Arc<DropshotState<C>>,
    request: Request<Body>,
) -> Result<Response<Body>, GenericError> {
    /*
     * This extra level of indirection makes error handling much more
     * straightforward, since the request handling code can simply return early
     * with an error and we'll treat it like an error from any of the endpoints
     * themselves.
     */
    let request_id = generate_request_id();
    let request_log = server.log.new(o!(
        "req_id" => request_id.clone(),
        "method" => request.method().as_str().to_string(),
        "uri" => format!("{}", request.uri()),
    ));
    trace!(request_log, "incoming request");
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

            /* TODO-debug: add request and response headers here */
            info!(request_log, "request completed";
                "response_code" => r.status().as_str().to_string(),
                "error_message_internal" => message_internal,
                "error_message_external" => message_external,
            );

            r
        }

        Ok(response) => {
            /* TODO-debug: add request and response headers here */
            info!(request_log, "request completed";
                "response_code" => response.status().as_str().to_string()
            );

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
    /*
     * TODO-hardening: is it correct to (and do we correctly) read the entire
     * request body even if we decide it's too large and are going to send a 400
     * response?
     * TODO-hardening: add a request read timeout as well so that we don't allow
     * this to take forever.
     * TODO-correctness: Do we need to dump the body on errors?
     */
    let method = request.method();
    let uri = request.uri();
    let lookup_result =
        server.router.lookup_route(&method, uri.path().into())?;
    let rqctx = RequestContext {
        server: Arc::clone(&server),
        request: Arc::new(Mutex::new(request)),
        path_variables: lookup_result.variables,
        request_id: request_id.to_string(),
        log: request_log,
    };
    let mut response = lookup_result.handler.handle_request(rqctx).await?;
    response.headers_mut().insert(
        HEADER_REQUEST_ID,
        http::header::HeaderValue::from_str(&request_id).unwrap(),
    );
    Ok(response)
}

/*
 * This function should probably be parametrized by some name of the service
 * that is expected to be unique within an organization.  That way, it would be
 * possible to determine from a given request id which service it was from.
 * TODO should we encode more information here?  Service?  Instance?  Time up to
 * the hour?
 */
fn generate_request_id() -> String {
    format!("{}", Uuid::new_v4())
}

/**
 * ServerConnectionHandler is a Hyper Service implementation that forwards
 * incoming connections to `http_connection_handle()`, providing the server
 * state object as an additional argument.  We could use `make_service_fn` here
 * using a closure to capture the state object, but the resulting code is a bit
 * simpler without it.
 */
pub struct ServerConnectionHandler<C: ServerContext> {
    /** backend state that will be made available to the connection handler */
    server: Arc<DropshotState<C>>,
}

impl<C: ServerContext> ServerConnectionHandler<C> {
    /**
     * Create an ServerConnectionHandler with the given state object that
     * will be made available to the handler.
     */
    fn new(server: Arc<DropshotState<C>>) -> Self {
        ServerConnectionHandler {
            server,
        }
    }
}

impl<T: ServerContext> Service<&AddrStream> for ServerConnectionHandler<T> {
    /*
     * Recall that a Service in this context is just something that takes a
     * request (which could be anything) and produces a response (which could be
     * anything).  This being a connection handler, the request type is an
     * AddrStream (which wraps a TCP connection) and the response type is
     * another Service: one that accepts HTTP requests and produces HTTP
     * responses.
     */
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
        /*
         * We're given a borrowed reference to the AddrStream, but our interface
         * is async (which is good, so that we can support time-consuming
         * operations as part of receiving requests).  To avoid having to ensure
         * that conn's lifetime exceeds that of this async operation, we simply
         * copy the only useful information out of the conn: the SocketAddr.  We
         * may want to create our own connection type to encapsulate the socket
         * address and any other per-connection state that we want to keep.
         */
        let server = Arc::clone(&self.server);
        let remote_addr = conn.remote_addr();
        Box::pin(http_connection_handle(server, remote_addr))
    }
}

/**
 * ServerRequestHandler is a Hyper Service implementation that forwards
 * incoming requests to `http_request_handle_wrap()`, including as an argument
 * the backend server state object.  We could use `service_fn` here using a
 * closure to capture the server state object, but the resulting code is a bit
 * simpler without all that.
 */
pub struct ServerRequestHandler<C: ServerContext> {
    /** backend state that will be made available to the request handler */
    server: Arc<DropshotState<C>>,
}

impl<C: ServerContext> ServerRequestHandler<C> {
    /**
     * Create a ServerRequestHandler object with the given state object that
     * will be provided to the handler function.
     */
    fn new(server: Arc<DropshotState<C>>) -> Self {
        ServerRequestHandler {
            server,
        }
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
        Box::pin(http_request_handle_wrap(Arc::clone(&self.server), req))
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
        _rqctx: Arc<RequestContext<i32>>,
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

        let config_logging = ConfigLogging::StderrTerminal {
            level: ConfigLoggingLevel::Warn,
        };
        let log_context = LogContext::new("test server", &config_logging);
        let log = &log_context.log;

        let server = HttpServerStarter::new(&config_dropshot, api, 0, log)
            .unwrap()
            .start();

        (server, TestConfig {
            log_context,
        })
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
        let client = single_client_request(server.local_addr, config.log());

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
