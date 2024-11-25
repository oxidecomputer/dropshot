// Copyright 2024 Oxide Computer Company

//! An example demonstrating how to return user-defined error types from
//! endpoint handlers.

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::ErrorStatusCode;
use dropshot::HttpError;
use dropshot::HttpResponseError;
use dropshot::HttpResponseOk;
use dropshot::Path;
use dropshot::RequestContext;
use dropshot::ServerBuilder;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

// This is the custom error type returned by our API. A common use case for
// custom error types is to return an `enum` type that represents
// application-specific errors in a way that generated clients can interact with
// programmatically, so the error in this example will be an enum type.
//
// In order to be returned from an endpoint handler, it must implement the
// `HttpResponseError` trait, which requires implementations of:
//
// - `HttpResponseContent`, which determines how to produce a response body
//   from the error type,
// - `std::fmt::Display`, which determines how to produce a human-readable
//    message for Dropshot to log when returning the error,
// - `From<dropshot::HttpError>`, so that errors returned by request extractors
//   and resposne body serialization can be converted to the user-defined error
//   type.
#[derive(Debug)]
// Deriving `Serialize` and `JsonSchema` for our error type provides an
// implementation of the `HttpResponseContent` trait, which is required to
// implement `HttpResponseError`:
#[derive(serde::Serialize, schemars::JsonSchema)]
// `HttpResponseError` also requires a `std::fmt::Display` implementation,
// which we'll generate using `thiserror`'s `Error` derive:
#[derive(thiserror::Error)]
enum ThingyError {
    // First, define some application-specific error variants that represent
    // structured error responses from our API:
    /// No thingies are currently available to satisfy this request.
    #[error("no thingies are currently available")]
    NoThingies,

    /// The requested thingy is invalid.
    #[error("invalid thingy: {:?}", .name)]
    InvalidThingy { name: String },

    // Then, we'll define a variant that can be constructed from a
    // `dropshot::HttpError`, so that errors returned by Dropshot can also be
    // represented in the error schema for our API:
    #[error("{internal_message}")]
    Other {
        message: String,
        error_code: Option<String>,

        // Skip serializing these fields, as they are used for the
        // `fmt::Display` implementation and for determining the status
        // code, respectively, rather than included in the response body:
        #[serde(skip)]
        internal_message: String,
        #[serde(skip)]
        status: ErrorStatusCode,
    },
}

impl HttpResponseError for ThingyError {
    // Note that this method returns a `dropshot::ErrorStatusCode`, rather than
    // an `http::StatusCode`. This type is a refinement of `http::StatusCode`
    // that can only be constructed from status codes in 4xx (client error) or
    // 5xx (server error) ranges.
    fn status_code(&self) -> dropshot::ErrorStatusCode {
        match self {
            ThingyError::NoThingies => {
                // The `dropshot::ErrorStatusCode` type provides constants for
                // all well-known 4xx and 5xx status codes, such as 503 Service
                // Unavailable.
                dropshot::ErrorStatusCode::SERVICE_UNAVAILABLE
            }
            ThingyError::InvalidThingy { .. } => {
                // Alternatively, an `ErrorStatusCode` can be constructed from a
                // u16, but the `ErrorStatusCode::from_u16` constructor
                // validates that the status code is a 4xx or 5xx.
                //
                // This allows using extended status codes, while still
                // validating that they are errors.
                dropshot::ErrorStatusCode::from_u16(442)
                    .expect("442 is a 4xx status code")
            }
            ThingyError::Other { status, .. } => *status,
        }
    }
}

impl From<HttpError> for ThingyError {
    fn from(error: HttpError) -> Self {
        ThingyError::Other {
            message: error.external_message,
            internal_message: error.internal_message,
            status: error.status_code,
            error_code: error.error_code,
        }
    }
}

/// Just some kind of thingy returned by the API. This doesn't actually matter.
#[derive(Deserialize, Serialize, JsonSchema)]
struct Thingy {
    magic_number: u64,
}

#[derive(Deserialize, JsonSchema)]
struct ThingyPathParams {
    name: ThingyName,
}

// Using an enum as a path parameter allows the API to also return extractor
// errors. Try sending a `GET` request for `/thingy/baz` or similar to see how
// the extractor error is converted into our custom error representation.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum ThingyName {
    Foo,
    Bar,
}

/// Fetch the thingy with the provided name.
#[endpoint {
    method = GET,
    path = "/thingy/{name}",
}]
async fn get_thingy(
    _rqctx: RequestContext<()>,
    path_params: Path<ThingyPathParams>,
) -> Result<HttpResponseOk<Thingy>, ThingyError> {
    let ThingyPathParams { name } = path_params.into_inner();
    Err(ThingyError::InvalidThingy { name: format!("{name:?}") })
}

#[endpoint {
    method = GET,
    path = "/nothing",
}]
async fn get_nothing(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<Thingy>, ThingyError> {
    Err(ThingyError::NoThingies)
}

/// Endpoints which return `Result<_, HttpError>` may be part of the same
/// API as endpoints which return user-defined error types.
#[endpoint {
    method = GET,
    path = "/something",
}]
async fn get_something(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<Thingy>, dropshot::HttpError> {
    Ok(HttpResponseOk(Thingy { magic_number: 42 }))
}

#[tokio::main]
async fn main() -> Result<(), String> {
    // See dropshot/examples/basic.rs for more details on most of these pieces.
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-custom-error")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    let mut api = ApiDescription::new();
    api.register(get_thingy).unwrap();
    api.register(get_nothing).unwrap();
    api.register(get_something).unwrap();

    api.openapi("Custom Error Example", semver::Version::new(0, 0, 0))
        .write(&mut std::io::stdout())
        .map_err(|e| e.to_string())?;

    let server = ServerBuilder::new(api, (), log)
        .start()
        .map_err(|error| format!("failed to create server: {}", error))?;

    server.await
}
