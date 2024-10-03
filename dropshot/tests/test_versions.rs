// Copyright 2024 Oxide Computer Company

//! Exercise the API versioning behavior

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ClientSpecifiesVersionInHeader;
use dropshot::HttpError;
use dropshot::HttpErrorResponseBody;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use dropshot::ServerBuilder;
use dropshot::VersionPolicy;
use reqwest::Method;
use reqwest::StatusCode;
use schemars::JsonSchema;
use semver::Version;
use serde::Deserialize;
use serde::Serialize;
use std::io::Cursor;

pub mod common;

const VERSION_HEADER_NAME: &str = "dropshot-test-version";
const HANDLER1_MSG: &str = "handler1";
const HANDLER2_MSG: &str = "handler2";
const HANDLER3_MSG: &str = "handler3";
const HANDLER4_MSG: &str = "handler3";
const FORBIDDEN_MSG: &str = "THAT VALUE IS FORBIDDEN!!!";

fn api() -> ApiDescription<()> {
    let mut api = ApiDescription::new();
    api.register(handler1).unwrap();
    api.register(handler2).unwrap();
    api.register(handler3).unwrap();
    api.register(handler4).unwrap();
    api
}

fn api_to_openapi_string<C: dropshot::ServerContext>(
    api: &ApiDescription<C>,
    name: &str,
    version: &semver::Version,
) -> String {
    let mut contents = Cursor::new(Vec::new());
    api.openapi(name, version.clone()).write(&mut contents).unwrap();
    String::from_utf8(contents.get_ref().to_vec()).unwrap()
}

// This is just here so that we can tell that types are included in the spec iff
// they are referenced by endpoints in that version of the spec.
#[derive(Deserialize, JsonSchema, Serialize)]
struct EarlyReturn {
    real_message: String,
}

#[endpoint {
    method = GET,
    path = "/demo",
    versions = .."1.0.0",
}]
async fn handler1(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<EarlyReturn>, HttpError> {
    Ok(HttpResponseOk(EarlyReturn { real_message: HANDLER1_MSG.to_string() }))
}

#[endpoint {
    method = GET,
    path = "/demo",
    versions = "1.1.0".."1.2.0",
}]
async fn handler2(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<&'static str>, HttpError> {
    Ok(HttpResponseOk(HANDLER2_MSG))
}

#[endpoint {
    method = GET,
    path = "/demo",
    versions = "1.3.0"..,
}]
async fn handler3(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<&'static str>, HttpError> {
    Ok(HttpResponseOk(HANDLER3_MSG))
}

#[endpoint {
    method = PUT,
    path = "/demo",
    versions = ..
}]
async fn handler4(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<&'static str>, HttpError> {
    Ok(HttpResponseOk(HANDLER4_MSG))
}

#[derive(Debug)]
struct TestVersionPolicy(ClientSpecifiesVersionInHeader);
impl dropshot::DynamicVersionPolicy for TestVersionPolicy {
    fn request_extract_version(
        &self,
        request: &http::Request<dropshot::Body>,
        log: &slog::Logger,
    ) -> Result<Version, HttpError> {
        let v = self.0.request_extract_version(request, log)?;
        if v.major == 4 && v.minor == 3 && v.patch == 2 {
            Err(HttpError::for_bad_request(None, String::from(FORBIDDEN_MSG)))
        } else {
            Ok(v)
        }
    }
}

/// Define an API with different versions and run through an exhaustive battery
/// of tests showing that we use the correct handler for each incoming request
/// based on the version requested.
#[tokio::test]
async fn test_versions() {
    let logctx = common::create_log_context("test_versions");
    let server = ServerBuilder::new(api(), (), logctx.log.clone())
        .version_policy(VersionPolicy::Dynamic(Box::new(TestVersionPolicy(
            ClientSpecifiesVersionInHeader::new(
                VERSION_HEADER_NAME.parse().unwrap(),
            ),
        ))))
        .start()
        .unwrap();

    let server_addr = server.local_addr();
    let mkurl = |path: &str| format!("http://{}{}", server_addr, path);
    let client = reqwest::Client::new();

    #[derive(Debug)]
    struct TestCase<'a> {
        /// HTTP method for the request
        method: Method,
        /// HTTP path for the request
        path: &'a str,
        /// Value to pass for our requested version
        /// (string supports providing bad (non-semver) input)
        header: Option<&'a str>,
        /// expected HTTP response status code
        expected_status: StatusCode,
        /// expected HTTP response body contents
        body_contents: String,
    }

    impl<'a> TestCase<'a> {
        fn new<S>(
            method: Method,
            path: &'a str,
            header: Option<&'a str>,
            expected_status: StatusCode,
            body_contents: S,
        ) -> TestCase<'a>
        where
            String: From<S>,
        {
            TestCase {
                method,
                path,
                header,
                expected_status,
                body_contents: String::from(body_contents),
            }
        }
    }

    let test_cases = [
        // Test that errors produced by the version policy get propagated
        // through to the client.
        TestCase::new(
            Method::GET,
            "/demo",
            None,
            StatusCode::BAD_REQUEST,
            format!("missing expected header {:?}", VERSION_HEADER_NAME),
        ),
        TestCase::new(
            Method::GET,
            "/demo",
            Some("not-a-semver"),
            StatusCode::BAD_REQUEST,
            format!(
                "bad value for header {:?}: unexpected character 'n' while \
                 parsing major version number: not-a-semver",
                VERSION_HEADER_NAME
            ),
        ),
        // Versions prior to (not including) 1.0.0 get "handler1".
        TestCase::new(
            Method::GET,
            "/demo",
            Some("0.9.0"),
            StatusCode::OK,
            HANDLER1_MSG,
        ),
        // Versions between 1.0.0 (inclusive) and 1.1.0 (exclusive) do not
        // exist.
        //
        // Because there's a PUT handler for all versions, the expected error is
        // 405 ("Method Not Allowed"), not 404 ("Not Found").
        TestCase::new(
            Method::GET,
            "/demo",
            Some("1.0.0"),
            StatusCode::METHOD_NOT_ALLOWED,
            StatusCode::METHOD_NOT_ALLOWED.canonical_reason().unwrap(),
        ),
        TestCase::new(
            Method::GET,
            "/demo",
            Some("1.0.99"),
            StatusCode::METHOD_NOT_ALLOWED,
            StatusCode::METHOD_NOT_ALLOWED.canonical_reason().unwrap(),
        ),
        // Versions between 1.1.0 (inclusive) and 1.2.0 (exclusive) get
        // "handler2".
        TestCase::new(
            Method::GET,
            "/demo",
            Some("1.1.0"),
            StatusCode::OK,
            HANDLER2_MSG,
        ),
        TestCase::new(
            Method::GET,
            "/demo",
            Some("1.1.99"),
            StatusCode::OK,
            HANDLER2_MSG,
        ),
        // Versions between 1.2.0 (inclusive) and 1.3.0 (exclusive) do not
        // exist.  See above for why this is a 405.
        TestCase::new(
            Method::GET,
            "/demo",
            Some("1.2.0"),
            StatusCode::METHOD_NOT_ALLOWED,
            StatusCode::METHOD_NOT_ALLOWED.canonical_reason().unwrap(),
        ),
        TestCase::new(
            Method::GET,
            "/demo",
            Some("1.2.99"),
            StatusCode::METHOD_NOT_ALLOWED,
            StatusCode::METHOD_NOT_ALLOWED.canonical_reason().unwrap(),
        ),
        // Versions after 1.3.0 (inclusive) get "handler3".
        TestCase::new(
            Method::GET,
            "/demo",
            Some("1.3.0"),
            StatusCode::OK,
            HANDLER3_MSG,
        ),
        // For all of these versions, a PUT should get handler4.
        TestCase::new(
            Method::PUT,
            "/demo",
            Some("0.9.0"),
            StatusCode::OK,
            HANDLER4_MSG,
        ),
        TestCase::new(
            Method::PUT,
            "/demo",
            Some("1.0.0"),
            StatusCode::OK,
            HANDLER4_MSG,
        ),
        TestCase::new(
            Method::PUT,
            "/demo",
            Some("1.1.0"),
            StatusCode::OK,
            HANDLER4_MSG,
        ),
        TestCase::new(
            Method::PUT,
            "/demo",
            Some("1.3.1"),
            StatusCode::OK,
            HANDLER4_MSG,
        ),
        TestCase::new(
            Method::PUT,
            "/demo",
            Some("1.4.0"),
            StatusCode::OK,
            HANDLER4_MSG,
        ),
    ];

    for t in test_cases {
        println!("test case: {:?}", t);
        let mut request = client.request(t.method, mkurl(t.path));
        if let Some(h) = t.header {
            request = request.header(VERSION_HEADER_NAME, h);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), t.expected_status);

        if !t.expected_status.is_success() {
            let error: HttpErrorResponseBody = response.json().await.unwrap();
            assert_eq!(error.message, t.body_contents);
        } else {
            // This is ugly.  But it's concise!
            //
            // We want to use a different type for `handler1` just so that we
            // can check in test_versions_openapi() that it appears in the
            // OpenAPI spec only in the appropriate versions.  So if we're
            // expecting to get something back from handler1, then parse the
            // type a little differently.
            let body: String = if t.body_contents == HANDLER1_MSG {
                let body: EarlyReturn = response.json().await.unwrap();
                body.real_message
            } else {
                response.json().await.unwrap()
            };

            assert_eq!(body, t.body_contents);
        }
    }
}

/// Test that the generated OpenAPI spec only refers to handlers in that version
/// and types that are used by those handlers.
#[test]
fn test_versions_openapi() {
    let api = api();

    for version in ["0.9.0", "1.0.0", "1.1.0", "1.3.1", "1.4.0"] {
        let semver: semver::Version = version.parse().unwrap();
        let mut found = Cursor::new(Vec::new());
        api.openapi("Evolving API", semver).write(&mut found).unwrap();
        let actual = std::str::from_utf8(found.get_ref()).unwrap();
        expectorate::assert_contents(
            &format!("tests/test_openapi_v{}.json", version),
            actual,
        );
    }
}

/// Test three different ways to define the same operation in two versions
/// (using different handlers).  These should all produce the same pair of
/// specs.
#[test]
fn test_versions_openapi_same_names() {
    // This approach uses freestanding functions in separate modules.
    let api_function_modules = {
        mod v1 {
            use super::*;

            #[derive(JsonSchema, Serialize)]
            pub struct MyReturn {
                #[allow(dead_code)]
                q: String,
            }

            #[endpoint {
                method = GET,
                path = "/demo",
                versions = .."2.0.0"
            }]
            pub async fn the_operation(
                _rqctx: RequestContext<()>,
            ) -> Result<HttpResponseOk<MyReturn>, HttpError> {
                unimplemented!();
            }
        }

        mod v2 {
            use super::*;

            #[derive(JsonSchema, Serialize)]
            pub struct MyReturn {
                #[allow(dead_code)]
                r: String,
            }

            #[endpoint {
                method = GET,
                path = "/demo",
                versions = "2.0.0"..
            }]
            pub async fn the_operation(
                _rqctx: RequestContext<()>,
            ) -> Result<HttpResponseOk<MyReturn>, HttpError> {
                unimplemented!();
            }
        }

        let mut api = ApiDescription::new();
        api.register(v1::the_operation).unwrap();
        api.register(v2::the_operation).unwrap();
        api
    };

    // This approach uses freestanding functions and types all in one module.
    // This requires applying overrides to the names in order to have them show
    // up with the same name in each version.
    let api_function_overrides = {
        #[derive(JsonSchema, Serialize)]
        #[schemars(rename = "MyReturn")]
        struct MyReturnV1 {
            #[allow(dead_code)]
            q: String,
        }

        #[endpoint {
            method = GET,
            path = "/demo",
            versions = .."2.0.0",
            operation_id = "the_operation",
        }]
        async fn the_operation_v1(
            _rqctx: RequestContext<()>,
        ) -> Result<HttpResponseOk<MyReturnV1>, HttpError> {
            unimplemented!();
        }

        #[derive(JsonSchema, Serialize)]
        #[schemars(rename = "MyReturn")]
        struct MyReturnV2 {
            #[allow(dead_code)]
            r: String,
        }

        #[endpoint {
            method = GET,
            path = "/demo",
            versions = "2.0.0"..,
            operation_id = "the_operation"
        }]
        async fn the_operation_v2(
            _rqctx: RequestContext<()>,
        ) -> Result<HttpResponseOk<MyReturnV2>, HttpError> {
            unimplemented!();
        }

        let mut api = ApiDescription::new();
        api.register(the_operation_v1).unwrap();
        api.register(the_operation_v2).unwrap();
        api
    };

    // This approach uses the trait-based interface, which requires using
    // `operation_id` to override the operation name if you want the name to be
    // the same across versions.
    let api_trait_overrides =
        trait_based::my_api_mod::stub_api_description().unwrap();

    const NAME: &str = "An API";
    let v1 = semver::Version::new(1, 0, 0);
    let v2 = semver::Version::new(2, 0, 0);
    let func_mods_v1 = api_to_openapi_string(&api_function_modules, NAME, &v1);
    let func_mods_v2 = api_to_openapi_string(&api_function_modules, NAME, &v2);
    let func_overrides_v1 =
        api_to_openapi_string(&api_function_overrides, NAME, &v1);
    let func_overrides_v2 =
        api_to_openapi_string(&api_function_overrides, NAME, &v2);
    let traits_v1 = api_to_openapi_string(&api_trait_overrides, NAME, &v1);
    let traits_v2 = api_to_openapi_string(&api_trait_overrides, NAME, &v2);

    expectorate::assert_contents(
        "tests/test_openapi_overrides_v1.json",
        &func_overrides_v1,
    );
    expectorate::assert_contents(
        "tests/test_openapi_overrides_v2.json",
        &func_overrides_v2,
    );

    assert_eq!(func_mods_v1, func_overrides_v1);
    assert_eq!(func_mods_v1, traits_v1);
    assert_eq!(func_mods_v2, func_overrides_v2);
    assert_eq!(func_mods_v2, traits_v2);
}

// The contents of this module logically belongs inside
// test_versions_openapi_same_names().  It can't go there due to
// oxidecomputer/dropshot#1128.
mod trait_based {
    use super::*;

    #[derive(JsonSchema, Serialize)]
    #[schemars(rename = "MyReturn")]
    pub struct MyReturnV1 {
        #[allow(dead_code)]
        q: String,
    }

    #[derive(JsonSchema, Serialize)]
    #[schemars(rename = "MyReturn")]
    pub struct MyReturnV2 {
        #[allow(dead_code)]
        r: String,
    }

    #[dropshot::api_description]
    // This `allow(dead_code)` works around oxidecomputer/dropshot#1129.
    #[allow(dead_code)]
    pub trait MyApi {
        type Context;

        #[endpoint {
            method = GET,
            path = "/demo",
            versions = .."2.0.0",
            operation_id = "the_operation",
        }]
        async fn the_operation_v1(
            _rqctx: RequestContext<Self::Context>,
        ) -> Result<HttpResponseOk<MyReturnV1>, HttpError>;

        #[endpoint {
            method = GET,
            path = "/demo",
            versions = "2.0.0"..,
            operation_id = "the_operation"
        }]
        async fn the_operation_v2(
            _rqctx: RequestContext<Self::Context>,
        ) -> Result<HttpResponseOk<MyReturnV2>, HttpError>;
    }
}
