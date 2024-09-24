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
use std::str::FromStr;

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

/// This impl of `DynamicVersionPolicy` tells Dropshot how to determine the
/// appropriate version number for a request.
#[derive(Debug)]
struct BasicVersionPolicy {}
impl DynamicVersionPolicy for BasicVersionPolicy {
    fn request_extract_version(
        &self,
        request: &Request<Body>,
        _log: &Logger,
    ) -> Result<Version, HttpError> {
        parse_header(request.headers(), "dropshot-demo-version")
    }
}

/// Parses a required header out of a request (producing useful error messages
/// for all failure modes)
fn parse_header<T>(
    headers: &http::HeaderMap,
    header_name: &str,
) -> Result<T, HttpError>
where
    T: FromStr,
    <T as FromStr>::Err: std::fmt::Display,
{
    let v_value = headers.get(header_name).ok_or_else(|| {
        HttpError::for_bad_request(
            None,
            format!("missing expected header {:?}", header_name),
        )
    })?;

    let v_str = v_value.to_str().map_err(|_| {
        HttpError::for_bad_request(
            None,
            format!(
                "bad value for header {:?}: not ASCII: {:?}",
                header_name, v_value
            ),
        )
    })?;

    v_str.parse::<T>().map_err(|e| {
        HttpError::for_bad_request(
            None,
            format!("bad value for header {:?}: {}: {}", header_name, e, v_str),
        )
    })
}

// HTTP API interface
//
// This API defines several different versions:
//
// - versions 1.0.0 through 1.0.3 use `v1::versioned_get`
// - versions 1.0.5 and later use `v2::versioned_get`
// - versions prior to 1.0.0 and version 1.0.4 do not exist

mod v1 {
    // The contents of this module define endpoints and types used in v1.0.0
    // through v1.0.3.

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
    // The contents of this module define endpoints and types used in v1.0.5 and
    // later.

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
