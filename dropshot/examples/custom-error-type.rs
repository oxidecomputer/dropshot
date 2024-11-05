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

/// Any type implementing the [`dropshot::error::IntoErrorResponse`],
/// [`schemars::JsonSchema`], and [`std::fmt::Display`] traits may be returned
/// from a handler function. In this case, let's use an enum rather than a
/// struct, just to demonstrate that this is possible.
#[derive(
    Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub enum MyError {
    CantGetYeFlask,
    OverfullHbox { badness: usize },
}

impl dropshot::error::IntoErrorResponse for MyError {
    fn into_error_response(
        &self,
        _request_id: &str,
    ) -> http::Response<dropshot::Body> {
        http::Response::builder()
            .status(http::StatusCode::BAD_REQUEST)
            .body(
                serde_json::to_string(self)
                    .expect("serialization of MyError should never fail")
                    .into(),
            )
            .expect("building response should never fail")
    }
}

impl std::fmt::Display for MyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MyError::CantGetYeFlask => f.write_str("can't get ye flask"),
            MyError::OverfullHbox { badness } => {
                write!(f, "overfull hbox, badness {badness}")
            }
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
#[endpoint {
    method = GET,
    path = "/pdflatex/{filename}",
}]
async fn get_pdflatex(
    _rqctx: RequestContext<()>,
    _path: dropshot::Path<PdflatexPathParams>,
) -> Result<http::Response<Body>, MyError> {
    Err(MyError::OverfullHbox { badness: 1000 })
}

#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
struct PdflatexPathParams {
    filename: String,
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
    api.register(get_pdflatex).unwrap();

    let server = ServerBuilder::new(api, (), log)
        .start()
        .map_err(|error| format!("failed to create server: {}", error))?;

    server.await
}
