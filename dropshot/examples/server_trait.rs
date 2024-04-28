use std::sync::atomic::{AtomicU64, Ordering};

use dropshot::{
    ApiDescription, ConfigLogging, ConfigLoggingLevel, HttpError,
    HttpResponseOk, HttpResponseUpdatedNoContent, HttpServerStarter,
    RequestContext, TypedBody,
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
    // to_api_description method. Need to make this better.
    let my_server = to_api_description(MyContext::new());
    let server = HttpServerStarter::new(&config_dropshot, my_server, (), &log)
        .map_err(|error| format!("failed to create server: {}", error))?
        .start();

    server.await
}

#[derive(Deserialize, Serialize, JsonSchema)]
struct CounterValue {
    counter: u64,
}

#[dropshot_server]
trait MyServer: Send + Sync + 'static {
    fn helper(&self) -> u64;

    #[endpoint { method = GET, path = "/counter" }]
    async fn get_counter(
        &self,
        rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<CounterValue>, HttpError>;

    #[endpoint { method = PUT, path = "/counter" }]
    async fn put_counter(
        &self,
        rqctx: RequestContext<()>,
        update: TypedBody<CounterValue>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

struct MyContext {
    counter: AtomicU64,
}

impl MyContext {
    pub fn new() -> MyContext {
        MyContext { counter: AtomicU64::new(0) }
    }
}

#[async_trait::async_trait]
impl MyServer for MyContext {
    fn helper(&self) -> u64 {
        self.counter.load(Ordering::Relaxed)
    }

    async fn get_counter(
        &self,
        rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<CounterValue>, HttpError> {
        Ok(HttpResponseOk(CounterValue { counter: self.helper() }))
    }

    async fn put_counter(
        &self,
        rqctx: RequestContext<()>,
        update: TypedBody<CounterValue>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        let updated_value = update.into_inner();

        if updated_value.counter == 10 {
            Err(HttpError::for_bad_request(
                Some(String::from("BadInput")),
                format!("do not like the number {}", updated_value.counter),
            ))
        } else {
            self.counter.store(updated_value.counter, Ordering::SeqCst);
            Ok(HttpResponseUpdatedNoContent())
        }
    }
}
