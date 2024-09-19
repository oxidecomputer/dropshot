// Copyright 2024 Oxide Computer Company

//! Example using API versioning

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::DynamicVersionPolicy;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::HttpServerStarter;
use dropshot::RequestContext;
use dropshot::VersionPolicy;
use hyper::Body;
use hyper::Request;
use schemars::JsonSchema;
use semver::Version;
use serde::Serialize;
use slog::Logger;

#[tokio::main]
async fn main() -> Result<(), String> {
    // See dropshot/examples/basic.rs for more details on these pieces.
    let config_dropshot: ConfigDropshot = Default::default();
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Debug };
    let log = config_logging
        .to_logger("example-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    let mut api = ApiDescription::new();
    api.register(v1::versioned_get).unwrap();
    api.register(v2::versioned_get).unwrap();

    let api_context = ();
    let server = HttpServerStarter::new_with_versioning(
        &config_dropshot,
        api,
        api_context,
        &log,
        None,
        VersionPolicy::Dynamic(Box::new(BasicVersionPolicy {})),
    )
    .map_err(|error| format!("failed to create server: {}", error))?
    .start();

    server.await
}

#[derive(Debug)]
struct BasicVersionPolicy {}
impl DynamicVersionPolicy for BasicVersionPolicy {
    fn request_extract_version(
        &self,
        request: &Request<Body>,
        _log: &Logger,
    ) -> Result<Version, HttpError> {
        // XXX-dap TODO-cleanup
        let headers = request.headers();
        let v_value =
            headers.get("dropshot-demo-version").ok_or_else(|| {
                HttpError::for_bad_request(
                    None,
                    String::from(
                        "missing expected header \"dropshot-demo-version",
                    ),
                )
            })?;
        let v_str = v_value.to_str().map_err(|_| {
            HttpError::for_bad_request(
                None,
                format!(
                    "bad value for header \"dropshot-demo-version\": \
                     not ASCII: {:?}",
                    v_value
                ),
            )
        })?;
        let v = v_str.parse::<Version>().map_err(|_| {
            HttpError::for_bad_request(
                None,
                format!(
                    "bad value for header \"dropshot-demo-version\": \
                     not a semver: {:?}",
                    v_str
                ),
            )
        })?;
        Ok(v)
    }
}

// HTTP API interface

mod v1 {
    use super::*;

    #[derive(Serialize, JsonSchema)]
    struct Value {
        s: &'static str,
    }

    /// Fetch the current value of the counter.
    #[endpoint {
        method = GET,
        path = "/",
        versions = "1.0.0".."1.0.3"
    }]
    pub async fn versioned_get(
        _rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<Value>, HttpError> {
        Ok(HttpResponseOk(Value { s: "hello from an early v1" }))
    }
}

mod v2 {
    use super::*;

    #[derive(Serialize, JsonSchema)]
    struct Value {
        s: &'static str,
        number: u32,
    }

    /// Fetch the current value of the counter.
    #[endpoint {
        method = GET,
        path = "/",
        versions = "1.0.5"..
    }]
    pub async fn versioned_get(
        _rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<Value>, HttpError> {
        Ok(HttpResponseOk(Value { s: "hello from a LATE v1", number: 12 }))
    }
}
