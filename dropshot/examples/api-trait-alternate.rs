// Copyright 2024 Oxide Computer Company

//! An extended version of `api-trait.rs`, demonstrating use of the
//! `dropshot::api_description` attribute macro to define an API with default
//! methods.
//!
//! In this example, all the server logic lives on the context type, and the
//! trait has default methods that delegate to the context type.
//!
//! ## Variations
//!
//! The general degrees of freedom available when dealing with traits in Rust
//! are almost all available here. The main bit of flexibility that isn't
//! available is that API traits can't be object-safe.
//!
//! ### Same trait with default methods
//!
//! For example:
//!
//! ```rust,ignore
//! #[dropshot::api_description]
//! trait CounterApi {
//!      type Context;
//!
//!      fn get_counter_impl(&self) -> impl Future<Output = u64> + Send;
//!      fn set_counter_impl(
//!          &self,
//!          value: u64,
//!      ) -> impl Future<Output = Result<(), String>> + Send;
//!
//!      #[endpoint { method = GET, path = "/counter" }
//!      async fn get_counter() {} // as below
//!
//!      #[endpoint { method = PUT, path = "/counter" }
//!      async fn put_counter() {} // as below
//! }
//! ```
//!
//! Implementations of `CounterApi` can override `get_counter` and
//! `put_counter`.
//!
//! This is similar to how [`std::io::Write`]'s `write_all` method is defined as
//! a default method over the required `write`. Implementations of `Write` can
//! override `write_all` if they wish.
//!
//! ### Supertrait
//!
//! ```rust,ignore
//! trait CounterBase {
//!     fn get_counter_impl(&self) -> impl Future<Output = u64> + Send;
//!     fn set_counter_impl(
//!         &self,
//!         value: u64,
//!     ) -> impl Future<Output = Result<(), String>> + Send;
//! }
//!
//! #[dropshot::api_description]
//! trait CounterApi: CounterBase {
//!     #[endpoint { method = GET, path = "/counter" }
//!     async fn get_counter() {} // as below
//!
//!     #[endpoint { method = PUT, path = "/counter" }
//!     async fn put_counter() {} // as below
//! }
//!
//! // And optionally, a blanket impl.
//! impl<T: CounterBase> CounterApi for T {}
//! ```
//!
//! With the optional blanket impl, implementations cannot override the default
//! `CounterApi` methods.
//!
//! This is similar to how [`tokio::io::AsyncWriteExt`] extends
//! [`AsyncWrite`](tokio::io::AsyncWrite) with a blanket impl. In that case, the
//! behavior of methods like `AsyncWriteExt::write_all` cannot be overridden.

use dropshot::{ConfigLogging, ConfigLoggingLevel, HttpServerStarter};

/// The interface.
mod api {
    use dropshot::{
        HttpError, HttpResponseOk, HttpResponseUpdatedNoContent,
        RequestContext, TypedBody,
    };
    use futures::Future;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    /// Define our base trait.
    ///
    /// If the methods are async, it is required that they be `Send`. You can
    /// use `async_trait` or `trait-variant` for this, or explicitly use `impl
    /// Future<...> + Send`. (Implementations can write `async fn` and
    /// automatically get the `Send` bound.)
    pub(crate) trait CounterBase {
        fn get_counter_impl(&self) -> impl Future<Output = u64> + Send;
        fn set_counter_impl(
            &self,
            value: u64,
        ) -> impl Future<Output = Result<(), String>> + Send;
    }

    /// The Dropshot API, with default methods.
    #[dropshot::api_description]
    pub(crate) trait CounterApi {
        /// Note the additional requirement on `CounterBase` here.
        type Context: CounterBase;

        /// Get the value of the counter.
        #[endpoint { method = GET, path = "/counter" }]
        async fn get_counter(
            rqctx: RequestContext<Self::Context>,
        ) -> Result<HttpResponseOk<CounterValue>, HttpError> {
            let cx = rqctx.context();
            Ok(HttpResponseOk(CounterValue {
                counter: cx.get_counter_impl().await,
            }))
        }

        /// Set the value of the counter.
        #[endpoint { method = PUT, path = "/counter" }]
        async fn put_counter(
            rqctx: RequestContext<Self::Context>,
            update: TypedBody<CounterValue>,
        ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
            let cx = rqctx.context();
            cx.set_counter_impl(update.into_inner().counter).await.map_err(
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
        let api = counter_api_mod::stub_api_description().unwrap();
        let spec = api.openapi("Counter Server", "1.0.0");
        serde_json::to_string_pretty(&spec.json().unwrap()).unwrap()
    }
}

/// The implementation.
///
/// This code may live in another crate.
mod imp {
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::api::{CounterApi, CounterBase};

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
        async fn get_counter_impl(&self) -> u64 {
            self.counter.load(Ordering::Relaxed)
        }

        async fn set_counter_impl(&self, value: u64) -> Result<(), String> {
            if value == 10 {
                Err(format!("do not like the number {}", value))
            } else {
                self.counter.store(value, Ordering::SeqCst);
                Ok(())
            }
        }
    }

    /// The type to hold the implementation of `CounterApi`. This could also
    /// be `AtomicCounter` itself.
    pub(crate) enum CounterImpl {}

    impl CounterApi for CounterImpl {
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
        .to_logger("example-api-trait-default")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    // Print the OpenAPI spec to stdout as an example.
    println!("OpenAPI spec:");
    println!("{}", api::generate_openapi_spec());

    let my_api =
        api::counter_api_mod::api_description::<imp::CounterImpl>().unwrap();
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
