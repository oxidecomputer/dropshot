// Copyright 2021 Oxide Computer Company
//! Example use of Dropshot for matching wildcard paths to serve static content.

use dropshot::ApiDescription;
use dropshot::Body;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::RequestContext;
use dropshot::ServerBuilder;
use dropshot::{Path, endpoint};
use http::{Response, StatusCode};
use schemars::JsonSchema;
use serde::Deserialize;

#[tokio::main]
async fn main() -> Result<(), String> {
    // See dropshot/examples/basic.rs for more details on most of these pieces.
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    let mut api = ApiDescription::new();
    api.register(index).unwrap();

    let server = ServerBuilder::new(api, (), log)
        .start()
        .map_err(|error| format!("failed to create server: {}", error))?;

    server.await
}

#[derive(Deserialize, JsonSchema)]
struct AllPath {
    path: Vec<String>,
}

/// Return static content for all paths.
#[endpoint {
    method = GET,

    /*
     * Match literally every path including the empty path.
     */
    path = "/{path:.*}",

    /*
     * This isn't an API so we don't want this to appear in the OpenAPI
     * description if we were to generate it.
     */
    unpublished = true,
}]
async fn index(
    _rqctx: RequestContext<()>,
    path: Path<AllPath>,
) -> Result<Response<Body>, HttpError> {
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "text/html")
        .body(
            format!(
                "<HTML><HEAD>nothing at {:?}</HEAD></HTML>",
                path.into_inner().path
            )
            .into(),
        )?)
}
