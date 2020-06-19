/*!
 * Test cases for the "demo" handlers.  These handlers exercise various
 * supported configurations of the HTTP handler interface.  We exercise them
 * here to make sure that even if these aren't used at a given point, they still
 * work.
 *
 * Note that the purpose is mainly to exercise the various possible function
 * signatures that can be used to implement handler functions.  We don't need to
 * exercises very many cases (or error cases) of each one because the handlers
 * themselves are not important, but we need to exercise enough to validate that
 * the generic JSON and query parsing handles error cases.
 *
 * TODO-hardening: add test cases that exceed limits (e.g., query string length,
 * JSON body length)
 */

use dropshot::test_util::read_json;
use dropshot::test_util::read_string;
use dropshot::test_util::LogContext;
use dropshot::test_util::TestContext;
use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingIfExists;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::Json;
use dropshot::Path;
use dropshot::Query;
use dropshot::RequestContext;
use dropshot::CONTENT_TYPE_JSON;
use dropshot_endpoint::endpoint;
use dropshot_endpoint::ExtractedParameter;
use http::StatusCode;
use hyper::Body;
use hyper::Method;
use hyper::Response;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

#[macro_use]
extern crate slog;

fn test_setup(test_name: &str) -> TestContext {
    /*
     * The IP address to which we bind can be any local IP, but we use
     * 127.0.0.1 because we know it's present, it shouldn't expose this server
     * on any external network, and we don't have to go looking for some other
     * local IP (likely in a platform-specific way).  We specify port 0 to
     * request any available port.  This is important because we may run
     * multiple concurrent tests, so any fixed port could result in spurious
     * failures due to port conflicts.
     */
    let config_dropshot = ConfigDropshot {
        bind_address: "127.0.0.1:0".parse().unwrap(),
    };

    let config_logging = ConfigLogging::File {
        level: ConfigLoggingLevel::Debug,
        path: "UNUSED".to_string(),
        if_exists: ConfigLoggingIfExists::Fail,
    };

    let mut api = ApiDescription::new();
    register_test_endpoints(&mut api);
    let logctx = LogContext::new(test_name, &config_logging);

    let log = logctx.log.new(o!());
    TestContext::new(
        api,
        Arc::new(0 as usize),
        &config_dropshot,
        Some(logctx),
        log,
    )
}

/*
 * The "demo1" handler consumes neither query nor JSON body parameters.  Here we
 * test that such handlers work.  There are no error cases for us to induce.
 */
#[tokio::test]
async fn test_demo1() {
    let testctx = test_setup("demo1");

    let private = testctx.server.app_private();
    let p = private.downcast::<usize>().expect("wrong type for private data");
    assert_eq!(*p, 0);

    let mut response = testctx
        .client_testctx
        .make_request(
            Method::GET,
            "/testing/demo1",
            None as Option<()>,
            StatusCode::OK,
        )
        .await
        .expect("expected success");
    let body = read_string(&mut response).await;
    assert_eq!(body, "\"demo_handler_args_1\"");
    testctx.teardown().await;
}

/*
 * The "demo2query" handler consumes only query arguments.  Here we make sure
 * such handlers work and also exercise various error cases associated with bad
 * query string parsing.
 * TODO-hardening there are a lot more to check here, particularly around
 * encoded values.
 */
#[tokio::test]
async fn test_demo2query() {
    let testctx = test_setup("demo2query");

    /* Test case: optional field missing */
    let mut response = testctx
        .client_testctx
        .make_request(
            Method::GET,
            "/testing/demo2query?test1=foo",
            None as Option<()>,
            StatusCode::OK,
        )
        .await
        .expect("expected success");
    let json: DemoJsonBody = read_json(&mut response).await;
    assert_eq!(json.test1, "foo");
    assert_eq!(json.test2, None);

    /* Test case: both fields specified */
    let mut response = testctx
        .client_testctx
        .make_request(
            Method::GET,
            "/testing/demo2query?test1=foo&test2=10",
            None as Option<()>,
            StatusCode::OK,
        )
        .await
        .expect("expected success");
    let json: DemoJsonBody = read_json(&mut response).await;
    assert_eq!(json.test1, "foo");
    assert_eq!(json.test2, Some(10));

    /* Test case: required field missing */
    let error = testctx
        .client_testctx
        .make_request(
            Method::GET,
            "/testing/demo2query",
            None as Option<()>,
            StatusCode::BAD_REQUEST,
        )
        .await
        .expect_err("expected failure");
    assert_eq!(
        error.message,
        "unable to parse query string: missing field `test1`"
    );

    /* Test case: typed field has bad value */
    let error = testctx
        .client_testctx
        .make_request(
            Method::GET,
            "/testing/demo2query?test1=foo&test2=bar",
            None as Option<()>,
            StatusCode::BAD_REQUEST,
        )
        .await
        .expect_err("expected failure");
    assert_eq!(
        error.message,
        "unable to parse query string: invalid digit found in string"
    );

    /* Test case: duplicated field name */
    let error = testctx
        .client_testctx
        .make_request(
            Method::GET,
            "/testing/demo2query?test1=foo&test1=bar",
            None as Option<()>,
            StatusCode::BAD_REQUEST,
        )
        .await
        .expect_err("expected failure");
    assert_eq!(
        error.message,
        "unable to parse query string: duplicate field `test1`"
    );

    testctx.teardown().await;
}

/*
 * The "demo2json" handler consumes only a JSON object.  Here we make sure such
 * handlers work and also exercise various error cases associated with bad JSON
 * handling.
 */
#[tokio::test]
async fn test_demo2json() {
    let testctx = test_setup("demo2json");

    /* Test case: optional field */
    let input = DemoJsonBody {
        test1: "bar".to_string(),
        test2: None,
    };
    let mut response = testctx
        .client_testctx
        .make_request(
            Method::GET,
            "/testing/demo2json",
            Some(input),
            StatusCode::OK,
        )
        .await
        .expect("expected success");
    let json: DemoJsonBody = read_json(&mut response).await;
    assert_eq!(json.test1, "bar");
    assert_eq!(json.test2, None);

    /* Test case: both fields populated */
    let input = DemoJsonBody {
        test1: "bar".to_string(),
        test2: Some(15),
    };
    let mut response = testctx
        .client_testctx
        .make_request(
            Method::GET,
            "/testing/demo2json",
            Some(input),
            StatusCode::OK,
        )
        .await
        .expect("expected success");
    let json: DemoJsonBody = read_json(&mut response).await;
    assert_eq!(json.test1, "bar");
    assert_eq!(json.test2, Some(15));

    /* Test case: no input specified */
    let error = testctx
        .client_testctx
        .make_request(
            Method::GET,
            "/testing/demo2json",
            None as Option<()>,
            StatusCode::BAD_REQUEST,
        )
        .await
        .expect_err("expected failure");
    assert!(error.message.starts_with("unable to parse body"));

    /* Test case: invalid JSON */
    let error = testctx
        .client_testctx
        .make_request_with_body(
            Method::GET,
            "/testing/demo2json",
            "}".into(),
            StatusCode::BAD_REQUEST,
        )
        .await
        .expect_err("expected failure");
    assert!(error.message.starts_with("unable to parse body"));

    /* Test case: bad type */
    let json_bad_type = "{ \"test1\": \"oops\", \"test2\": \"oops\" }";
    let error = testctx
        .client_testctx
        .make_request_with_body(
            Method::GET,
            "/testing/demo2json",
            json_bad_type.into(),
            StatusCode::BAD_REQUEST,
        )
        .await
        .expect_err("expected failure");
    assert!(error.message.starts_with(
        "unable to parse body: invalid type: string \"oops\", expected u32"
    ));

    testctx.teardown().await;
}

/*
 * The "demo3" handler takes both query arguments and a JSON body.  This test
 * makes sure that both sets of parameters are received by the handler function
 * and at least one error case from each of those sources is exercised.  We
 * don't need exhaustively re-test the query and JSON error handling paths.
 */
#[tokio::test]
async fn test_demo3json() {
    let testctx = test_setup("demo3json");

    /* Test case: everything filled in. */
    let json_input = DemoJsonBody {
        test1: "bart".to_string(),
        test2: Some(0),
    };

    let mut response = testctx
        .client_testctx
        .make_request(
            Method::GET,
            "/testing/demo3?test1=martin&test2=2",
            Some(json_input),
            StatusCode::OK,
        )
        .await
        .expect("expected success");
    let json: DemoJsonAndQuery = read_json(&mut response).await;
    assert_eq!(json.json.test1, "bart");
    assert_eq!(json.json.test2.unwrap(), 0);
    assert_eq!(json.query.test1, "martin");
    assert_eq!(json.query.test2.unwrap(), 2);

    /* Test case: error parsing query */
    let json_input = DemoJsonBody {
        test1: "bart".to_string(),
        test2: Some(0),
    };
    let error = testctx
        .client_testctx
        .make_request(
            Method::GET,
            "/testing/demo3?test2=2",
            Some(json_input),
            StatusCode::BAD_REQUEST,
        )
        .await
        .expect_err("expected error");
    assert_eq!(
        error.message,
        "unable to parse query string: missing field `test1`"
    );

    /* Test case: error parsing body */
    let error = testctx
        .client_testctx
        .make_request_with_body(
            Method::GET,
            "/testing/demo3?test1=martin&test2=2",
            "}".into(),
            StatusCode::BAD_REQUEST,
        )
        .await
        .expect_err("expected error");
    assert!(error.message.starts_with("unable to parse body"));

    testctx.teardown().await;
}

/*
 * The "demo_path_param_string" handler takes just a single string path
 * parameter.
 */
#[tokio::test]
async fn test_demo_path_param_string() {
    let testctx = test_setup("demo_path_param_string");

    /*
     * Simple error cases.  All of these should produce 404 "Not Found" errors.
     */
    let bad_paths = vec![
        /* missing path parameter (won't match route) */
        "/testing/demo_path_string",
        /* missing path parameter (won't match route) */
        "/testing/demo_path_string/",
        /* missing path parameter (won't match route) */
        "/testing/demo_path_string//",
        /* extra path segment (won't match route) */
        "/testing/demo_path_string/okay/then",
    ];

    for bad_path in bad_paths {
        let error = testctx
            .client_testctx
            .make_request_with_body(
                Method::GET,
                bad_path,
                Body::empty(),
                StatusCode::NOT_FOUND,
            )
            .await
            .unwrap_err();
        assert_eq!(error.message, "Not Found");
    }

    /*
     * Success cases (use the path parameter).
     * TODO-coverage verify whether the values below are encoded correctly in
     * all the right places.  It's a little surprising that they come back
     * encoded.
     */
    let okay_paths = vec![
        ("/testing/demo_path_string/okay", "okay"),
        ("/testing/demo_path_string/okay/", "okay"),
        ("/testing/demo_path_string//okay", "okay"),
        ("/testing/demo_path_string//okay//", "okay"),
        ("/testing/demo_path_string//%7Bevil%7D", "%7Bevil%7D"),
        (
            "/testing/demo_path_string//%7Bsurprisingly_okay",
            "%7Bsurprisingly_okay",
        ),
        (
            "/testing/demo_path_string//surprisingly_okay%7D",
            "surprisingly_okay%7D",
        ),
    ];

    for (okay_path, matched_part) in okay_paths {
        let mut response = testctx
            .client_testctx
            .make_request_with_body(
                Method::GET,
                okay_path,
                Body::empty(),
                StatusCode::OK,
            )
            .await
            .unwrap();
        let json: DemoPathString = read_json(&mut response).await;
        assert_eq!(json.test1, matched_part);
    }
}

/*
 * The "demo_path_param_uuid" handler takes just a single uuid path parameter.
 */
#[tokio::test]
async fn test_demo_path_param_uuid() {
    let testctx = test_setup("demo_path_param_uuid");

    /*
     * Error case: not a valid uuid.  The other error cases are the same as for
     * the string-valued path parameter and they're tested above.
     */
    let error = testctx
        .client_testctx
        .make_request_with_body(
            Method::GET,
            "/testing/demo_path_uuid/abcd",
            Body::empty(),
            StatusCode::BAD_REQUEST,
        )
        .await
        .unwrap_err();
    assert!(error.message.starts_with("bad parameter in URL path:"));

    /*
     * Success case (use the Uuid)
     */
    let uuid_str = "e7de8ccc-8938-43fa-8404-a040a0836ee4";
    let valid_path = format!("/testing/demo_path_uuid/{}", uuid_str);
    let mut response = testctx
        .client_testctx
        .make_request_with_body(
            Method::GET,
            &valid_path,
            Body::empty(),
            StatusCode::OK,
        )
        .await
        .unwrap();
    let json: DemoPathUuid = read_json(&mut response).await;
    assert_eq!(json.test1.to_string(), uuid_str);
}

/*
 * Demo handler functions
 */
pub fn register_test_endpoints(api: &mut ApiDescription) {
    api.register(demo_handler_args_1).unwrap();
    api.register(demo_handler_args_2query).unwrap();
    api.register(demo_handler_args_2json).unwrap();
    api.register(demo_handler_args_3).unwrap();
    api.register(demo_handler_path_param_string).unwrap();
    api.register(demo_handler_path_param_uuid).unwrap();

    /*
     * We don't need to exhaustively test these cases, as they're tested by unit
     * tests.
     */
    let error = api.register(demo_handler_path_param_impossible).unwrap_err();
    assert_eq!(
        error,
        "path parameters are not consumed (different_param_name) and \
         specified parameters do not appear in the path (test1)"
    );
}

#[endpoint {
    method = GET,
    path = "/testing/demo1",
}]
async fn demo_handler_args_1(
    _rqctx: Arc<RequestContext>,
) -> Result<Response<Body>, HttpError> {
    http_echo(&"demo_handler_args_1")
}

#[derive(Serialize, Deserialize, ExtractedParameter)]
pub struct DemoQueryArgs {
    pub test1: String,
    pub test2: Option<u32>,
}
#[endpoint {
    method = GET,
    path = "/testing/demo2query",
}]
async fn demo_handler_args_2query(
    _rqctx: Arc<RequestContext>,
    query: Query<DemoQueryArgs>,
) -> Result<Response<Body>, HttpError> {
    http_echo(&query.into_inner())
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DemoJsonBody {
    pub test1: String,
    pub test2: Option<u32>,
}
#[endpoint {
    method = GET,
    path = "/testing/demo2json",
}]
async fn demo_handler_args_2json(
    _rqctx: Arc<RequestContext>,
    json: Json<DemoJsonBody>,
) -> Result<Response<Body>, HttpError> {
    http_echo(&json.into_inner())
}

#[derive(Deserialize, Serialize, ExtractedParameter)]
pub struct DemoJsonAndQuery {
    pub query: DemoQueryArgs,
    pub json: DemoJsonBody,
}
#[endpoint {
    method = GET,
    path = "/testing/demo3",
}]
async fn demo_handler_args_3(
    _rqctx: Arc<RequestContext>,
    query: Query<DemoQueryArgs>,
    json: Json<DemoJsonBody>,
) -> Result<Response<Body>, HttpError> {
    let combined = DemoJsonAndQuery {
        query: query.into_inner(),
        json: json.into_inner(),
    };
    http_echo(&combined)
}

#[derive(Deserialize, Serialize, ExtractedParameter)]
pub struct DemoPathString {
    pub test1: String,
}
#[endpoint {
    method = GET,
    path = "/testing/demo_path_string/{test1}",
}]
async fn demo_handler_path_param_string(
    _rqctx: Arc<RequestContext>,
    path_params: Path<DemoPathString>,
) -> Result<Response<Body>, HttpError> {
    http_echo(&path_params.into_inner())
}

#[derive(Deserialize, Serialize, ExtractedParameter)]
pub struct DemoPathUuid {
    pub test1: Uuid,
}
#[endpoint {
    method = GET,
    path = "/testing/demo_path_uuid/{test1}",
}]
async fn demo_handler_path_param_uuid(
    _rqctx: Arc<RequestContext>,
    path_params: Path<DemoPathUuid>,
) -> Result<Response<Body>, HttpError> {
    http_echo(&path_params.into_inner())
}

#[derive(Deserialize, Serialize, ExtractedParameter)]
pub struct DemoPathImpossible {
    pub test1: String,
}
#[endpoint {
    method = GET,
    path = "/testing/demo_path_impossible/{different_param_name}",
}]
async fn demo_handler_path_param_impossible(
    _rqctx: Arc<RequestContext>,
    path_params: Path<DemoPathImpossible>,
) -> Result<Response<Body>, HttpError> {
    http_echo(&path_params.into_inner())
}

fn http_echo<T: Serialize>(t: &T) -> Result<Response<Body>, HttpError> {
    Ok(Response::builder()
        .header(http::header::CONTENT_TYPE, CONTENT_TYPE_JSON)
        .status(StatusCode::OK)
        .body(serde_json::to_string(t).unwrap().into())?)
}
