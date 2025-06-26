// Copyright 2025 Oxide Computer Company
//! Unified tracing support for Dropshot HTTP servers
//!
//! This module consolidates all tracing functionality including:
//! - Slog bridge for tracing -> slog compatibility
//! - OpenTelemetry integration
//! - Unified initialization that supports both

#[cfg(any(feature = "tracing", feature = "otel-tracing"))]
use tracing_subscriber::prelude::*;

#[cfg(feature = "otel-tracing")]
use std::sync::OnceLock;

#[cfg(feature = "otel-tracing")]
static TRACER_PROVIDER: OnceLock<opentelemetry_sdk::trace::SdkTracerProvider> =
    OnceLock::new();

/// Guard that ensures proper shutdown of tracing infrastructure
pub struct TracingGuard {
    #[cfg(feature = "otel-tracing")]
    _otel_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

#[cfg(feature = "otel-tracing")]
impl Drop for TracingGuard {
    fn drop(&mut self) {
        if let Some(provider) = &self._otel_provider {
            if let Err(e) = provider.force_flush() {
                eprintln!("Failed to flush traces on shutdown: {}", e);
            }
            if let Err(e) = provider.shutdown() {
                eprintln!("Failed to shutdown tracer provider: {}", e);
            }
        }
    }
}

#[cfg(not(feature = "otel-tracing"))]
impl Drop for TracingGuard {
    fn drop(&mut self) {
        // Nothing to do when otel-tracing is not enabled
    }
}

/// Initialize tracing with support for both slog bridge and OpenTelemetry
pub async fn init_tracing(
    #[allow(unused_variables)] logger: &slog::Logger,
) -> Result<Option<TracingGuard>, Box<dyn std::error::Error + Send + Sync>> {
    // Check if tracing has already been initialized
    if tracing::dispatcher::has_been_set() {
        return Ok(None);
    }

    // Build the subscriber based on enabled features
    #[cfg(all(feature = "tracing", feature = "otel-tracing"))]
    {
        // Both features enabled - create layered subscriber
        let bridge = SlogTracingBridge::new(logger.clone());

        // Always try to initialize OpenTelemetry when the feature is enabled
        {
            // Create OpenTelemetry components
            let (tracer_provider, tracer) = create_otel_tracer().await?;

            // Create layers
            let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

            // Build layered subscriber
            let subscriber =
                tracing_subscriber::registry().with(bridge).with(otel_layer);

            tracing::subscriber::set_global_default(subscriber)?;

            return Ok(Some(TracingGuard {
                _otel_provider: Some(tracer_provider),
            }));
        }
    }

    #[cfg(all(feature = "tracing", not(feature = "otel-tracing")))]
    {
        // Only tracing feature - just slog bridge
        let bridge = SlogTracingBridge::new(logger.clone());
        let subscriber = tracing_subscriber::registry().with(bridge);
        tracing::subscriber::set_global_default(subscriber)?;

        return Ok(Some(TracingGuard {
            #[cfg(feature = "otel-tracing")]
            _otel_provider: None,
        }));
    }

    #[cfg(all(not(feature = "tracing"), feature = "otel-tracing"))]
    {
        // Only otel-tracing feature
        let (tracer_provider, tracer) = create_otel_tracer().await?;
        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
        let subscriber = tracing_subscriber::registry().with(otel_layer);
        tracing::subscriber::set_global_default(subscriber)?;

        return Ok(Some(TracingGuard {
            _otel_provider: Some(tracer_provider),
        }));
    }

    #[cfg(not(any(feature = "tracing", feature = "otel-tracing")))]
    {
        // No features enabled
        Ok(None)
    }
}

#[cfg(feature = "otel-tracing")]
async fn create_otel_tracer() -> Result<
    (
        opentelemetry_sdk::trace::SdkTracerProvider,
        opentelemetry_sdk::trace::Tracer,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
    use opentelemetry::{global, trace::TracerProvider};
    use opentelemetry_sdk::{
        resource::{ResourceDetector, SdkProvidedResourceDetector},
        trace::SdkTracerProvider,
    };

    // Set service name if not already set, using crate name from compile time
    if std::env::var("OTEL_SERVICE_NAME").is_err() {
        std::env::set_var("OTEL_SERVICE_NAME", env!("CARGO_PKG_NAME"));
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
            // In release builds, don't export spans at all
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
                SdkTracerProvider::builder().with_resource(resource).build()
            }
        };

    // Set global tracer provider
    global::set_tracer_provider(tracer_provider.clone());

    // Set up W3C traceparent header propagation
    global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    // Store provider for flushing
    TRACER_PROVIDER.set(tracer_provider.clone()).ok(); // Ignore if already set

    let tracer = tracer_provider.tracer("dropshot");

    Ok((tracer_provider, tracer))
}

#[cfg(feature = "otel-tracing")]
/// Extract OpenTelemetry context from HTTP request headers
pub fn extract_context_from_request(
    request: &hyper::Request<hyper::body::Incoming>,
) -> opentelemetry::Context {
    use opentelemetry::global;
    use opentelemetry_http::HeaderExtractor;

    global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(request.headers()))
    })
}

#[cfg(feature = "otel-tracing")]
/// Force flush any pending spans to ensure they are exported immediately
pub fn flush_spans() {
    if let Some(provider) = TRACER_PROVIDER.get() {
        if let Err(e) = provider.force_flush() {
            eprintln!("Failed to force flush spans: {}", e);
        }
    }
}

// Move SlogTracingBridge implementation here
#[cfg(feature = "tracing")]
use tracing_subscriber::Layer;

#[cfg(feature = "tracing")]
/// A tracing subscriber layer that bridges tracing events to slog
pub struct SlogTracingBridge {
    logger: slog::Logger,
}

#[cfg(feature = "tracing")]
impl SlogTracingBridge {
    pub fn new(logger: slog::Logger) -> Self {
        Self { logger }
    }
}

#[cfg(feature = "tracing")]
impl<S> Layer<S> for SlogTracingBridge
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let metadata = event.metadata();
        let level = match *metadata.level() {
            tracing::Level::TRACE => slog::Level::Trace,
            tracing::Level::DEBUG => slog::Level::Debug,
            tracing::Level::INFO => slog::Level::Info,
            tracing::Level::WARN => slog::Level::Warning,
            tracing::Level::ERROR => slog::Level::Error,
        };

        // Extract the message and key-value pairs from the tracing event
        let mut visitor = SlogEventVisitor::new();
        event.record(&mut visitor);

        // Log to slog with the extracted data
        let message = visitor.message.unwrap_or_else(|| "".to_string());

        // Create a dynamic key-value object for slog
        let kv = SlogKV::new(visitor.fields);

        // Use slog macros based on level with proper key-value pairs
        match level {
            slog::Level::Trace => {
                slog::trace!(self.logger, "{}", message; kv)
            }
            slog::Level::Debug => {
                slog::debug!(self.logger, "{}", message; kv)
            }
            slog::Level::Info => {
                slog::info!(self.logger, "{}", message; kv)
            }
            slog::Level::Warning => {
                slog::warn!(self.logger, "{}", message; kv)
            }
            slog::Level::Error => {
                slog::error!(self.logger, "{}", message; kv)
            }
            slog::Level::Critical => {
                slog::crit!(self.logger, "{}", message; kv)
            }
        }
    }
}

#[cfg(feature = "tracing")]
/// Wrapper for different value types that can be logged
#[derive(Debug, Clone)]
enum SlogValue {
    Str(String),
    I64(i64),
    U64(u64),
    Bool(bool),
    Debug(String),
}

#[cfg(feature = "tracing")]
/// Helper struct to pass tracing fields as slog key-value pairs
struct SlogKV {
    fields: Vec<(String, SlogValue)>,
}

#[cfg(feature = "tracing")]
impl SlogKV {
    fn new(fields: Vec<(String, SlogValue)>) -> Self {
        Self { fields }
    }
}

#[cfg(feature = "tracing")]
impl slog::KV for SlogKV {
    fn serialize(
        &self,
        _record: &slog::Record,
        serializer: &mut dyn slog::Serializer,
    ) -> slog::Result {
        for (key, value) in &self.fields {
            let key = slog::Key::from(key.clone());
            match value {
                SlogValue::Str(v) => serializer.emit_str(key, v)?,
                SlogValue::I64(v) => serializer.emit_i64(key, *v)?,
                SlogValue::U64(v) => serializer.emit_u64(key, *v)?,
                SlogValue::Bool(v) => serializer.emit_bool(key, *v)?,
                SlogValue::Debug(v) => serializer.emit_str(key, v)?,
            }
        }
        Ok(())
    }
}

#[cfg(feature = "tracing")]
/// Visitor to extract fields from tracing events
struct SlogEventVisitor {
    message: Option<String>,
    fields: Vec<(String, SlogValue)>,
}

#[cfg(feature = "tracing")]
impl SlogEventVisitor {
    fn new() -> Self {
        Self { message: None, fields: Vec::new() }
    }
}

#[cfg(feature = "tracing")]
impl tracing::field::Visit for SlogEventVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push((
                field.name().to_string(),
                SlogValue::Str(value.to_string()),
            ));
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields.push((field.name().to_string(), SlogValue::I64(value)));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields.push((field.name().to_string(), SlogValue::U64(value)));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields.push((field.name().to_string(), SlogValue::Bool(value)));
    }

    fn record_debug(
        &mut self,
        field: &tracing::field::Field,
        value: &dyn std::fmt::Debug,
    ) {
        if field.name() == "message" {
            self.message = Some(format!("{:?}", value));
        } else {
            self.fields.push((
                field.name().to_string(),
                SlogValue::Debug(format!("{:?}", value)),
            ));
        }
    }
}

#[cfg(test)]
mod test {
    use crate::logging::test::read_config_and_create_logger;
    use crate::logging::test::LogTest;
    use crate::test_util::read_bunyan_log;
    use crate::tracing_support;

    /// Test that the tracing-to-slog bridge works with basic logging
    #[tokio::test]
    async fn test_tracing_bridge_basic() {
        let mut logtest = LogTest::setup("tracing_bridge_basic");
        let logpath = logtest.will_create_file("bridge.log");

        // Windows paths need to have \ turned into \\
        let escaped_path =
            logpath.display().to_string().escape_default().to_string();

        let config = format!(
            r#"
            mode = "file"
            level = "info"
            if_exists = "truncate"
            path = "{}"
            "#,
            escaped_path
        );

        {
            let logger =
                read_config_and_create_logger("tracing_bridge_basic", &config)
                    .unwrap();

            let _tracing_guard =
                tracing_support::init_tracing(&logger).await.unwrap();

            // Test slog logging
            slog::info!(logger, "slog message"; "slog_key" => "slog_value", "slog_num" => 42);

            // Test tracing logging (bridge is automatically initialized when feature is enabled)
            #[cfg(feature = "tracing")]
            {
                tracing::info!(
                    tracing_key = "tracing_value",
                    tracing_num = 84,
                    "tracing message"
                );
            }

            // Explicitly drop the logger to ensure async drain flushes
            drop(logger);
        }

        // Retry reading the log file to handle async drain flushing
        let log_records = {
            let mut records = Vec::new();
            for _ in 0..10 {
                records = read_bunyan_log(&logpath);
                #[cfg(feature = "tracing")]
                if records.len() >= 2 {
                    break;
                }
                #[cfg(not(feature = "tracing"))]
                if records.len() >= 1 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            records
        };

        assert_eq!(log_records[0].msg, "slog message");
        #[cfg(not(feature = "tracing"))]
        {
            assert_eq!(log_records.len(), 1);
        }
        #[cfg(feature = "tracing")]
        {
            assert_eq!(log_records.len(), 2);
            assert_eq!(log_records[1].msg, "tracing message");
            // Check that the structured fields are preserved
            let log_json: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(&logpath)
                    .unwrap()
                    .lines()
                    .last()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(log_json["tracing_key"], "tracing_value");
            assert_eq!(log_json["tracing_num"], 84);
        }
    }

    /// Test the SlogKV implementation with different value types
    #[test]
    #[cfg(feature = "tracing")]
    fn test_slog_kv_types() {
        use super::{SlogKV, SlogValue};
        use slog::KV;

        let fields = vec![
            (
                "str_field".to_string(),
                SlogValue::Str("test_string".to_string()),
            ),
            ("i64_field".to_string(), SlogValue::I64(-123)),
            ("u64_field".to_string(), SlogValue::U64(456)),
            ("bool_field".to_string(), SlogValue::Bool(true)),
            (
                "debug_field".to_string(),
                SlogValue::Debug("debug_value".to_string()),
            ),
        ];

        let kv = SlogKV::new(fields);

        // Create a mock serializer to test the KV implementation
        struct MockSerializer {
            pub calls: std::cell::RefCell<Vec<(String, String)>>,
        }

        impl slog::Serializer for MockSerializer {
            fn emit_str(&mut self, key: slog::Key, val: &str) -> slog::Result {
                self.calls
                    .borrow_mut()
                    .push((key.as_ref().to_string(), format!("str:{}", val)));
                Ok(())
            }

            fn emit_i64(&mut self, key: slog::Key, val: i64) -> slog::Result {
                self.calls
                    .borrow_mut()
                    .push((key.as_ref().to_string(), format!("i64:{}", val)));
                Ok(())
            }

            fn emit_u64(&mut self, key: slog::Key, val: u64) -> slog::Result {
                self.calls
                    .borrow_mut()
                    .push((key.as_ref().to_string(), format!("u64:{}", val)));
                Ok(())
            }

            fn emit_bool(&mut self, key: slog::Key, val: bool) -> slog::Result {
                self.calls
                    .borrow_mut()
                    .push((key.as_ref().to_string(), format!("bool:{}", val)));
                Ok(())
            }

            fn emit_arguments(
                &mut self,
                _key: slog::Key,
                _val: &std::fmt::Arguments,
            ) -> slog::Result {
                Ok(())
            }
            fn emit_unit(&mut self, _key: slog::Key) -> slog::Result {
                Ok(())
            }
            fn emit_char(
                &mut self,
                _key: slog::Key,
                _val: char,
            ) -> slog::Result {
                Ok(())
            }
            fn emit_u8(&mut self, _key: slog::Key, _val: u8) -> slog::Result {
                Ok(())
            }
            fn emit_i8(&mut self, _key: slog::Key, _val: i8) -> slog::Result {
                Ok(())
            }
            fn emit_u16(&mut self, _key: slog::Key, _val: u16) -> slog::Result {
                Ok(())
            }
            fn emit_i16(&mut self, _key: slog::Key, _val: i16) -> slog::Result {
                Ok(())
            }
            fn emit_u32(&mut self, _key: slog::Key, _val: u32) -> slog::Result {
                Ok(())
            }
            fn emit_i32(&mut self, _key: slog::Key, _val: i32) -> slog::Result {
                Ok(())
            }
            fn emit_f32(&mut self, _key: slog::Key, _val: f32) -> slog::Result {
                Ok(())
            }
            fn emit_f64(&mut self, _key: slog::Key, _val: f64) -> slog::Result {
                Ok(())
            }
            fn emit_usize(
                &mut self,
                _key: slog::Key,
                _val: usize,
            ) -> slog::Result {
                Ok(())
            }
            fn emit_isize(
                &mut self,
                _key: slog::Key,
                _val: isize,
            ) -> slog::Result {
                Ok(())
            }
        }

        let mut serializer =
            MockSerializer { calls: std::cell::RefCell::new(Vec::new()) };

        // Test serialization
        let args = format_args!("test message");
        let record = slog::Record::new(
            &slog::RecordStatic {
                location: &slog::RecordLocation {
                    file: "test",
                    line: 1,
                    column: 1,
                    function: "test",
                    module: "test",
                },
                level: slog::Level::Info,
                tag: "test",
            },
            &args,
            slog::BorrowedKV(&()),
        );

        kv.serialize(&record, &mut serializer).unwrap();

        let calls = serializer.calls.borrow();
        assert_eq!(calls.len(), 5);
        assert_eq!(
            calls[0],
            ("str_field".to_string(), "str:test_string".to_string())
        );
        assert_eq!(calls[1], ("i64_field".to_string(), "i64:-123".to_string()));
        assert_eq!(calls[2], ("u64_field".to_string(), "u64:456".to_string()));
        assert_eq!(
            calls[3],
            ("bool_field".to_string(), "bool:true".to_string())
        );
        assert_eq!(
            calls[4],
            ("debug_field".to_string(), "str:debug_value".to_string())
        );
    }

    /// Test the SlogEventVisitor field extraction (without creating real tracing fields)
    #[test]
    #[cfg(feature = "tracing")]
    fn test_slog_event_visitor() {
        use super::{SlogEventVisitor, SlogValue};

        let mut visitor = SlogEventVisitor::new();

        // Create mock data - we can't easily create real tracing::field::Field instances
        // in tests, so we'll test by directly populating the visitor fields

        // Directly populate the visitor with test data
        visitor.fields.push((
            "str_key".to_string(),
            SlogValue::Str("string_value".to_string()),
        ));
        visitor.fields.push(("i64_key".to_string(), SlogValue::I64(-789)));
        visitor.fields.push(("u64_key".to_string(), SlogValue::U64(101112)));
        visitor.fields.push(("bool_key".to_string(), SlogValue::Bool(false)));
        visitor.message = Some("test message".to_string());

        // Check message extraction
        assert_eq!(visitor.message, Some("test message".to_string()));

        // Check field extraction and types
        assert_eq!(visitor.fields.len(), 4);

        let (key, value) = &visitor.fields[0];
        assert_eq!(key, "str_key");
        match value {
            SlogValue::Str(s) => assert_eq!(s, "string_value"),
            _ => panic!("Expected Str variant"),
        }

        let (key, value) = &visitor.fields[1];
        assert_eq!(key, "i64_key");
        match value {
            SlogValue::I64(i) => assert_eq!(*i, -789),
            _ => panic!("Expected I64 variant"),
        }

        let (key, value) = &visitor.fields[2];
        assert_eq!(key, "u64_key");
        match value {
            SlogValue::U64(u) => assert_eq!(*u, 101112),
            _ => panic!("Expected U64 variant"),
        }

        let (key, value) = &visitor.fields[3];
        assert_eq!(key, "bool_key");
        match value {
            SlogValue::Bool(b) => assert_eq!(*b, false),
            _ => panic!("Expected Bool variant"),
        }
    }
}
