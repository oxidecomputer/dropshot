// Copyright 2023 Oxide Computer Company

//! Example use of Dropshot with request headers
//!
//! The headers accessed here will not be recorded as inputs in the OpenAPI
//! spec.  This is not currently supported out-of-the-box with Dropshot, but it
//! could be done by implementing you're own `SharedExtractor` that pulls the
//! headers out, similar to what's done here.
//!
//! This example is based on the "basic.rs" one.  See that one for more detailed
//! comments on the common code.

use dropshot::ApiDescription;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use dropshot::ServerBuilder;
use dropshot::endpoint;

#[tokio::main]
async fn main() -> Result<(), String> {
    // See dropshot/examples/basic.rs for more details on most of these pieces.
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;
    let mut api = ApiDescription::new();
    api.register(example_api_get_header_generic).unwrap();

    let api_context = ();
    let server = ServerBuilder::new(api, api_context, log)
        .start()
        .map_err(|error| format!("failed to create server: {}", error))?;
    server.await
}

/// Shows how to access a header that's not part of the OpenAPI spec
#[endpoint {
    method = GET,
    path = "/header-example-generic",
}]
async fn example_api_get_header_generic(
    rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<String>, HttpError> {
    // Note that clients can provide multiple values for a header.  See
    // http::HeaderMap for ways to get all of them.
    let header = rqctx.request.headers().get("demo-header");
    Ok(HttpResponseOk(format!("value for header: {:?}", header)))
}
