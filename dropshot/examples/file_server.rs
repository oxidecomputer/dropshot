// Copyright 2021 Oxide Computer Company

use dropshot::ApiDescription;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::HttpServerStarter;
use dropshot::RequestContext;
use dropshot::{endpoint, Path};
use http::{Response, StatusCode};
use hyper::Body;
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

/**
 * Our context is simply the root of the directory we want to serve.
 */
struct FileServerContext {
    base: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), String> {
    /*
     * Register DTrace probes.
     */
    dropshot::register_probes().expect("Failed to register DTrace probes");

    /*
     * We must specify a configuration with a bind address.  We'll use 127.0.0.1
     * since it's available and won't expose this server outside the host.  We
     * request port 0, which allows the operating system to pick any available
     * port.
     */
    let config_dropshot = Default::default();

    /*
     * For simplicity, we'll configure an "info"-level logger that writes to
     * stderr assuming that it's a terminal.
     */
    let config_logging = ConfigLogging::StderrTerminal {
        level: ConfigLoggingLevel::Info,
    };
    let log = config_logging
        .to_logger("example-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    /*
     * Build a description of the API -- in this case it's not much of an API!.
     */
    let mut api = ApiDescription::new();
    api.register(static_content).unwrap();

    /*
     * Specify the directory we want to serve.
     */
    let context = FileServerContext {
        base: PathBuf::from("."),
    };

    /*
     * Set up the server.
     */
    let server = HttpServerStarter::new(&config_dropshot, api, context, &log)
        .map_err(|error| format!("failed to create server: {}", error))?
        .start();

    /*
     * Wait for the server to stop.  Note that there's not any code to shut down
     * this server, so we should never get past this point.
     */
    server.await
}

/**
 * Dropshot deserializes the input path into this Vec.
 */
#[derive(Deserialize, JsonSchema)]
struct AllPath {
    path: Vec<String>,
}

/**
 * Serve files from the specified root path.
 */
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
async fn static_content(
    rqctx: Arc<RequestContext<FileServerContext>>,
    path: Path<AllPath>,
) -> Result<Response<Body>, HttpError> {
    let path = path.into_inner().path;
    let mut entry = rqctx.context().base.clone();
    for component in &path {
        /* The previous iteration needs to have resulted in a directory. */
        if !entry.is_dir() {
            return Err(HttpError::for_bad_request(
                None,
                "ENOTDIR".to_string(),
            ));
        }
        /* Dropshot won't ever give us dot-components. */
        assert_ne!(component, ".");
        assert_ne!(component, "..");
        entry.push(component);

        /*
         * We explicitly prohibit consumers from following symlinks to prevent
         * showing data outside of the intended directory.
         */
        let m = entry.symlink_metadata().map_err(|_| {
            HttpError::for_bad_request(None, "ENOENT".to_string())
        })?;
        if m.file_type().is_symlink() {
            return Err(HttpError::for_bad_request(None, "EMLINK".to_string()));
        }
    }

    /*
     * If the entry is a directory, we serve a listing of its contents. If it's
     * regular file we serve the file.
     */
    if entry.is_dir() {
        let body = dir_body(entry).await.map_err(|_| {
            HttpError::for_bad_request(None, "EBADF".to_string())
        })?;

        Ok(Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, "text/html")
            .body(body.into())?)
    } else {
        let body = tokio::fs::read(&entry).await.map_err(|_| {
            HttpError::for_bad_request(None, "EBADF".to_string())
        })?;

        /* Derive the MIME type from the file name */
        let content_type = mime_guess::from_path(&entry)
            .first()
            .map_or_else(|| "text/plain".to_string(), |m| m.to_string());

        Ok(Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, content_type)
            .body(body.into())?)
    }
}

/**
 * Generate a simple HTML listing of files within the directory.
 * See the note below regarding the handling of trailing slashes.
 */
async fn dir_body(dir_path: PathBuf) -> Result<String, std::io::Error> {
    let dir_link = dir_path.to_string_lossy();
    let mut dir = tokio::fs::read_dir(&dir_path).await?;

    let mut body = String::new();

    body.push_str(
        format!(
            "<html>
            <head><title>{}/</title></head>
            <body>
            <h1>{}/</h1>
            ",
            dir_link, dir_link
        )
        .as_str(),
    );
    body.push_str("<ul>\n");
    while let Some(entry) = dir.next_entry().await? {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        /*
         * Note that Dropshot handles paths with and without trailing slashes
         * as identical. This is important with respect to relative paths as
         * the destination of a relative path is different depending on whether
         * or not a trailing slash is present in the browser's location bar.
         * For example, a relative url of "bar" would go from the location
         * "localhost:123/foo" to "localhost:123/bar" and from the location
         * "localhost:123/foo/" to "localhost:123/foo/bar". More robust
         * handling would require distinct handling of the trailing slash
         * and a redirect in the case of its absence when navigating to a
         * directory.
         */
        body.push_str(
            format!(
                r#"<li><a href="{}{}">{}</a></li>"#,
                name,
                if entry.file_type().await?.is_dir() { "/" } else { "" },
                name
            )
            .as_str(),
        );
        body.push('\n');
    }
    body.push_str(
        "</ul>
        </body>
        </html>\n",
    );

    Ok(body)
}
