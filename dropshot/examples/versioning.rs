// Copyright 2024 Oxide Computer Company

//! Example using API versioning
//!
//! This example defines a bunch of API versions:
//!
//! - Versions 1.x contain an endpoint `GET /` that returns a `Thing1` type with
//!   just one field: `thing1_early`.
//! - Versions 2.x and later contain an endpoint `GET /` that returns a `Thing1`
//!   type with one field: `thing1_late`.
//!
//! The client chooses which version they want to use by specifying the
//! `dropshot-demo-version` header with their request.
//!
//! ## Generating OpenAPI specs
//!
//! You can generate the OpenAPI spec for 1.0.0 using:
//!
//! ```text
//! $ cargo run --example=versioning -- openapi 1.0.0
//! ```
//!
//! You'll see that the this spec contains one operation, it produces `Thing1`,
//! and that the `Thing1` type only has the one field `thing1_early`.  It also
//! contains an operation that produces a `Thing2`.
//!
//! You can generate the OpenAPI spec for 2.0.0 and see that the corresponding
//! `Thing1` type has a different field, `thing1_late`, as expected.  `Thing2`
//! is also present and unchanged.
//!
//! You can generate the OpenAPI spec for any other version.  You'll see that
//! 0.9.0, for example, has only the `Thing2` type and its associated getter.
//!
//! ## Running the server
//!
//! Start the Dropshot HTTP server with:
//!
//! ```text
//! $ cargo run --example=versioning -- run
//! ```
//!
//! The server will listen on 127.0.0.1:12345.  You can use `curl` to make
//! requests.  If we don't specify a version, we get an error:
//!
//! ```text
//! $ curl http://127.0.0.1:12345/thing1
//! {
//!   "request_id": "73f62e8a-b363-488a-b662-662814e306ee",
//!   "message": "missing expected header \"dropshot-demo-version\""
//! }
//! ```
//!
//! You can customize this behavior for your Dropshot server, but this one
//! requires that the client specify a version.
//!
//! If we provide a bogus one, we'll also get an error:
//!
//! ```text
//! $ curl -H 'dropshot-demo-version: threeve'  http://127.0.0.1:12345/thing1
//! {
//!   "request_id": "18c1964e-88c6-4122-8287-1f2f399871bd",
//!   "message": "bad value for header \"dropshot-demo-version\": unexpected character 't' while parsing major version number: threeve"
//! }
//! ```
//!
//! If we provide version 0.9.0, there is no endpoint at `/thing1`, so we get a
//! 404:
//!
//! ```text
//! $ curl -i -H 'dropshot-demo-version: 0.9.0'  http://127.0.0.1:12345/thing1
//! HTTP/1.1 404 Not Found
//! content-type: application/json
//! x-request-id: 0d3d25b8-4c48-43b2-a417-018ebce68870
//! content-length: 84
//! date: Thu, 26 Sep 2024 16:55:20 GMT
//!
//! {
//!   "request_id": "0d3d25b8-4c48-43b2-a417-018ebce68870",
//!   "message": "Not Found"
//! }
//! ```
//!
//! If we provide version 1.0.0, we get the v1 handler we defined:
//!
//! ```text
//! $ curl -H 'dropshot-demo-version: 1.0.0'  http://127.0.0.1:12345/thing1
//! {"thing1_early":"hello from an early v1"}
//! ```
//!
//! If we provide version 2.0.0, we get the later version that we defined, with
//! a different response body type:
//!
//! ```text
//! $ curl -H 'dropshot-demo-version: 2.0.0'  http://127.0.0.1:12345/thing1
//! {"thing1_late":"hello from a LATE v1"}
//! ```

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ClientSpecifiesVersionInHeader;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use dropshot::ServerBuilder;
use dropshot::VersionPolicy;
use http::HeaderName;
use schemars::JsonSchema;
use serde::Serialize;

#[tokio::main]
async fn main() -> Result<(), String> {
    // See dropshot/examples/basic.rs for more details on these pieces.
    let config_dropshot = ConfigDropshot {
        bind_address: "127.0.0.1:12345".parse().unwrap(),
        ..Default::default()
    };
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Debug };
    let log = config_logging
        .to_logger("example-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    let mut api = ApiDescription::new();
    api.register(v1::get_thing1).unwrap();
    api.register(v2::get_thing1).unwrap();
    api.register(get_thing2).unwrap();

    // Determine if we're generating the OpenAPI spec or starting the server.
    // Skip the first argument because that's just the name of the program.
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() {
        Err(String::from("expected subcommand: \"run\" or \"openapi\""))
    } else if args[0] == "openapi" {
        if args.len() != 2 {
            return Err(String::from(
                "subcommand \"openapi\": expected exactly one argument",
            ));
        }

        // Write an OpenAPI spec for the requested version.
        let version: semver::Version =
            args[1].parse().map_err(|e| format!("expected semver: {}", e))?;
        let _ = api
            .openapi("Example API with versioning", version)
            .write(&mut std::io::stdout());
        Ok(())
    } else if args[0] == "run" {
        // Run a versioned server.
        let api_context = ();

        // When using API versioning, you must provide a `VersionPolicy` that
        // tells Dropshot how to determine what API version is being used for
        // each incoming request.
        //
        // For this example, we use `ClientSpecifiesVersionInHeader` to tell
        // Dropshot that, as the name suggests, the client always specifies the
        // version using the "dropshot-demo-version" header.  We specify a max
        // API version of "2.0.0".  This is the "current" (latest) API that this
        // example server is intended to support.
        //
        // You can provide your own impl of `DynamicVersionPolicy` that does
        // this differently (e.g., filling in a default if the client doesn't
        // provide one).  See `DynamicVersionPolicy` for details.
        let header_name = "dropshot-demo-version"
            .parse::<HeaderName>()
            .map_err(|_| String::from("demo header name was not valid"))?;
        let max_version = semver::Version::new(2, 0, 0);
        let version_impl =
            ClientSpecifiesVersionInHeader::new(header_name, max_version);
        let version_policy = VersionPolicy::Dynamic(Box::new(version_impl));
        let server = ServerBuilder::new(api, api_context, log)
            .config(config_dropshot)
            .version_policy(version_policy)
            .start()
            .map_err(|error| format!("failed to create server: {}", error))?;

        server.await
    } else {
        Err(String::from("unknown subcommand"))
    }
}

// HTTP API interface
//
// This API defines several different versions:
//
// - versions prior to 1.0.0 do not contain a `get_thing1` endpoint
// - versions 1.0.0 through 2.0.0 (exclusive) use `v1::get_thing1`
// - versions 2.0.0 and later use `v2::get_thing1`
//
// `get_thing2` appears in all versions.

mod v1 {
    // The contents of this module define endpoints and types used in all v1.x
    // versions.

    use super::*;

    #[derive(Serialize, JsonSchema)]
    struct Thing1 {
        thing1_early: &'static str,
    }

    /// Fetch `thing1`
    #[endpoint {
        method = GET,
        path = "/thing1",
        versions = "1.0.0".."2.0.0"
    }]
    pub async fn get_thing1(
        _rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<Thing1>, HttpError> {
        Ok(HttpResponseOk(Thing1 { thing1_early: "hello from an early v1" }))
    }
}

mod v2 {
    // The contents of this module define endpoints and types used in v2.x and
    // later.

    use super::*;

    #[derive(Serialize, JsonSchema)]
    struct Thing1 {
        thing1_late: &'static str,
    }

    /// Fetch `thing1`
    #[endpoint {
        method = GET,
        path = "/thing1",
        versions = "2.0.0"..
    }]
    pub async fn get_thing1(
        _rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<Thing1>, HttpError> {
        Ok(HttpResponseOk(Thing1 { thing1_late: "hello from a LATE v1" }))
    }
}

// The following are used in all API versions.  The delta for each version is
// proportional to what actually changed in each version.  i.e., you don't have
// to repeat the code for all the unchanged endpoints.

#[derive(Serialize, JsonSchema)]
struct Thing2 {
    thing2: &'static str,
}

/// Fetch `thing2`
#[endpoint {
    method = GET,
    path = "/thing2",
}]
pub async fn get_thing2(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<Thing2>, HttpError> {
    Ok(HttpResponseOk(Thing2 { thing2: "hello from any version" }))
}
