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

/// Any type implementing `dropshot::HttpResponseError` and `HttpResponseContent` may be used as an error
/// return value from an endpoint handler.
impl HttpResponseError for ThingyError {
    fn status_code(&self) -> http::StatusCode {
        match self {
            ThingyError::NoThingies => http::StatusCode::SERVICE_UNAVAILABLE,
            ThingyError::InvalidThingy { .. } => http::StatusCode::BAD_REQUEST,
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

    api.openapi("Custom Error Example", "0.0.0")
        .write(&mut std::io::stdout())
        .map_err(|e| e.to_string())?;

    let server = ServerBuilder::new(api, (), log)
        .start()
        .map_err(|error| format!("failed to create server: {}", error))?;

    server.await
}
