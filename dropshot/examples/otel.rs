// Copyright 2024 Oxide Computer Company
//! Example use of Dropshot with OpenTelemetry integration.
//!
//! equinix-otel-tools will parse the standard otel exporter
//! environment variables, e.g.
//! If you launch an otel-collector or otel-enabled jaeger-all-in-one
//! listening for otlp over grpc, then you can do:
//!
//! ```bash
//! export OTEL_EXPORTER_OTLP_ENDPOINT=grpc://localhost:4317
//! export OTEL_EXPORTER_OTLP_INSECURE=true
//! export OTEL_EXPORTER_OTLP_PROTOCOL=grpc
//! cargo run --features=otel-tracing --example otel&
//! curl http://localhost:4000/get
//! ```
//!
//! And you should see an example trace.

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::HttpServerStarter;
use dropshot::RequestContext;
use dropshot::TypedBody;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use http_body_util::Full;
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use opentelemetry::{
    global,
    trace::{SpanKind, TraceContextExt, Tracer},
    Context,
};
use opentelemetry_http::{Bytes, HeaderInjector};

#[tokio::main]
async fn main() -> Result<(), String> {
    let _otel_guard = equinix_otel_tools::init("otel-demo");

    let config_dropshot = ConfigDropshot {
        bind_address: "127.0.0.1:4000".parse().unwrap(),
        ..Default::default()
    };

    // For simplicity, we'll configure an "info"-level logger that writes to
    // stderr assuming that it's a terminal.
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    // Build a description of the API.
    let mut api = ApiDescription::new();
    api.register(example_api_get_counter).unwrap();
    api.register(example_api_put_counter).unwrap();
    api.register(example_api_get).unwrap();
    api.register(example_api_error).unwrap();
    api.register(example_api_panic).unwrap();

    // The functions that implement our API endpoints will share this context.
    let api_context = ExampleContext::new();

    // Set up the server.
    let server =
        HttpServerStarter::new(&config_dropshot, api, api_context, &log)
            .map_err(|error| format!("failed to create server: {}", error))?
            .start();

    // Wait for the server to stop.  Note that there's not any code to shut down
    // this server, so we should never get past this point.
    let _ = server.await;
    Ok(())
}

/// Application-specific example context (state shared by handler functions)
#[derive(Debug)]
struct ExampleContext {
    /// counter that can be manipulated by requests to the HTTP API
    counter: AtomicU64,
}

impl ExampleContext {
    /// Return a new ExampleContext.
    pub fn new() -> ExampleContext {
        ExampleContext { counter: AtomicU64::new(0) }
    }
}

// HTTP API interface

/// `CounterValue` represents the value of the API's counter, either as the
/// response to a GET request to fetch the counter or as the body of a PUT
/// request to update the counter.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct CounterValue {
    counter: u64,
}

/// Helper function for propagating a traceparent using hyper
async fn traced_request(
    uri: &str,
    cx: &Context,
) -> hyper::Request<Full<Bytes>> {
    let mut req = hyper::Request::builder()
        .uri(uri)
        .method(hyper::Method::GET)
        .header("accept", "application/json")
        .header("content-type", "application/json");
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(
            &cx,
            &mut HeaderInjector(req.headers_mut().unwrap()),
        )
    });
    req.body(Full::new(Bytes::from("".to_string()))).unwrap()
}

/// Do a bunch of work to show off otel tracing
#[endpoint {
    method = GET,
    path = "/get",
}]
#[cfg_attr(feature = "tokio-tracing", tracing::instrument(skip_all))]
async fn example_api_get(
    rqctx: RequestContext<ExampleContext>,
) -> Result<HttpResponseOk<CounterValue>, HttpError> {
    let trace_context = opentelemetry::Context::new();
    let parent_context =
        opentelemetry::trace::TraceContextExt::with_remote_span_context(
            &trace_context,
            rqctx.span_context.clone(),
        );

    let client = Client::builder(TokioExecutor::new()).build_http();
    let tracer = global::tracer("");
    let span = tracer
        .span_builder(String::from("example_api_get"))
        .with_kind(SpanKind::Internal)
        .start_with_context(&tracer, &parent_context);
    let cx = Context::current_with_span(span);
    //assert!(cx.has_active_span());

    let mut req = hyper::Request::builder()
        .uri("http://localhost:4000/counter")
        .method(hyper::Method::GET)
        .header("accept", "application/json")
        .header("content-type", "application/json");
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(
            &cx,
            &mut HeaderInjector(req.headers_mut().unwrap()),
        )
    });
    let _res = client
        .request(req.body(Full::new(Bytes::from("".to_string()))).unwrap())
        .await;

    let mut req = hyper::Request::builder()
        .uri("http://localhost:4000/counter")
        .method(hyper::Method::GET)
        .header("accept", "application/json")
        .header("content-type", "application/json");
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(
            &cx,
            &mut HeaderInjector(req.headers_mut().unwrap()),
        )
    });
    let _res = client
        .request(req.body(Full::new(Bytes::from("".to_string()))).unwrap())
        .await;

    let mut req = hyper::Request::builder()
        .uri("http://localhost:4000/does-not-exist")
        .method(hyper::Method::GET)
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .header("user-agent", "dropshot-otel-example");
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(
            &cx,
            &mut HeaderInjector(req.headers_mut().unwrap()),
        )
    });
    let _res = client
        .request(req.body(Full::new(Bytes::from("".to_string()))).unwrap())
        .await;

    let req = traced_request("http://localhost:4000/error", &cx).await;
    let _res = client.request(req).await;

    let req = traced_request("http://localhost:4000/panic", &cx).await;
    let _res = client.request(req).await;

    let api_context = rqctx.context();
    Ok(HttpResponseOk(CounterValue {
        counter: api_context.counter.load(Ordering::SeqCst),
    }))
}

/// Fetch the current value of the counter.
#[endpoint {
    method = GET,
    path = "/counter",
}]
async fn example_api_get_counter(
    rqctx: RequestContext<ExampleContext>,
) -> Result<HttpResponseOk<CounterValue>, HttpError> {
    let api_context = rqctx.context();

    Ok(HttpResponseOk(CounterValue {
        counter: api_context.counter.load(Ordering::SeqCst),
    }))
}

/// Cause an error!
#[endpoint {
    method = GET,
    path = "/error",
}]
async fn example_api_error(
    _rqctx: RequestContext<ExampleContext>,
) -> Result<HttpResponseOk<CounterValue>, HttpError> {
    //XXX Why does this create a 499 rather than a 500 error???
    // This feels like a bug in dropshot. As a downstream consumer
    // I just want anything blowing up in my handler to be a somewhat useful 500 error.
    // It does help that the compiler is strict about what can otherwise be returned...
    //panic!("This handler is totally broken!");

    Err(HttpError::for_internal_error("This endpoint is broken".to_string()))
}

/// This endpoint panics!
#[endpoint {
    method = GET,
    path = "/panic",
}]
#[cfg_attr(feature = "tokio-tracing", tracing::instrument(skip_all, err))]
async fn example_api_panic(
    _rqctx: RequestContext<ExampleContext>,
) -> Result<HttpResponseOk<CounterValue>, HttpError> {
    //XXX Why does this create a 499 rather than a 500 error???
    // This feels like a bug in dropshot. As a downstream consumer
    // I just want anything blowing up in my handler to be a somewhat useful 500 error.
    // It does help that the compiler is strict about what can otherwise be returned...
    panic!("This handler is totally broken!");
}

/// Update the current value of the counter.  Note that the special value of 10
/// is not allowed (just to demonstrate how to generate an error).
#[endpoint {
    method = PUT,
    path = "/counter",
}]
#[cfg_attr(feature = "tokio-tracing", tracing::instrument(skip_all, err))]
async fn example_api_put_counter(
    rqctx: RequestContext<ExampleContext>,
    update: TypedBody<CounterValue>,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    let api_context = rqctx.context();
    let updated_value = update.into_inner();

    if updated_value.counter == 10 {
        Err(HttpError::for_bad_request(
            Some(String::from("BadInput")),
            format!("do not like the number {}", updated_value.counter),
        ))
    } else {
        api_context.counter.store(updated_value.counter, Ordering::SeqCst);
        Ok(HttpResponseUpdatedNoContent())
    }
}
