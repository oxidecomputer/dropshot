// Copyright 2024 Oxide Computer Company

//! An example demonstrating how to return user-defined error types from
//! endpoint handlers.

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpResponseError;
use dropshot::HttpResponseOk;
use dropshot::Path;
use dropshot::RequestContext;
use dropshot::ServerBuilder;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, thiserror::Error, Serialize, Deserialize, JsonSchema)]
enum ThingyError {
    #[error("no thingies are currently available")]
    NoThingies,
    #[error("invalid thingy: {:?}", .name)]
    InvalidThingy { name: String },
}

/// Any type implementing `dropshot::HttpResponseError` and
/// `HttpResponseContent` may be used as an error type for a
/// return value from an endpoint handler.
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
        }
    }
}

/// Just some kind of thingy returned by the API. This doesn't actually matter.
#[derive(Deserialize, Serialize, JsonSchema)]
struct Thingy {
    magic_number: u64,
}

#[derive(Deserialize, Serialize, JsonSchema)]
struct NoThingy {}

#[derive(Deserialize, JsonSchema)]
struct ThingyPathParams {
    name: String,
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
    Err(ThingyError::InvalidThingy { name })
}

/// An example of an endpoint which does not return a `Result<_, _>`.
#[endpoint {
    method = GET,
    path = "/nothing",
}]
async fn get_nothing(_rqctx: RequestContext<()>) -> HttpResponseOk<NoThingy> {
    HttpResponseOk(NoThingy {})
}

/// An example of an endpoint which returns a `Result<_, HttpError>`.
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
