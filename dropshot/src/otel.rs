// Copyright 2024 Oxide Computer Company
//! Opentelemetry tracing support
//!
// XXX Not sure if we want to just mimic reqwest-tracing
// or not...
//! Fields that we want to produce to provide comparable
//! functionality to [reqwest-tracing]:
//!
//! - [ ] error.cause_chain
//! - [ ] error.message
//! - [x] http.request.method
//! - [x] http.response.status_code
//! - [x] otel.kind (showing up as span.kind?)
//! - [x] otel.name
//! - [ ] otel.status_code (needs investigation)
//! - [x] server.address
//! - [x] server.port
//! - [ ] url.scheme
//! - [x] user_agent.original
//!
//! Where possible we use the [opentelemetry-semantic-conventions] crate for naming.
//!
//! [reqwest-tracing]: https://docs.rs/reqwest-tracing/0.5.4/reqwest_tracing/macro.reqwest_otel_span.html
//! [opentelemetry-semantic-conventions]:
//! <https://docs.rs/opentelemetry-semantic-conventions/latest/opentelemetry_semantic_conventions/trace/index.html>

use opentelemetry::{
    global, trace::Span, trace::Tracer, trace::TracerProvider,
};
use opentelemetry_http::HeaderExtractor;
use opentelemetry_semantic_conventions::trace;

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct RequestInfo {
    pub id: String,
    pub local_addr: std::net::SocketAddr,
    pub remote_addr: std::net::SocketAddr,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub user_agent: String,
}

pub(crate) struct ResponseInfo<'a> {
    pub status_code: u16,
    pub message: String,
    pub error: Option<&'a crate::handler::HandlerError>,
}

/// Generate an opentelementry Context based on the headers
/// in the incoming hyper request.
fn extract_context_from_request(
    request: &hyper::Request<hyper::body::Incoming>,
) -> opentelemetry::Context {
    global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(request.headers()))
    })
}

/// Create an opentelemetry Span to represent the handling of the incoming request.
pub fn create_request_span(
    request: &hyper::Request<hyper::body::Incoming>,
) -> opentelemetry::global::BoxedSpan {
    let tracer_provider = global::tracer_provider();
    let scope =
        opentelemetry::InstrumentationScope::builder("dropshot_tracing")
            .with_version(env!("CARGO_PKG_VERSION"))
            .with_schema_url("https://opentelemetry.io/schemas/1.17.0")
            .build();
    let tracer = tracer_provider.tracer_with_scope(scope);
    let parent_cx = extract_context_from_request(&request);
    tracer
        .span_builder("dropshot_request") // TODO? Naming is hard.
        .with_kind(opentelemetry::trace::SpanKind::Server)
        .start_with_context(&tracer, &parent_cx)
}

/// This trait contains all functionality needed for dropshot to annotate
/// opentelemetry spans with information about a request and the corresponding response.
pub trait TraceDropshot {
    /// Attach attributes to the span based on the provided
    /// RequestInfo
    fn trace_request(&mut self, request: RequestInfo);
    /// Attach the information from the ResponseInfo to the span
    /// including marking the span as an error if an error occurred.
    fn trace_response(&mut self, response: ResponseInfo);
}

impl TraceDropshot for opentelemetry::global::BoxedSpan {
    fn trace_request(&mut self, request: RequestInfo) {
        self.set_attributes(vec![
            // Rename to dropshot.id ????
            opentelemetry::KeyValue::new("http.id".to_string(), request.id),
            opentelemetry::KeyValue::new(trace::URL_PATH, request.path),
            opentelemetry::KeyValue::new(
                trace::HTTP_REQUEST_METHOD,
                request.method,
            ),
            opentelemetry::KeyValue::new(
                trace::SERVER_ADDRESS,
                request.local_addr.ip().to_string(),
            ),
            opentelemetry::KeyValue::new(
                trace::SERVER_PORT,
                request.local_addr.port().to_string(),
            ),
            opentelemetry::KeyValue::new(
                trace::USER_AGENT_ORIGINAL,
                request.user_agent,
            ),
        ]);
    }

    /// TODO: Do we want to differentiate between 4xx vs 5xx errors
    /// and not mark 4xx spans as error spans?
    fn trace_response(&mut self, response: ResponseInfo) {
        self.set_attributes(vec![
            opentelemetry::KeyValue::new(
                trace::HTTP_RESPONSE_STATUS_CODE,
                i64::from(response.status_code),
            ),
            opentelemetry::KeyValue::new(
                "http.message".to_string(),
                response.message,
            ),
        ]);
        if let Some(error) = response.error {
            self.set_status(opentelemetry::trace::Status::error(
                error.internal_message().clone(),
            ));
        }
    }
}
