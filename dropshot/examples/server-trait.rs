// Copyright 2023 Oxide Computer Company

//! An example demonstrating use of the `dropshot_server` attribute macro to
//! define an endpoint.

use std::sync::atomic::{AtomicU64, Ordering};

use dropshot::{
    ConfigLogging, ConfigLoggingLevel, HttpError, HttpResponseOk,
    HttpResponseUpdatedNoContent, HttpServerStarter, RequestContext, TypedBody,
};
use dropshot_endpoint::dropshot_server;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[tokio::main]
async fn main() -> Result<(), String> {
    let config_dropshot = Default::default();
    // For simplicity, we'll configure an "info"-level logger that writes to
    // stderr assuming that it's a terminal.
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    // XXX: The `dropshot_server` attribute macro conjures up this
    // to_api_description method. Consider making this better somehow. (How? Any
    // trait-based attempts run into Rust's orphan rules).
    let my_server = CounterServer_api_description::<CounterImpl>().unwrap();
    let server = HttpServerStarter::new(
        &config_dropshot,
        my_server,
        CounterImpl::new(),
        &log,
    )
    .map_err(|error| format!("failed to create server: {}", error))?
    .start();

    server.await
}

#[derive(Deserialize, Serialize, JsonSchema)]
struct CounterValue {
    counter: u64,
}

#[derive(Deserialize, Serialize, JsonSchema)]
struct MultiplyAndAddPath {
    counter: u64,
}

#[dropshot_server]
trait CounterServer {
    /// By default, the name of the context type is Context. To specify a
    /// different name, use the { context = ... } attribute on
    /// `#[dropshot_server]`.
    type Context;

    #[endpoint { method = GET, path = "/counter" }]
    async fn get_counter(
        rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<CounterValue>, HttpError>;

    #[endpoint { method = PUT, path = "/counter" }]
    async fn put_counter(
        rqctx: RequestContext<Self::Context>,
        update: TypedBody<CounterValue>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;

    /// A miscellaneous function that is left untouched by the `dropshot_server`
    /// attribute macro.
    fn helper(&self) -> u64;
}

struct CounterImpl {
    counter: AtomicU64,
}

impl CounterImpl {
    pub fn new() -> CounterImpl {
        CounterImpl { counter: AtomicU64::new(0) }
    }
}

impl CounterServer for CounterImpl {
    type Context = Self;

    async fn get_counter(
        rqctx: RequestContext<Self>,
    ) -> Result<HttpResponseOk<CounterValue>, HttpError> {
        let self_ = rqctx.context();
        Ok(HttpResponseOk(CounterValue { counter: self_.helper() }))
    }

    async fn put_counter(
        rqctx: RequestContext<Self>,
        update: TypedBody<CounterValue>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        let self_ = rqctx.context();
        let updated_value = update.into_inner();

        if updated_value.counter == 10 {
            Err(HttpError::for_bad_request(
                Some(String::from("BadInput")),
                format!("do not like the number {}", updated_value.counter),
            ))
        } else {
            self_.counter.store(updated_value.counter, Ordering::SeqCst);
            Ok(HttpResponseUpdatedNoContent())
        }
    }

    fn helper(&self) -> u64 {
        self.counter.load(Ordering::Relaxed)
    }
}
