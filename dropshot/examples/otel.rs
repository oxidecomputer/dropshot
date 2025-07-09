// Copyright 2025 Oxide Computer Company
//! Example use of Dropshot with OpenTelemetry integration.
//!
//! Dropshot's built-in OpenTelemetry support will automatically parse
//! standard OTEL environment variables.
//! If you launch an otel-collector or otel-enabled jaeger-all-in-one
//! listening for otlp over http, then you can do:
//!
//! ```bash
//! export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
//! export OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf
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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(any(feature = "tracing", feature = "otel-tracing"))]
use tracing;

#[tokio::main]
async fn main() -> Result<(), String> {
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

    // Initialize tracing with both slog bridge and OpenTelemetry support
    #[cfg(any(feature = "tracing", feature = "otel-tracing"))]
    let _tracing_guard = dropshot::tracing_support::init_tracing(&log)
        .await
        .map_err(|e| format!("failed to initialize tracing: {}", e))?;

    // Build a description of the API.
    let mut api = ApiDescription::new();
    api.register(example_api_get_counter).unwrap();
    api.register(example_api_put_counter).unwrap();
    api.register(example_api_get).unwrap();
    api.register(example_api_error).unwrap();
    api.register(example_api_panic).unwrap();
    api.register(example_api_sleep).unwrap();
    api.register(example_api_exit).unwrap();

    // The functions that implement our API endpoints will share this context.
    let api_context = ExampleContext::new();

    // Set up the server.
    let server =
        HttpServerStarter::new(&config_dropshot, api, api_context, &log)
            .map_err(|error| format!("failed to create server: {}", error))?
            .start();

    let shutdown = server.wait_for_shutdown();

    tokio::task::spawn(async move {
        loop {
            if server.app_private().shutdown.load(Ordering::SeqCst) {
                break;
            } else {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }
        server.close().await.unwrap();
    });

    // From a separate task, wait for the server to stop.
    shutdown.await
}

/// Application-specific example context (state shared by handler functions)
#[derive(Debug)]
struct ExampleContext {
    /// counter that can be manipulated by requests to the HTTP API
    counter: AtomicU64,
    shutdown: AtomicBool,
}

impl ExampleContext {
    /// Return a new ExampleContext.
    pub fn new() -> ExampleContext {
        ExampleContext {
            counter: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
        }
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

/// Demonstrates creating child spans for internal operations using tracing instrumentation
#[endpoint {
    method = GET,
    path = "/get",
}]
#[cfg_attr(
any(feature = "tracing", feature = "otel-tracing"),
tracing::instrument(skip(rqctx), fields(counter_processing = tracing::field::Empty)))]
async fn example_api_get(
    rqctx: RequestContext<ExampleContext>,
) -> Result<HttpResponseOk<CounterValue>, HttpError> {
    #[cfg(any(feature = "tracing", feature = "otel-tracing"))]
    tracing::info!("Starting counter fetch with processing");

    // Simulate some work
    fetch_counter_with_delay().await;

    let api_context = rqctx.context();
    let counter_value = api_context.counter.load(Ordering::SeqCst);

    // Do some "processing" that would benefit from being traced
    let processed_value = process_counter_value(counter_value).await;

    // Record the processing result in the span
    #[cfg(any(feature = "tracing", feature = "otel-tracing"))]
    tracing::Span::current().record("counter_processing", processed_value);

    #[cfg(any(feature = "tracing", feature = "otel-tracing"))]
    tracing::info!(
        processed_value = processed_value,
        "Counter processing completed"
    );

    Ok(HttpResponseOk(CounterValue { counter: processed_value }))
}

#[cfg_attr(
    any(feature = "tracing", feature = "otel-tracing"),
    tracing::instrument
)]
async fn fetch_counter_with_delay() {
    #[cfg(any(feature = "tracing", feature = "otel-tracing"))]
    tracing::debug!("Simulating work");
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
}

#[cfg_attr(
    any(feature = "tracing", feature = "otel-tracing"),
    tracing::instrument
)]
async fn process_counter_value(counter_value: u64) -> u64 {
    #[cfg(any(feature = "tracing", feature = "otel-tracing"))]
    tracing::debug!(input_value = counter_value, "Processing counter value");
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let result = counter_value * 2; // Some arbitrary processing
    #[cfg(any(feature = "tracing", feature = "otel-tracing"))]
    tracing::debug!(output_value = result, "Counter processing complete");
    result
}

/// Fetch the current value of the counter.
#[endpoint {
    method = GET,
    path = "/counter",
}]
#[cfg_attr(
    any(feature = "tracing", feature = "otel-tracing"),
    tracing::instrument(skip(rqctx))
)]
async fn example_api_get_counter(
    rqctx: RequestContext<ExampleContext>,
) -> Result<HttpResponseOk<CounterValue>, HttpError> {
    let api_context = rqctx.context();
    let counter = api_context.counter.load(Ordering::SeqCst);

    #[cfg(any(feature = "tracing", feature = "otel-tracing"))]
    tracing::info!(counter_value = counter, "Retrieved counter value");

    Ok(HttpResponseOk(CounterValue { counter }))
}

/// Demonstrates error tracing - errors will be marked on the span
#[endpoint {
    method = GET,
    path = "/error",
}]
#[cfg_attr(
    any(feature = "tracing", feature = "otel-tracing"),
    tracing::instrument(skip(_rqctx))
)]
async fn example_api_error(
    _rqctx: RequestContext<ExampleContext>,
) -> Result<HttpResponseOk<CounterValue>, HttpError> {
    #[cfg(any(feature = "tracing", feature = "otel-tracing"))]
    tracing::warn!("About to return an error for demonstration");
    let error =
        HttpError::for_internal_error("This endpoint is broken".to_string());
    #[cfg(any(feature = "tracing", feature = "otel-tracing"))]
    tracing::error!(error = ?error, "Returning demonstration error");
    Err(error)
}

/// Demonstrates panic handling - panics are converted to 500 errors and traced
#[endpoint {
    method = GET,
    path = "/panic",
}]
#[cfg_attr(
    any(feature = "tracing", feature = "otel-tracing"),
    tracing::instrument(skip(_rqctx))
)]
async fn example_api_panic(
    _rqctx: RequestContext<ExampleContext>,
) -> Result<HttpResponseOk<CounterValue>, HttpError> {
    panic!("This handler panics to demonstrate error tracing");
}

/// Takes too long so the client disconnects
#[endpoint {
    method = GET,
    path = "/sleep",
}]
#[cfg_attr(
    any(feature = "tracing", feature = "otel-tracing"),
    tracing::instrument(skip(_rqctx))
)]
async fn example_api_sleep(
    _rqctx: RequestContext<ExampleContext>,
) -> Result<HttpResponseOk<CounterValue>, HttpError> {
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    Err(HttpError::for_internal_error(
        "This endpoint takes too long".to_string(),
    ))
}

/// Exit shortcut
#[endpoint {
    method = GET,
    path = "/exit",
}]
#[cfg_attr(
    any(feature = "tracing", feature = "otel-tracing"),
    tracing::instrument(skip(rqctx))
)]
async fn example_api_exit(
    rqctx: RequestContext<ExampleContext>,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    rqctx.context().shutdown.store(true, Ordering::SeqCst);
    Ok(HttpResponseUpdatedNoContent())
}

/// Update the current value of the counter.  Note that the special value of 10
/// is not allowed (just to demonstrate how to generate an error).
#[endpoint {
    method = PUT,
    path = "/counter",
}]
#[cfg_attr(
any(feature = "tracing", feature = "otel-tracing"),
tracing::instrument(skip(rqctx, update), fields(new_value = tracing::field::Empty)))]
async fn example_api_put_counter(
    rqctx: RequestContext<ExampleContext>,
    update: TypedBody<CounterValue>,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    let api_context = rqctx.context();
    let updated_value = update.into_inner();

    // Record the new value in the span
    #[cfg(any(feature = "tracing", feature = "otel-tracing"))]
    tracing::Span::current().record("new_value", updated_value.counter);
    #[cfg(any(feature = "tracing", feature = "otel-tracing"))]
    tracing::info!(
        new_counter_value = updated_value.counter,
        "Updating counter"
    );

    if updated_value.counter == 10 {
        #[cfg(any(feature = "tracing", feature = "otel-tracing"))]
        tracing::warn!(
            rejected_value = updated_value.counter,
            "Rejecting forbidden value"
        );
        Err(HttpError::for_bad_request(
            Some(String::from("BadInput")),
            format!("do not like the number {}", updated_value.counter),
        ))
    } else {
        api_context.counter.store(updated_value.counter, Ordering::SeqCst);
        #[cfg(any(feature = "tracing", feature = "otel-tracing"))]
        tracing::info!(
            updated_counter = updated_value.counter,
            "Counter updated successfully"
        );
        Ok(HttpResponseUpdatedNoContent())
    }
}
