// Copyright 2023 Oxide Computer Company
//! Example use of Dropshot for multipart form-data.

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::Body;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::MultipartBody;
use dropshot::RequestContext;
use dropshot::ServerBuilder;
use http::{Response, StatusCode};

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

/// Return static content for all paths.
#[endpoint {
    method = POST,

    /*
     * Match literally every path including the empty path.
     */
    path = "/",

    /*
     * This isn't an API so we don't want this to appear in the OpenAPI
     * description if we were to generate it.
     */
    unpublished = true,
}]
async fn index(
    _rqctx: RequestContext<()>,
    mut body: MultipartBody,
) -> Result<Response<Body>, HttpError> {
    // Iterate over the fields, use `next_field()` to get the next field.
    while let Some(mut field) = body.content.next_field().await.unwrap() {
        // Get field name.
        let name = field.name();
        // Get the field's filename if provided in "Content-Disposition" header.
        let file_name = field.file_name();

        println!("Name: {:?}, File Name: {:?}", name, file_name);

        // Process the field data chunks e.g. store them in a file.
        while let Some(chunk) = field.chunk().await.unwrap() {
            // Do something with field chunk.
            println!("Chunk: {:?}", chunk);
        }
    }

    Ok(Response::builder().status(StatusCode::OK).body("Ok".into())?)
}
