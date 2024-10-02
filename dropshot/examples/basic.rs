// Copyright 2020 Oxide Computer Company
//! Example use of Dropshot.

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;
use dropshot::ServerBuilder;
use dropshot::TypedBody;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

#[tokio::main]
async fn main() -> Result<(), String> {
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

    // The functions that implement our API endpoints will share this context.
    let api_context = ExampleContext::new();

    // Set up the server.
    //
    // We use the default configuration here, which uses 127.0.0.1 since it's
    // always available and won't expose this server outside the host.  It also
    // uses port 0, which allows the operating system to pick any available
    // port.
    let server = ServerBuilder::new(api, api_context, log)
        .start()
        .map_err(|error| format!("failed to create server: {}", error))?;

    // Wait for the server to stop.  Note that there's not any code to shut down
    // this server, so we should never get past this point.
    server.await
}

/// Application-specific example context (state shared by handler functions)
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
#[derive(Deserialize, Serialize, JsonSchema)]
struct CounterValue {
    counter: u64,
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

/// Update the current value of the counter.  Note that the special value of 10
/// is not allowed (just to demonstrate how to generate an error).
#[endpoint {
    method = PUT,
    path = "/counter",
}]
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
