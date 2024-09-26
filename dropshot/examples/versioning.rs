// Copyright 2024 Oxide Computer Company

//! Example using API versioning
//!
//! This example defines a bunch of API versions:
//!
//! - Versions 1.0.0 through 1.0.3 contain an endpoint `GET /` that returns a
//!   `Value` type with just one field: `s`.
//! - Versions 1.0.5 and later contain an endpoint `GET /` that returns a
//!   `Value` type with two fields: `s` and `number`.
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
//! You'll see that the this spec contains one operation, it produces `Value`,
//! and that the `Value` type only has the one field `s`.
//!
//! You can generate the OpenAPI spec for 1.0.5 and see that the corresponding
//! `Value` type has the extra field `number`, as expected.
//!
//! You can generate the OpenAPI spec for any other version.  You'll see that
//! 0.9.0, for example, has no operations and no `Value` type.
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
//! $ curl http://127.0.0.1:12345
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
//! $ curl -H 'dropshot-demo-version: threeve'  http://127.0.0.1:12345
//! {
//!   "request_id": "18c1964e-88c6-4122-8287-1f2f399871bd",
//!   "message": "bad value for header \"dropshot-demo-version\": unexpected character 't' while parsing major version number: threeve"
//! }
//! ```
//!
//! If we provide version 0.9.0 (or 1.0.4), there is no endpoint at `/`, so we
//! get a 404:
//!
//! ```text
//! $ curl -i -H 'dropshot-demo-version: 0.9.0'  http://127.0.0.1:12345
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
//! $ curl -H 'dropshot-demo-version: 1.0.0'  http://127.0.0.1:12345
//! {"s":"hello from an early v1"}
//! ```
//!
//! If we provide version 1.0.5, we get the later version that we defined, with
//! a different response body type:
//!
//! ```text
//! $ curl -H 'dropshot-demo-version: 1.0.5'  http://127.0.0.1:12345
//! {"s":"hello from a LATE v1","number":12}
//! ```


use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ClientSpecifiesVersionInHeader;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::HttpServerStarter;
use dropshot::RequestContext;
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
    api.register(v1::versioned_get).unwrap();
    api.register(v2::versioned_get).unwrap();

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
        // For this example, we will say that the client always specifies the
        // version using the "dropshot-demo-version" header.  You can provide
        // your own impl of `DynamicVersionPolicy` that does this differently
        // (e.g., filling in a default if the client doesn't provide one).
        let header_name = "dropshot-demo-version"
            .parse::<HeaderName>()
            .map_err(|_| String::from("demo header name was not valid"))?;
        let version_impl = ClientSpecifiesVersionInHeader::new(header_name);
        let version_policy = VersionPolicy::Dynamic(Box::new(version_impl));
        let server = HttpServerStarter::new_with_versioning(
            &config_dropshot,
            api,
            api_context,
            &log,
            None,
            version_policy,
        )
        .map_err(|error| format!("failed to create server: {}", error))?
        .start();

        server.await
    } else {
        Err(String::from("unknown subcommand"))
    }
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
