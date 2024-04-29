// Copyright 2023 Oxide Computer Company

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
    let my_server = MyServer_to_api_description::<MyImpl>().unwrap();
    let server = HttpServerStarter::new(
        &config_dropshot,
        my_server,
        MyImpl::new(),
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

#[dropshot_server]
trait MyServer: Send + Sync + Sized + 'static {
    type ExtraType;

    fn helper(&self) -> u64;

    #[endpoint { method = GET, path = "/counter" }]
    async fn get_counter(
        rqctx: RequestContext<Self>,
    ) -> Result<HttpResponseOk<CounterValue>, HttpError>;

    #[endpoint { method = PUT, path = "/counter" }]
    async fn put_counter(
        rqctx: RequestContext<Self>,
        update: TypedBody<CounterValue>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

struct MyImpl {
    counter: AtomicU64,
}

impl MyImpl {
    pub fn new() -> MyImpl {
        MyImpl { counter: AtomicU64::new(0) }
    }
}

#[async_trait::async_trait]
impl MyServer for MyImpl {
    type ExtraType = ();

    fn helper(&self) -> u64 {
        self.counter.load(Ordering::Relaxed)
    }

    async fn get_counter(
        _rqctx: RequestContext<Self>,
    ) -> Result<HttpResponseOk<CounterValue>, HttpError> {
        let self_ = _rqctx.context();
        Ok(HttpResponseOk(CounterValue { counter: self_.helper() }))
    }

    async fn put_counter(
        _rqctx: RequestContext<Self>,
        update: TypedBody<CounterValue>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        let self_ = _rqctx.context();
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
}
