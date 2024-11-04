// Copyright 2024 Oxide Computer Companyy

//! Example of an API using a user-defined error type to serialize error
//! representations differently from [`dropshot::HttpError`].

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::Body;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::RequestContext;
use dropshot::ServerBuilder;

/// Any type implementing the [`serde::Serialize`], [`schemars::JsonSchema`],
/// and [`dropshot::error::AsStatusCode`] traits may be returned from a
/// handler function. In this case, let's use an enum rather than a struct, just
/// to demonstrate that this is possible.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub enum MyError {
    CantGetYeFlask,
    OverfullHbox { badness: usize, }
}

/// In order to return a user-defined error type, it must implement
/// `dropshot::error::AsStatusCode`, which determines the HTTP status code to
/// return for this error.
impl dropshot::error::AsStatusCode for MyError {
    fn as_status_code(&self) -> http::StatusCode {
        match self {
            Self::CantGetYeFlask => http::StatusCode::FORBIDDEN,
            Self::OverfullHbox { .. } => http::StatusCode::BAD_REQUEST,
        }
    }
}

#[endpoint {
    method = GET,
    path = "/ye-flask",
}]
async fn get_ye_flask(
    _rqctx: RequestContext<()>,
) -> Result<http::Response<Body>, MyError> {
    Err(MyError::CantGetYeFlask)
}


#[tokio::main]
async fn main() -> Result<(), String> {
    // See dropshot/examples/basic.rs for more details on most of these pieces.
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-custom-error-type")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    let mut api = ApiDescription::new();
    api.register(get_ye_flask).unwrap();

    let server = ServerBuilder::new(api, (), log)
        .start()
        .map_err(|error| format!("failed to create server: {}", error))?;

    server.await
}
