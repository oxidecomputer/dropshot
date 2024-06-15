// Copyright 2024 Oxide Computer Company

//! A basic example demonstrating use of the `dropshot::api_description`
//! attribute macro to define an API.
//!
//! There are two parts: the interface and the implementation. The interface
//! defines the endpoints and the types used by the API. The implementation
//! provides the actual behavior of the server.
//!
//! In production code, the interface and implementation would likely be in
//! separate crates. This allows the OpenAPI spec to be generated without the
//! implementation having to be compiled (or even exist in the first place). See
//! the note on "Where to put the implementation" in the crate `lib.rs` for more
//! details.
//!
//! This example puts the interface and implementation in separate modules.

use dropshot::{ConfigLogging, ConfigLoggingLevel, HttpServerStarter};

/// The interface.
mod api {
    use dropshot::{
        HttpError, HttpResponseOk, HttpResponseUpdatedNoContent,
        RequestContext, TypedBody,
    };
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    /// The Dropshot API trait.
    #[dropshot::api_description]
    pub(crate) trait CounterApi {
        /// By default, the name of the context type is Context. To specify a
        /// different name, use the { context = ... } attribute on
        /// `#[dropshot::api_description]`.
        type Context;

        /// Get the value of the counter.
        #[endpoint { method = GET, path = "/counter" }]
        async fn get_counter(
            rqctx: RequestContext<Self::Context>,
        ) -> Result<HttpResponseOk<CounterValue>, HttpError>;

        /// Set the value of the counter.
        #[endpoint { method = PUT, path = "/counter" }]
        async fn put_counter(
            rqctx: RequestContext<Self::Context>,
            update: TypedBody<CounterValue>,
        ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
    }

    /// A request and respose type used by `CounterApi` above.
    #[derive(Deserialize, Serialize, JsonSchema)]
    pub(crate) struct CounterValue {
        pub(crate) counter: u64,
    }

    // A simple function to generate an OpenAPI spec for the trait, without having
    // a real implementation available.
    //
    // If the interface and implementation (see below) are in different crates, then
    // this function would live in the interface crate.
    pub(crate) fn generate_openapi_spec() -> String {
        let description = counter_api::stub_api_description().unwrap();
        let spec = description.openapi("Counter Server", "1.0.0");
        serde_json::to_string_pretty(&spec.json().unwrap()).unwrap()
    }
}

/// The implementation.
///
/// This code may live in another crate.
mod imp {
    use std::sync::atomic::{AtomicU64, Ordering};

    use dropshot::{
        HttpError, HttpResponseOk, HttpResponseUpdatedNoContent,
        RequestContext, TypedBody,
    };

    use crate::api::{CounterApi, CounterValue};

    /// The context type for our implementation.
    pub(crate) struct AtomicCounter {
        counter: AtomicU64,
    }

    impl AtomicCounter {
        pub(crate) fn new() -> AtomicCounter {
            AtomicCounter { counter: AtomicU64::new(0) }
        }
    }

    // Define a type to hold the implementation of `CounterApi`. This type will
    // never be constructed -- it is just a place to put the implementation of the
    // trait.
    //
    // In this case, it is alternatively possible to `impl CounterApi for
    // CounterImpl` directly with `type Context = Self`. This is an explicitly
    // supported option. In general, though, the context may be a foreign type (e.g.
    // `Arc<T>`) and having the separation between Self and Self::Context is useful.
    pub(crate) enum CounterImpl {}

    impl CounterApi for CounterImpl {
        type Context = AtomicCounter;

        async fn get_counter(
            rqctx: RequestContext<Self::Context>,
        ) -> Result<HttpResponseOk<CounterValue>, HttpError> {
            let cx = rqctx.context();
            Ok(HttpResponseOk(CounterValue {
                counter: cx.counter.load(Ordering::Relaxed),
            }))
        }

        async fn put_counter(
            rqctx: RequestContext<Self::Context>,
            update: TypedBody<CounterValue>,
        ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
            let cx = rqctx.context();
            let updated_value = update.into_inner();

            if updated_value.counter == 10 {
                Err(HttpError::for_bad_request(
                    Some(String::from("BadInput")),
                    format!("do not like the number {}", updated_value.counter),
                ))
            } else {
                cx.counter.store(updated_value.counter, Ordering::SeqCst);
                Ok(HttpResponseUpdatedNoContent())
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let config_dropshot = Default::default();
    // For simplicity, we'll configure an "info"-level logger that writes to
    // stderr assuming that it's a terminal.
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-api-trait")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    // Print the OpenAPI spec to stdout as an example.
    println!("OpenAPI spec:");
    println!("{}", api::generate_openapi_spec());

    // The api_description function accepts the specific implementation as a
    // type parameter.
    let my_api =
        api::counter_api::api_description::<imp::CounterImpl>().unwrap();
    let server = HttpServerStarter::new(
        &config_dropshot,
        my_api,
        imp::AtomicCounter::new(),
        &log,
    )
    .map_err(|error| format!("failed to create server: {}", error))?
    .start();

    server.await
}
