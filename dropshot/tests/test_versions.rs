// Copyright 2024 Oxide Computer Company

//! Exercise the API versioning behavior

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ClientSpecifiesVersionInHeader;
use dropshot::ConfigDropshot;
use dropshot::HttpError;
use dropshot::HttpErrorResponseBody;
use dropshot::HttpResponseOk;
use dropshot::HttpServerStarter;
use dropshot::RequestContext;
use dropshot::VersionPolicy;
use reqwest::Method;
use reqwest::StatusCode;
use semver::Version;

pub mod common;

const VERSION_HEADER_NAME: &str = "dropshot-test-version";
const HANDLER1_MSG: &str = "handler1";
const HANDLER2_MSG: &str = "handler2";
const HANDLER3_MSG: &str = "handler3";
const HANDLER4_MSG: &str = "handler3";
const FORBIDDEN_MSG: &str = "THAT VALUE IS FORBIDDEN!!!";

#[tokio::test]
async fn test_versions() {
    let mut api = ApiDescription::<()>::new();
    api.register(handler1).unwrap();
    api.register(handler2).unwrap();
    api.register(handler3).unwrap();
    api.register(handler4).unwrap();

    let logctx = common::create_log_context("test_versions");
    let server = HttpServerStarter::new_with_versioning(
        &ConfigDropshot::default(),
        api,
        (),
        &logctx.log,
        None,
        VersionPolicy::Dynamic(Box::new(TestVersionPolicy(
            ClientSpecifiesVersionInHeader::new(
                VERSION_HEADER_NAME.parse().unwrap(),
            ),
        ))),
    )
    .unwrap()
    .start();

    let server_addr = server.local_addr();
    let mkurl = |path: &str| format!("http://{}{}", server_addr, path);
    let client = reqwest::Client::new();

    #[derive(Debug)]
    struct TestCase<'a> {
        method: Method,
        path: &'a str,
        header: Option<&'a str>,
        expected_status: StatusCode,
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
        // Versions prior to and including 1.0.0 get "handler1".
        TestCase::new(
            Method::GET,
            "/demo",
            Some("0.9.0"),
            StatusCode::OK,
            HANDLER1_MSG,
        ),
        TestCase::new(
            Method::GET,
            "/demo",
            Some("1.0.0"),
            StatusCode::OK,
            HANDLER1_MSG,
        ),
        // Versions between 1.0.0 and 1.1.0 (non-inclusive) do not exist.
        //
        // Because there's a PUT handler for all versions, the expected error is
        // 405 ("Method Not Allowed"), not 404 ("Not Found").
        TestCase::new(
            Method::GET,
            "/demo",
            Some("1.0.1"),
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
        // Versions between 1.1.0 and 1.3.0 (inclusive) get "handler2".
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
            Some("1.3.0"),
            StatusCode::OK,
            HANDLER2_MSG,
        ),
        // Versions between 1.3.0 and 1.4.0 (exclusive) do not exist.
        // See above for why this is a 405.
        TestCase::new(
            Method::GET,
            "/demo",
            Some("1.3.1"),
            StatusCode::METHOD_NOT_ALLOWED,
            StatusCode::METHOD_NOT_ALLOWED.canonical_reason().unwrap(),
        ),
        TestCase::new(
            Method::GET,
            "/demo",
            Some("1.3.99"),
            StatusCode::METHOD_NOT_ALLOWED,
            StatusCode::METHOD_NOT_ALLOWED.canonical_reason().unwrap(),
        ),
        // Versions after 1.4.0 (inclusive) get "handler3".
        TestCase::new(
            Method::GET,
            "/demo",
            Some("1.4.0"),
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
            let body: String = response.json().await.unwrap();
            assert_eq!(body, t.body_contents);
        }
    }
}

#[endpoint {
    method = GET,
    path = "/demo",
    versions = .."1.0.0",
}]
async fn handler1(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<&'static str>, HttpError> {
    Ok(HttpResponseOk(HANDLER1_MSG))
}

#[endpoint {
    method = GET,
    path = "/demo",
    versions = "1.1.0".."1.3.0",
}]
async fn handler2(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<&'static str>, HttpError> {
    Ok(HttpResponseOk(HANDLER2_MSG))
}

#[endpoint {
    method = GET,
    path = "/demo",
    versions = "1.4.0"..,
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
        request: &http::Request<hyper::Body>,
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
