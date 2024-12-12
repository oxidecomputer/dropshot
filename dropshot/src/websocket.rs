// Copyright 2023 Oxide Computer Company
//! Implements websocket upgrades as an Extractor for use in API route handler
//! parameters to indicate that the given endpoint is meant to be upgraded to
//! a websocket.
//!
//! This exposes a raw upgraded HTTP connection to a user-provided async future,
//! which will be spawned to handle the incoming connection.

use crate::api_description::ExtensionMode;
use crate::body::Body;
use crate::{
    ApiEndpointBodyContentType, ExclusiveExtractor, ExtractorMetadata,
    HttpError, RequestContext, ServerContext,
};
use async_trait::async_trait;
use base64::Engine;
use http::header;
use http::Response;
use http::StatusCode;
use hyper::upgrade::OnUpgrade;
use schemars::JsonSchema;
use serde_json::json;
use sha1::{Digest, Sha1};
use slog::Logger;
use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

/// WebsocketUpgrade is an ExclusiveExtractor used to upgrade and handle an HTTP
/// request as a websocket when present in a Dropshot endpoint's function
/// arguments.
///
/// The consumer of this must call [WebsocketUpgrade::handle] for the connection
/// to be upgraded. (This is done for you by `#[channel]`.)
#[derive(Debug)]
pub struct WebsocketUpgrade(Option<WebsocketUpgradeInner>);

/// This is the return type of the websocket-handling future provided to
/// [`dropshot_endpoint::channel`]
/// (which in turn provides it to [WebsocketUpgrade::handle]).
pub type WebsocketChannelResult =
    Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>;

/// [WebsocketUpgrade::handle]'s return type.
/// The `#[endpoint]` handler must return the value returned by
/// [WebsocketUpgrade::handle]. (This is done for you by `#[channel]`.)
pub type WebsocketEndpointResult = Result<Response<Body>, HttpError>;

/// The upgraded connection passed as the last argument to the websocket
/// handler function. [`WebsocketConnection::into_inner`] can be used to
/// access the raw upgraded connection, for passing to any implementation
/// of the websockets protocol.
pub struct WebsocketConnection(WebsocketConnectionRaw);

/// A type that implements [tokio::io::AsyncRead] + [tokio::io::AsyncWrite].
// A newtype so as to not expose the less-stable hyper-util type.
pub struct WebsocketConnectionRaw(
    hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>,
);

impl WebsocketConnection {
    /// Consumes `self` and returns the held raw connection.
    pub fn into_inner(self) -> WebsocketConnectionRaw {
        self.0
    }
}

#[derive(Debug)]
struct WebsocketUpgradeInner {
    upgrade_fut: OnUpgrade,
    accept_key: String,
    route: String,
    ws_log: Logger,
}

// Originally copied from tungstenite-0.17.3 (rather than taking a whole
// dependency for this one function).
fn derive_accept_key(request_key: &[u8]) -> String {
    // ... field is constructed by concatenating /key/ ...
    // ... with the string "258EAFA5-E914-47DA-95CA-C5AB0DC85B11" (RFC 6455)
    const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let mut sha1 = Sha1::default();
    sha1.update(request_key);
    sha1.update(WS_GUID);
    base64::engine::general_purpose::STANDARD.encode(&sha1.finalize())
}

/// This `ExclusiveExtractor` implementation constructs an instance of
/// `WebsocketUpgrade` from an HTTP request, and returns an error if the given
/// request does not contain websocket upgrade headers.
#[async_trait]
impl ExclusiveExtractor for WebsocketUpgrade {
    async fn from_request<Context: ServerContext>(
        rqctx: &RequestContext<Context>,
        request: hyper::Request<Body>,
    ) -> Result<Self, HttpError> {
        if !request
            .headers()
            .get(header::CONNECTION)
            .and_then(|hv| hv.to_str().ok())
            .map(|hv| {
                hv.split(|c| c == ',' || c == ' ')
                    .any(|vs| vs.eq_ignore_ascii_case("upgrade"))
            })
            .unwrap_or(false)
        {
            return Err(HttpError::for_bad_request(
                None,
                "expected connection upgrade".to_string(),
            ));
        }

        if !request
            .headers()
            .get(header::UPGRADE)
            .and_then(|v| v.to_str().ok())
            .map(|v| {
                v.split(|c| c == ',' || c == ' ')
                    .any(|v| v.eq_ignore_ascii_case("websocket"))
            })
            .unwrap_or(false)
        {
            return Err(HttpError::for_bad_request(
                None,
                "unexpected protocol for upgrade".to_string(),
            ));
        }

        if request
            .headers()
            .get(header::SEC_WEBSOCKET_VERSION)
            .map(|v| v.as_bytes())
            != Some(b"13")
        {
            return Err(HttpError::for_bad_request(
                None,
                "missing or invalid websocket version".to_string(),
            ));
        }

        let accept_key = request
            .headers()
            .get(header::SEC_WEBSOCKET_KEY)
            .map(|hv| hv.as_bytes())
            .map(|key| derive_accept_key(key))
            .ok_or_else(|| {
                HttpError::for_bad_request(
                    None,
                    "missing websocket key".to_string(),
                )
            })?;

        let route = request.uri().to_string();
        let upgrade_fut = hyper::upgrade::on(request);
        // note: this is just used in our wrapper in `handle`; if a user wants
        // to slog in their future, they can obtain it from rqctx the same way
        // they do in any other endpoint & let it get `move`d into the closure
        let ws_log = rqctx.log.new(o!(
            "upgrade" => "websocket".to_string(),
        ));

        Ok(Self(Some(WebsocketUpgradeInner {
            upgrade_fut,
            accept_key,
            ws_log,
            route,
        })))
    }

    fn metadata(
        _content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata {
        ExtractorMetadata {
            parameters: vec![],
            extension_mode: ExtensionMode::Websocket,
        }
    }
}

impl WebsocketUpgrade {
    /// Upgrade the HTTP connection to a websocket and spawn a user-provided
    /// async handler to service it.
    ///
    /// This function's return value should be the basis of the return value of
    /// your endpoint's function, as it sends the headers to tell the HTTP
    /// client that we are accepting the upgrade.
    ///
    /// `handler` is a closure that accepts a [`WebsocketConnection`]
    /// and returns a future that will be spawned by this function,
    /// in which the `WebsocketConnection`'s inner `Upgraded` connection may be
    /// used with your choice of websocket-handling code operating over an
    /// [`tokio::io::AsyncRead`] + [`tokio::io::AsyncWrite`] type
    /// (e.g. `tokio_tungstenite`).
    ///
    /// ```
    /// #[dropshot::endpoint { method = GET, path = "/my/ws/endpoint/{id}" }]
    /// async fn my_ws_endpoint(
    ///     rqctx: dropshot::RequestContext<()>,
    ///     id: dropshot::Path<String>,
    ///     websock: dropshot::WebsocketUpgrade,
    /// ) -> dropshot::WebsocketEndpointResult {
    ///     let logger = rqctx.log.new(slog::o!());
    ///     websock.handle(move |upgraded| async move {
    ///         slog::info!(logger, "Entered handler for ID {}", id.into_inner());
    ///         use futures::stream::StreamExt;
    ///         let mut ws_stream = tokio_tungstenite::WebSocketStream::from_raw_socket(
    ///             upgraded.into_inner(), tokio_tungstenite::tungstenite::protocol::Role::Server, None
    ///         ).await;
    ///         slog::info!(logger, "Received from websocket: {:?}", ws_stream.next().await);
    ///         Ok(())
    ///     })
    /// }
    /// ```
    ///
    /// Note that as a consumer of this crate, you most likely do not want to
    /// call this function directly; rather, prefer to annotate your function
    /// with [`dropshot_endpoint::channel`] instead of `endpoint`.
    pub fn handle<C, F>(mut self, handler: C) -> WebsocketEndpointResult
    where
        C: FnOnce(WebsocketConnection) -> F + Send + 'static,
        F: Future<Output = WebsocketChannelResult> + Send + 'static,
    {
        // we .take() here to tell Drop::drop that we handled the request.
        match self.0.take() {
            None => Err(HttpError::for_internal_error(
                "Tried to handle websocket twice".to_string(),
            )),
            Some(WebsocketUpgradeInner {
                upgrade_fut,
                accept_key,
                ws_log,
                ..
            }) => {
                tokio::spawn(async move {
                    match upgrade_fut.await {
                        Ok(upgrade) => {
                            let io = hyper_util::rt::TokioIo::new(upgrade);
                            let raw = WebsocketConnectionRaw(io);
                            match handler(WebsocketConnection(raw)).await {
                                Ok(x) => Ok(x),
                                Err(e) => {
                                    error!(
                                        ws_log,
                                        "Error returned from handler: {:?}", e
                                    );
                                    Err(e)
                                }
                            }
                        }
                        Err(e) => {
                            error!(
                                ws_log,
                                "Error upgrading connection: {:?}", e
                            );
                            Err(e.into())
                        }
                    }
                });
                Response::builder()
                    .status(StatusCode::SWITCHING_PROTOCOLS)
                    .header(header::CONNECTION, "Upgrade")
                    .header(header::UPGRADE, "websocket")
                    .header(header::SEC_WEBSOCKET_ACCEPT, accept_key)
                    .body(Body::empty())
                    .map_err(Into::into)
            }
        }
    }
}

impl Drop for WebsocketUpgrade {
    fn drop(&mut self) {
        if let Some(inner) = self.0.take() {
            debug!(
                inner.ws_log,
                "Didn't handle websocket in route {}", inner.route
            );
        }
    }
}

// To indicate websocket usage by the endpoint to code generators (i.e. Progenitor)
pub(crate) const WEBSOCKET_EXTENSION: &str = "x-dropshot-websocket";
pub(crate) const WEBSOCKET_PARAM_SENTINEL: &str = "x-dropshot-websocket-param";

impl JsonSchema for WebsocketUpgrade {
    fn schema_name() -> String {
        "WebsocketUpgrade".to_string()
    }

    fn json_schema(
        _gen: &mut schemars::gen::SchemaGenerator,
    ) -> schemars::schema::Schema {
        let mut schema = schemars::schema::SchemaObject::default();
        schema
            .extensions
            .insert(WEBSOCKET_PARAM_SENTINEL.to_string(), json!(true));
        schemars::schema::Schema::Object(schema)
    }
}

impl tokio::io::AsyncRead for WebsocketConnectionRaw {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for WebsocketConnectionRaw {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn is_write_vectored(&self) -> bool {
        self.0.is_write_vectored()
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write_vectored(cx, bufs)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use crate::body::Body;
    use crate::config::HandlerTaskMode;
    use crate::router::HttpRouter;
    use crate::server::{DropshotState, ServerConfig};
    use crate::{
        ExclusiveExtractor, HttpError, RequestContext, RequestEndpointMetadata,
        RequestInfo, VersionPolicy, WebsocketUpgrade,
    };
    use debug_ignore::DebugIgnore;
    use http::Request;
    use std::net::{IpAddr, Ipv6Addr, SocketAddr};
    use std::num::NonZeroU32;
    use std::sync::Arc;
    use std::time::Duration;
    use waitgroup::WaitGroup;

    async fn ws_upg_from_mock_rqctx() -> Result<WebsocketUpgrade, HttpError> {
        let log = slog::Logger::root(slog::Discard, slog::o!()).new(slog::o!());
        let request = Request::builder()
            .header(http::header::CONNECTION, "Upgrade")
            .header(http::header::UPGRADE, "websocket")
            .header(http::header::SEC_WEBSOCKET_VERSION, "13")
            .header(http::header::SEC_WEBSOCKET_KEY, "aGFjayB0aGUgcGxhbmV0IQ==")
            .body(Body::empty())
            .unwrap();
        let remote_addr =
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 12345);
        let rqctx = RequestContext {
            server: Arc::new(DropshotState {
                private: (),
                config: ServerConfig {
                    default_request_body_max_bytes: 0,
                    page_max_nitems: NonZeroU32::new(1).unwrap(),
                    page_default_nitems: NonZeroU32::new(1).unwrap(),
                    default_handler_task_mode:
                        HandlerTaskMode::CancelOnDisconnect,
                    log_headers: Default::default(),
                },
                router: HttpRouter::new(),
                log: log.clone(),
                local_addr: SocketAddr::new(
                    IpAddr::V6(Ipv6Addr::LOCALHOST),
                    8080,
                ),
                tls_acceptor: None,
                handler_waitgroup_worker: DebugIgnore(
                    WaitGroup::new().worker(),
                ),
                version_policy: VersionPolicy::Unversioned,
            }),
            request: RequestInfo::new(&request, remote_addr),
            endpoint: RequestEndpointMetadata {
                operation_id: "".to_string(),
                variables: Default::default(),
                body_content_type: Default::default(),
                request_body_max_bytes: None,
            },
            request_id: "".to_string(),
            log: log.clone(),
            #[cfg(feature = "otel-tracing")]
            span_context: opentelemetry::trace::SpanContext::empty_context(),
        };
        let fut = WebsocketUpgrade::from_request(&rqctx, request);
        tokio::time::timeout(Duration::from_secs(1), fut)
            .await
            .expect("Deadlocked in WebsocketUpgrade constructor")
    }

    #[tokio::test]
    async fn test_ws_upg_task_is_spawned() {
        let (send, recv) = tokio::sync::oneshot::channel();
        ws_upg_from_mock_rqctx()
            .await
            .unwrap()
            .handle(move |_upgrade| async move {
                send.send(()).unwrap();
                Ok(())
            })
            .unwrap();
        // note: not a real connection, so we don't get our future's Ok, but we *do* spawn the task
        let _ = tokio::time::timeout(Duration::from_secs(1), recv)
            .await
            .expect("Task not spawned or never completed");
    }
}
