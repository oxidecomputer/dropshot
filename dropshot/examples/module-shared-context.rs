// Copyright 2021 Oxide Computer Company
//! Example use of Dropshot where a client wants to act on
//! a custom context object that outlives endpoint functions.

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use dropshot::ServerBuilder;
use futures::FutureExt;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), String> {
    // See dropshot/examples/basic.rs for more details on most of these pieces.
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    let mut api = ApiDescription::new();
    api.register(example_api_get_counter).unwrap();

    let api_context = Arc::new(ExampleContext::new());

    let server = ServerBuilder::new(api, api_context.clone(), log)
        .start()
        .map_err(|error| format!("failed to create server: {}", error))?;

    // Wait for the server to stop.  Note that there's not any code to shut down
    // this server, so we should never get past this point.
    //
    // Even with the endpoints acting on the `ExampleContext` object,
    // we can still hold a reference and act on the object beyond the lifetime
    // of those endpoints.
    //
    // In this example, we increment the counter every five seconds,
    // regardless of received HTTP requests.
    futures::pin_mut!(server);
    loop {
        let sleep =
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).fuse();
        futures::pin_mut!(sleep);
        futures::select! {
            _ = sleep => { api_context.counter.fetch_add(1, Ordering::SeqCst); }
            _ = server => break,
        }
    }

    Ok(())
}

/// Application-specific example context (state shared by handler functions)
pub struct ExampleContext {
    /// counter that can be read by requests to the HTTP API
    pub counter: AtomicU64,
}

impl ExampleContext {
    /// Return a new ExampleContext.
    pub fn new() -> ExampleContext {
        ExampleContext { counter: AtomicU64::new(0) }
    }
}

// HTTP API interface

/// `CounterValue` represents the value of the API's counter.
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct CounterValue {
    counter: u64,
}

/// Fetch the current value of the counter.
#[endpoint {
      method = GET,
      path = "/counter",
  }]
pub async fn example_api_get_counter(
    rqctx: RequestContext<Arc<ExampleContext>>,
) -> Result<HttpResponseOk<CounterValue>, HttpError> {
    let api_context = rqctx.context();

    Ok(HttpResponseOk(CounterValue {
        counter: api_context.counter.load(Ordering::SeqCst),
    }))
}
