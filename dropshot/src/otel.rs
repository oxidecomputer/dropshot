// Copyright 2025 Oxide Computer Company
//! OpenTelemetry tracing support for Dropshot HTTP servers
//!
//! This module provides OpenTelemetry integration via tracing-opentelemetry.
//! All functionality is gated behind the `otel-tracing` feature flag.

use opentelemetry::{global, trace::TracerProvider};
use opentelemetry_http::HeaderExtractor;
use std::sync::OnceLock;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

static TRACER_PROVIDER: OnceLock<opentelemetry_sdk::trace::SdkTracerProvider> =
    OnceLock::new();

/// Initialize OpenTelemetry tracing with auto-configuration from environment.
///
/// This function reads standard OpenTelemetry environment variables like:
/// - `OTEL_SERVICE_NAME`
/// - `OTEL_EXPORTER_OTLP_ENDPOINT`
/// - `OTEL_EXPORTER_OTLP_PROTOCOL`
///
/// If no OTLP endpoint is configured, traces will be printed to stdout
/// in debug builds but not in release builds.
///
/// Returns a guard that will shutdown the tracer provider when dropped.
pub async fn init_tracing(
    service_name: &str,
) -> Result<impl Drop, Box<dyn std::error::Error + Send + Sync>> {
    use opentelemetry_sdk::{
        resource::{ResourceDetector, SdkProvidedResourceDetector},
        trace::SdkTracerProvider,
    };

    // If the environment variable wasn't set, use the value
    // that was provided to this function.
    if std::env::var("OTEL_SERVICE_NAME").is_err() {
        std::env::set_var("OTEL_SERVICE_NAME", service_name);
    }
    let resource = SdkProvidedResourceDetector.detect();

    let tracer_provider =
        if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok() {
            // Use OTLP exporter if endpoint is configured
            use opentelemetry_otlp::SpanExporter;
            let exporter = SpanExporter::builder().with_http().build()?;
            SdkTracerProvider::builder()
                .with_resource(resource)
                .with_batch_exporter(exporter)
                .build()
        } else {
            // In debug builds, use stdout exporter for development/testing
            // In release builds, don't ship spans at all
            #[cfg(debug_assertions)]
            {
                use opentelemetry_stdout::SpanExporter;
                let exporter = SpanExporter::default();
                SdkTracerProvider::builder()
                    .with_resource(resource)
                    .with_simple_exporter(exporter)
                    .build()
            }
            #[cfg(not(debug_assertions))]
            {
                // No exporter for release builds - spans will be dropped
                SdkTracerProvider::builder().with_resource(resource).build()
            }
        };

    global::set_tracer_provider(tracer_provider.clone());

    // Set up W3C traceparent header propagation
    global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    // Store the provider in static for flushing
    TRACER_PROVIDER
        .set(tracer_provider.clone())
        .expect("Tracer provider already set");

    // Set up tracing-opentelemetry integration
    // We use the tracer provider directly instead of the global tracer to get proper types
    let telemetry = OpenTelemetryLayer::new(tracer_provider.tracer("dropshot"));

    // Initialize tracing subscriber with OpenTelemetry layer
    // This will work with existing slog bridge if the "tracing" feature is enabled
    if tracing::dispatcher::has_been_set() {
        // If tracing is already initialized (e.g., by slog bridge), we need to
        // reinitialize with both the existing layers and the OpenTelemetry layer.
        // We can't add layers to an existing subscriber, so we need to rebuild it.

        // Create a new subscriber with just the OpenTelemetry layer.
        // Note: This will replace the existing subscriber, which may include a slog bridge.
        // The slog logging will still work through the slog logger directly, but tracing
        // events will now go through OpenTelemetry instead of the slog bridge.
        let new_subscriber = tracing_subscriber::registry().with(telemetry);
        tracing::subscriber::set_global_default(new_subscriber).map_err(
            |e| {
                format!(
                    "Failed to reinitialize tracing with OpenTelemetry: {}",
                    e
                )
            },
        )?;
    } else {
        // Initialize fresh tracing subscriber
        tracing_subscriber::registry().with(telemetry).init();
    }

    // Return a guard that shuts down the provider when dropped
    Ok(TracingGuard { _provider: tracer_provider })
}

/// Guard that ensures proper shutdown of the tracer provider.
struct TracingGuard {
    _provider: opentelemetry_sdk::trace::SdkTracerProvider,
}

impl Drop for TracingGuard {
    fn drop(&mut self) {
        // Force shutdown of the provider to ensure all spans are exported
        if let Err(e) = self._provider.force_flush() {
            eprintln!("Failed to flush traces on shutdown: {}", e);
        }
        if let Err(e) = self._provider.shutdown() {
            eprintln!("Failed to shutdown tracer provider: {}", e);
        }
    }
}

/// Extract OpenTelemetry context from HTTP request headers.
/// This should be called at the start of request processing to establish proper trace context.
pub fn extract_context_from_request(
    request: &hyper::Request<hyper::body::Incoming>,
) -> opentelemetry::Context {
    // Extract parent context from headers using the global text map propagator
    // This handles W3C traceparent headers and other propagation formats
    global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(request.headers()))
    })
}

/// Force flush any pending spans to ensure they are exported immediately.
/// This is useful in error scenarios to ensure spans are not lost.
pub fn flush_spans() {
    if let Some(provider) = TRACER_PROVIDER.get() {
        if let Err(e) = provider.force_flush() {
            eprintln!("Failed to force flush spans: {}", e);
        }
    }
}

/// Create an HTTP request span with proper parent context inheritance
pub fn create_request_span(
    request: &hyper::Request<hyper::body::Incoming>,
    request_id: &str,
    remote_addr: std::net::SocketAddr,
) -> tracing::Span {
    let parent_context = extract_context_from_request(request);
    let guard = opentelemetry::Context::attach(parent_context);
    let span = tracing::info_span!(
        "http_request",
        http.request.method = %request.method(),
        http.request.uri = %request.uri(),
        http.request.id = %request_id,
        client.address = %remote_addr.ip(),
        client.port = remote_addr.port(),
        user_agent.original = request.headers().get("user-agent")
            .and_then(|h| h.to_str().ok())
            .unwrap_or(""),
        otel.kind = "server",
        http.response.status_code = tracing::field::Empty,
        error = tracing::field::Empty,
        error.type = tracing::field::Empty,
        error.message = tracing::field::Empty
    );
    drop(guard);
    span
}

/// Record client disconnect information on a span
pub fn record_disconnect_on_span(span: &tracing::Span) {
    span.record("http.response.status_code", 499);
    span.record("error", true);
    span.record("error.type", "client_disconnect");
    span.record(
        "error.message",
        "Client disconnected before response returned",
    );
    flush_spans();
}

/// Record error information on a span
pub fn record_error_on_span(span: &tracing::Span, status: u16, message: &str) {
    span.record("http.response.status_code", status);
    span.record("error", true);
    span.record("error.message", message);
    flush_spans();
}

/// Record successful response information on a span
pub fn record_success_on_span(span: &tracing::Span, status: u16) {
    span.record("http.response.status_code", status);
}
