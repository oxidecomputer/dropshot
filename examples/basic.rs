/*!
 * Example use of Dropshot.
 */

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::HttpResponseOkObject;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::HttpServer;
use dropshot::Json;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::any::Any;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), String> {
    /*
     * We must specify a configuration with a bind address.  We'll use 127.0.0.1
     * since it's available and won't expose this server outside the host.  We
     * request port 0, which allows the operating system to pick any available
     * port.
     */
    let config_dropshot = ConfigDropshot {
        bind_address: "127.0.0.1:0".parse().unwrap(),
    };

    /*
     * For simplicity, we'll configure an "info"-level logger that writes to
     * stderr assuming that it's a terminal.
     */
    let config_logging = ConfigLogging::StderrTerminal {
        level: ConfigLoggingLevel::Info,
    };
    let log = config_logging
        .to_logger("example-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    /*
     * Build a description of the API.
     */
    let mut api = ApiDescription::new();
    api.register(example_api_get_counter).unwrap();
    api.register(example_api_put_counter).unwrap();

    /*
     * The functions that implement our API endpoints will share this context.
     */
    let api_context = ExampleContext::new();

    /*
     * Set up the server.
     */
    let mut server = HttpServer::new(&config_dropshot, api, api_context, &log)
        .map_err(|error| format!("failed to create server: {}", error))?;
    let server_task = server.run();

    /*
     * Wait for the server to stop.  Note that there's not any code to shut down
     * this server, so we should never get past this point.
     */
    server.wait_for_shutdown(server_task).await
}

/**
 * Application-specific example context (state shared by handler functions)
 */
struct ExampleContext {
    /** counter that can be manipulated by requests to the HTTP API */
    counter: AtomicU64,
}

impl ExampleContext {
    /**
     * Return a new ExampleContext.
     */
    pub fn new() -> Arc<ExampleContext> {
        Arc::new(ExampleContext {
            counter: AtomicU64::new(0),
        })
    }

    /**
     * Given `rqctx` (which is provided by Dropshot to all HTTP handler
     * functions), return our application-specific context.
     */
    pub fn from_rqctx(rqctx: &Arc<RequestContext>) -> Arc<ExampleContext> {
        let ctx: Arc<dyn Any + Send + Sync + 'static> =
            Arc::clone(&rqctx.server.private);
        ctx.downcast::<ExampleContext>().expect("wrong type for private data")
    }
}

/*
 * HTTP API interface
 */

/**
 * `CounterValue` represents the value of the API's counter, either as the
 * response to a GET request to fetch the counter or as the body of a PUT
 * request to update the counter.
 */
#[derive(Deserialize, Serialize, JsonSchema)]
struct CounterValue {
    counter: u64,
}

/**
 * Fetch the current value of the counter.
 */
#[endpoint {
    method = GET,
    path = "/counter",
}]
async fn example_api_get_counter(
    rqctx: Arc<RequestContext>,
) -> Result<HttpResponseOkObject<CounterValue>, HttpError> {
    let api_context = ExampleContext::from_rqctx(&rqctx);

    Ok(HttpResponseOkObject(CounterValue {
        counter: api_context.counter.load(Ordering::SeqCst),
    }))
}

/**
 * Update the current value of the counter.  Note that the special value of 10
 * is not allowed (just to demonstrate how to generate an error).
 */
#[endpoint {
    method = PUT,
    path = "/counter",
}]
async fn example_api_put_counter(
    rqctx: Arc<RequestContext>,
    update: Json<CounterValue>,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    let api_context = ExampleContext::from_rqctx(&rqctx);
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
