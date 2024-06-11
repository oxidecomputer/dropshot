// Copyright 2024 Oxide Computer Company

//! An extended version of `server-trait.rs`, demonstrating use of the
//! `dropshot::server` attribute macro to define a server with default methods.
//!
//! In this example, all the behavior lives on the context type, and the
//! Dropshot server is defined as a trait with default methods that delegate to
//! the context type.
//!
//! There are a number of possible variations of this, including:
//!
//! * The base behavior and endpoints are methods on the same trait, allo=wing
//!   implementations to override the base behavior if desired (similar to std's
//!   `Read` and `Write`).
//! * The behavior lives in a base trait, and the server is an extension trait
//!   on top of it, with a blanket impl prohibiting overrides (similar to
//!   tokio's `AsyncReadExt` and `AsyncWriteExt`).
//!
//! The general degrees of freedom available when dealing with traits in Rust
//! are almost all available here. The main bit of flexibility that isn't
//! available is that server traits can't be object-safe.

use dropshot::{ConfigLogging, ConfigLoggingLevel, HttpServerStarter};

/// The interface.
mod api {
    use dropshot::{
        HttpError, HttpResponseOk, HttpResponseUpdatedNoContent,
        RequestContext, TypedBody,
    };
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    /// Define our base trait.
    pub(crate) trait CounterBase {
        fn get_counter_impl(&self) -> u64;
        fn set_counter_impl(&self, value: u64) -> Result<(), String>;
    }

    /// The Dropshot server, with default methods.
    #[dropshot::server]
    pub(crate) trait CounterServer {
        /// Note the additional requirement on `CounterBase` here.
        type Context: CounterBase;

        /// Get the value of the counter.
        #[endpoint { method = GET, path = "/counter" }]
        async fn get_counter(
            rqctx: RequestContext<Self::Context>,
        ) -> Result<HttpResponseOk<CounterValue>, HttpError> {
            let cx = rqctx.context();
            Ok(HttpResponseOk(CounterValue { counter: cx.get_counter_impl() }))
        }

        /// Set the value of the counter.
        #[endpoint { method = PUT, path = "/counter" }]
        async fn put_counter(
            rqctx: RequestContext<Self::Context>,
            update: TypedBody<CounterValue>,
        ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
            let cx = rqctx.context();
            cx.set_counter_impl(update.into_inner().counter).map_err(
                |error| {
                    HttpError::for_bad_request(
                        Some(String::from("BadInput")),
                        error,
                    )
                },
            )?;

            Ok(HttpResponseUpdatedNoContent())
        }
    }

    /// A request and respose type used by `CounterServer` above.
    #[derive(Deserialize, Serialize, JsonSchema)]
    pub(crate) struct CounterValue {
        pub(crate) counter: u64,
    }

    // A simple function to generate an OpenAPI spec for the server, without having
    // a real implementation available.
    //
    // If the interface and implementation (see below) are in different crates, then
    // this function would live in the interface crate.
    pub(crate) fn generate_openapi_spec() -> String {
        let my_server = counter_server::stub_api_description().unwrap();
        let spec = my_server.openapi("Counter Server", "1.0.0");
        serde_json::to_string_pretty(&spec.json().unwrap()).unwrap()
    }
}

/// The implementation.
///
/// This code may live in another crate.
mod imp {
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::api::{CounterBase, CounterServer};

    /// The context type for our implementation.
    pub(crate) struct AtomicCounter {
        counter: AtomicU64,
    }

    impl AtomicCounter {
        pub(crate) fn new() -> AtomicCounter {
            AtomicCounter { counter: AtomicU64::new(0) }
        }
    }

    impl CounterBase for AtomicCounter {
        fn get_counter_impl(&self) -> u64 {
            self.counter.load(Ordering::Relaxed)
        }

        fn set_counter_impl(&self, value: u64) -> Result<(), String> {
            if value == 10 {
                Err(format!("do not like the number {}", value))
            } else {
                self.counter.store(value, Ordering::SeqCst);
                Ok(())
            }
        }
    }

    /// The type to hold the implementation of `CounterServer`. This could also
    /// be `AtomicCounter` itself.
    pub(crate) enum CounterImpl {}

    impl CounterServer for CounterImpl {
        type Context = AtomicCounter;

        // The default methods aren't overridden here, but that can be done if
        // desired.
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
        .to_logger("example-server-trait-default")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    // Print the OpenAPI spec to stdout as an example.
    println!("OpenAPI spec:");
    println!("{}", api::generate_openapi_spec());

    let my_server =
        api::counter_server::api_description::<imp::CounterImpl>().unwrap();
    let server = HttpServerStarter::new(
        &config_dropshot,
        my_server,
        imp::AtomicCounter::new(),
        &log,
    )
    .map_err(|error| format!("failed to create server: {}", error))?
    .start();

    server.await
}
