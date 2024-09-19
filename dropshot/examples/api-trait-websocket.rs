// Copyright 2024 Oxide Computer Company

//! Example use of `dropshot::api_description` with a WebSocket endpoint.

use dropshot::{ConfigLogging, ConfigLoggingLevel, HttpServerStarter};

/// The interface.
mod api {
    use dropshot::{
        HttpError, HttpResponseUpdatedNoContent, RequestContext, TypedBody,
        WebsocketConnection,
    };
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    /// The Dropshot server.
    #[dropshot::api_description]
    pub(crate) trait CounterApi {
        /// By default, the name of the context type is Context. To specify a
        /// different name, use the { context = ... } attribute on
        /// `#[dropshot::api_description]`.
        type Context;

        /// Set the start value of the counter.
        #[endpoint { method = PUT, path = "/counter" }]
        async fn put_counter(
            rqctx: RequestContext<Self::Context>,
            update: TypedBody<CounterValue>,
        ) -> Result<HttpResponseUpdatedNoContent, HttpError>;

        /// An eternally-increasing sequence of bytes, wrapping on overflow,
        /// starting from the value given for the query parameter "start."
        #[channel { protocol = WEBSOCKETS, path = "/ws" }]
        async fn get_counter_ws(
            rqctx: RequestContext<Self::Context>,
            upgraded: WebsocketConnection,
        ) -> dropshot::WebsocketChannelResult;
    }

    /// A request and response type used by `CounterApi` above.
    #[derive(Deserialize, Serialize, JsonSchema)]
    pub(crate) struct CounterValue {
        pub(crate) counter: u8,
    }

    // A simple function to generate an OpenAPI spec for the server, without
    // having a real implementation available.
    //
    // If the interface and implementation (see below) are in different crates,
    // then this function would live in the interface crate.
    pub(crate) fn generate_openapi_spec() -> String {
        let my_server = counter_api_mod::stub_api_description().unwrap();
        let spec =
            my_server.openapi("Counter Server", semver::Version::new(1, 0, 0));
        serde_json::to_string_pretty(&spec.json().unwrap()).unwrap()
    }
}

/// The implementation.
///
/// This code may live in another crate.
mod imp {
    use std::sync::atomic::{AtomicU8, Ordering};

    use dropshot::{
        HttpError, HttpResponseUpdatedNoContent, RequestContext, TypedBody,
        WebsocketConnection,
    };
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::{protocol::Role, Message};

    use crate::api::{CounterApi, CounterValue};

    /// The context type for our implementation.
    pub(crate) struct AtomicCounter {
        counter: AtomicU8,
    }

    impl AtomicCounter {
        pub(crate) fn new() -> AtomicCounter {
            AtomicCounter { counter: AtomicU8::new(0) }
        }
    }

    // Define a type to hold the implementation of `CounterServer`. This type will
    // never be constructed -- it is just a place to put the implementation of the
    // trait.
    //
    // In this case, it is alternatively possible to `impl CounterServer for
    // CounterImpl` directly with `type Context = Self`. This is an explicitly
    // supported option. In general, though, the context may be a foreign type (e.g.
    // `Arc<T>`) and having the separation between Self and Self::Context is useful.
    pub(crate) enum CounterImpl {}

    impl CounterApi for CounterImpl {
        type Context = AtomicCounter;

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

        async fn get_counter_ws(
            rqctx: RequestContext<Self::Context>,
            upgraded: WebsocketConnection,
        ) -> dropshot::WebsocketChannelResult {
            let mut ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
                upgraded.into_inner(),
                Role::Server,
                None,
            )
            .await;
            let mut count = rqctx.context().counter.load(Ordering::Relaxed);
            while ws.send(Message::Binary(vec![count])).await.is_ok() {
                count = count.wrapping_add(1);
            }
            Ok(())
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
        .to_logger("example-server-trait-websocket")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    // Print the OpenAPI spec to stdout as an example.
    println!("OpenAPI spec:");
    println!("{}", api::generate_openapi_spec());

    let my_server =
        api::counter_api_mod::api_description::<imp::CounterImpl>().unwrap();
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
