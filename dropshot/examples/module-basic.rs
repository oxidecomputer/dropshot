// Copyright 2020 Oxide Computer Company
//! Example use of Dropshot.

use dropshot::ApiDescription;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::ServerBuilder;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::sync::atomic::AtomicU64;

#[tokio::main]
async fn main() -> Result<(), String> {
    // See dropshot/examples/basic.rs for more details on most of these pieces.
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    let mut api = ApiDescription::new();
    api.register(routes::example_api_get_counter).unwrap();
    api.register(routes::example_api_put_counter).unwrap();

    let api_context = ExampleContext::new();

    let server = ServerBuilder::new(api, api_context, log)
        .start()
        .map_err(|error| format!("failed to create server: {}", error))?;

    server.await
}

/// Application-specific example context (state shared by handler functions)
pub struct ExampleContext {
    /// counter that can be manipulated by requests to the HTTP API
    pub counter: AtomicU64,
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
pub struct CounterValue {
    counter: u64,
}

/// The routes module might be imported from another crate that publishes
/// mountable routes
pub mod routes {
    use crate::{CounterValue, ExampleContext};
    use dropshot::HttpError;
    use dropshot::HttpResponseOk;
    use dropshot::HttpResponseUpdatedNoContent;
    use dropshot::RequestContext;
    use dropshot::TypedBody;
    use dropshot::endpoint;
    use std::sync::atomic::Ordering;

    /// Fetch the current value of the counter.
    /// NOTE: The endpoint macro inherits its module visibility from
    /// the endpoint async function definition
    #[endpoint {
          method = GET,
          path = "/counter",
      }]
    pub async fn example_api_get_counter(
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
    pub async fn example_api_put_counter(
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
}
