// Copyright 2020 Oxide Computer Company
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

use dropshot::channel;
use dropshot::endpoint;
use dropshot::test_util::object_delete;
use dropshot::test_util::read_json;
use dropshot::test_util::read_string;
use dropshot::test_util::TEST_HEADER_1;
use dropshot::test_util::TEST_HEADER_2;
use dropshot::ApiDescription;
use dropshot::HttpError;
use dropshot::HttpResponseDeleted;
use dropshot::HttpResponseHeaders;
use dropshot::HttpResponseOk;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::Path;
use dropshot::Query;
use dropshot::RequestContext;
use dropshot::TypedBody;
use dropshot::UntypedBody;
use dropshot::WebsocketChannelResult;
use dropshot::WebsocketConnection;
use dropshot::CONTENT_TYPE_JSON;
use futures_util::SinkExt;
use futures_util::StreamExt;
use http::StatusCode;
use hyper::Body;
use hyper::Method;
use hyper::Response;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::sync::Arc;
use tokio_tungstenite::WebSocketStream;
use tungstenite::protocol::Role;
use tungstenite::Message;
use uuid::Uuid;

extern crate slog;

pub mod common;

fn demo_api() -> ApiDescription<usize> {
    let mut api = ApiDescription::<usize>::new();
    api.register(demo_handler_args_1).unwrap();
    api.register(demo_handler_args_2query).unwrap();
    api.register(demo_handler_args_2json).unwrap();
    api.register(demo_handler_args_2urlencoded).unwrap();
    api.register(demo_handler_args_3).unwrap();
    api.register(demo_handler_path_param_string).unwrap();
    api.register(demo_handler_path_param_uuid).unwrap();
    api.register(demo_handler_path_param_u32).unwrap();
    api.register(demo_handler_untyped_body).unwrap();
    api.register(demo_handler_delete).unwrap();
    api.register(demo_handler_headers).unwrap();
    api.register(demo_handler_websocket).unwrap();

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

    api
}

/*
 * The "demo1" handler consumes neither query nor JSON body parameters.  Here we
 * test that such handlers work.  There are no error cases for us to induce.
 */
#[tokio::test]
async fn test_demo1() {
    let api = demo_api();
    let testctx = common::test_setup("demo1", api);

    let private = testctx.server.app_private();
    assert_eq!(*private, 0);

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
    let api = demo_api();
    let testctx = common::test_setup("demo2query", api);

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
    let api = demo_api();
    let testctx = common::test_setup("demo2json", api);

    /* Test case: optional field */
    let input = DemoJsonBody { test1: "bar".to_string(), test2: None };
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
    let input = DemoJsonBody { test1: "bar".to_string(), test2: Some(15) };
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
    assert!(error.message.starts_with("unable to parse JSON body"));

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
    assert!(error.message.starts_with("unable to parse JSON body"));

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
        "unable to parse JSON body: invalid type: string \"oops\", expected u32"
    ));

    testctx.teardown().await;
}

/*
 * Handlers may also accept form/URL-encoded bodies. Here we test such
 * bodies with both valid and invalid encodings.
 */
#[tokio::test]
async fn test_demo2urlencoded() {
    let api = demo_api();
    let testctx = common::test_setup("demo2urlencoded", api);

    /* Test case: optional field */
    let input = DemoJsonBody { test1: "bar".to_string(), test2: None };
    let mut response = testctx
        .client_testctx
        .make_request_url_encoded(
            Method::GET,
            "/testing/demo2urlencoded",
            Some(input),
            StatusCode::OK,
        )
        .await
        .expect("expected success");
    let json: DemoJsonBody = read_json(&mut response).await;
    assert_eq!(json.test1, "bar");
    assert_eq!(json.test2, None);

    /* Test case: both fields populated */
    let input = DemoJsonBody { test1: "baz".to_string(), test2: Some(20) };
    let mut response = testctx
        .client_testctx
        .make_request_url_encoded(
            Method::GET,
            "/testing/demo2urlencoded",
            Some(input),
            StatusCode::OK,
        )
        .await
        .expect("expected success");
    let json: DemoJsonBody = read_json(&mut response).await;
    assert_eq!(json.test1, "baz");
    assert_eq!(json.test2, Some(20));

    /* Error case: wrong content type for endpoint */
    let input = DemoJsonBody { test1: "qux".to_string(), test2: Some(30) };
    let error = testctx
        .client_testctx
        .make_request(
            Method::GET,
            "/testing/demo2urlencoded",
            Some(input),
            StatusCode::BAD_REQUEST,
        )
        .await
        .expect_err("expected failure");
    assert!(error.message.starts_with(
        "expected content type \"application/x-www-form-urlencoded\", \
         got \"application/json\""
    ));

    /* Error case: invalid encoding */
    let error = testctx
        .client_testctx
        .make_request_with_body_url_encoded(
            Method::GET,
            "/testing/demo2urlencoded",
            "test1=oops&test2".into(),
            StatusCode::BAD_REQUEST,
        )
        .await
        .expect_err("expected failure");
    assert!(error.message.starts_with("unable to parse URL-encoded body"));

    /* Error case: bad type */
    let error = testctx
        .client_testctx
        .make_request_with_body_url_encoded(
            Method::GET,
            "/testing/demo2urlencoded",
            "test1=oops&test2=oops".into(),
            StatusCode::BAD_REQUEST,
        )
        .await
        .expect_err("expected failure");
    assert!(error.message.starts_with(
        "unable to parse URL-encoded body: invalid digit found in string"
    ));
}

/*
 * The "demo3" handler takes both query arguments and a JSON body.  This test
 * makes sure that both sets of parameters are received by the handler function
 * and at least one error case from each of those sources is exercised.  We
 * don't need exhaustively re-test the query and JSON error handling paths.
 */
#[tokio::test]
async fn test_demo3json() {
    let api = demo_api();
    let testctx = common::test_setup("demo3json", api);

    /* Test case: everything filled in. */
    let json_input = DemoJsonBody { test1: "bart".to_string(), test2: Some(0) };

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
    let json_input = DemoJsonBody { test1: "bart".to_string(), test2: Some(0) };
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
    assert!(error.message.starts_with("unable to parse JSON body"));

    testctx.teardown().await;
}

/*
 * The "demo_path_param_string" handler takes just a single string path
 * parameter.
 */
#[tokio::test]
async fn test_demo_path_param_string() {
    let api = demo_api();
    let testctx = common::test_setup("demo_path_param_string", api);

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
     */
    let okay_paths = vec![
        ("/testing/demo_path_string/okay", "okay"),
        ("/testing/demo_path_string/okay/", "okay"),
        ("/testing/demo_path_string//okay", "okay"),
        ("/testing/demo_path_string//okay//", "okay"),
        ("/testing/demo_path_string//%7Bevil%7D", "{evil}"),
        (
            "/testing/demo_path_string//%7Bsurprisingly_okay",
            "{surprisingly_okay",
        ),
        (
            "/testing/demo_path_string//surprisingly_okay%7D",
            "surprisingly_okay}",
        ),
        ("/testing/demo_path_string/parent%2Fchild", "parent/child"),
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
        assert_eq!(json.test1.0, matched_part);
    }

    testctx.teardown().await;
}

/*
 * The "demo_path_param_uuid" handler takes just a single uuid path parameter.
 */
#[tokio::test]
async fn test_demo_path_param_uuid() {
    let api = demo_api();
    let testctx = common::test_setup("demo_path_param_uuid", api);

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

    testctx.teardown().await;
}

/*
 * The "demo_path_param_u32" handler takes just a single u32 path parameter.
 */
#[tokio::test]
async fn test_demo_path_param_u32() {
    let api = demo_api();
    let testctx = common::test_setup("demo_path_param_u32", api);

    /*
     * Error case: not a valid u32.  Other error cases are the same as for the
     * string-valued path parameter and they're tested above.
     */
    let error = testctx
        .client_testctx
        .make_request_with_body(
            Method::GET,
            "/testing/demo_path_u32/abcd",
            Body::empty(),
            StatusCode::BAD_REQUEST,
        )
        .await
        .unwrap_err();
    assert!(error.message.starts_with("bad parameter in URL path:"));

    /*
     * Success case (use the number)
     */
    let u32_str = "37";
    let valid_path = format!("/testing/demo_path_u32/{}", u32_str);
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
    let json: DemoPathU32 = read_json(&mut response).await;
    assert_eq!(json.test1, 37);

    testctx.teardown().await;
}

/*
 * Test `UntypedBody`.
 */
#[tokio::test]
async fn test_untyped_body() {
    let api = demo_api();
    let testctx = common::test_setup("test_untyped_body", api);
    let client = &testctx.client_testctx;

    /* Error case: body too large. */
    let big_body = vec![0u8; 1025];
    let error = client
        .make_request_with_body(
            Method::PUT,
            "/testing/untyped_body",
            big_body.into(),
            StatusCode::BAD_REQUEST,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "request body exceeded maximum size of 1024 bytes"
    );

    /* Error case: invalid UTF-8, when parsing as a UTF-8 string. */
    let bad_body = vec![0x80u8; 1];
    let error = client
        .make_request_with_body(
            Method::PUT,
            "/testing/untyped_body?parse_str=true",
            bad_body.clone().into(),
            StatusCode::BAD_REQUEST,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "failed to parse body as UTF-8 string: invalid utf-8 sequence of 1 \
         bytes from index 0"
    );

    /* Success case: invalid UTF-8, when not parsing. */
    let mut response = client
        .make_request_with_body(
            Method::PUT,
            "/testing/untyped_body",
            bad_body.into(),
            StatusCode::OK,
        )
        .await
        .unwrap();
    let json: DemoUntyped = read_json(&mut response).await;
    assert_eq!(json.nbytes, 1);
    assert_eq!(json.as_utf8, None);

    /* Success case: empty body */
    let mut response = client
        .make_request_with_body(
            Method::PUT,
            "/testing/untyped_body?parse_str=true",
            "".into(),
            StatusCode::OK,
        )
        .await
        .unwrap();
    let json: DemoUntyped = read_json(&mut response).await;
    assert_eq!(json.nbytes, 0);
    assert_eq!(json.as_utf8, Some(String::from("")));

    /* Success case: non-empty content */
    let body: Vec<u8> = Vec::from(&b"t\xce\xbcv"[..]);
    let mut response = client
        .make_request_with_body(
            Method::PUT,
            "/testing/untyped_body?parse_str=true",
            body.into(),
            StatusCode::OK,
        )
        .await
        .unwrap();
    let json: DemoUntyped = read_json(&mut response).await;
    assert_eq!(json.nbytes, 4);
    assert_eq!(json.as_utf8, Some(String::from("tμv")));

    testctx.teardown().await;
}

/*
 * Test delete request
 */
#[tokio::test]
async fn test_delete_request() {
    let api = demo_api();
    let testctx = common::test_setup("test_delete_request", api);
    let client = &testctx.client_testctx;

    object_delete(&client, "/testing/delete").await;

    testctx.teardown().await;
}

/*
 * Test response headers
 */
#[tokio::test]
async fn test_header_request() {
    let api = demo_api();
    let testctx = common::test_setup("test_header_request", api);
    let response = testctx
        .client_testctx
        .make_request(
            Method::GET,
            "/testing/headers",
            None as Option<()>,
            StatusCode::NO_CONTENT,
        )
        .await
        .expect("expected success");

    let headers = response
        .headers()
        .get_all(TEST_HEADER_1)
        .iter()
        .map(|v| v.to_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(headers, vec!["howdy"]);

    let headers = response
        .headers()
        .get_all(TEST_HEADER_2)
        .iter()
        .map(|v| v.to_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(headers, vec!["hi", "howdy"]);
}

/*
 * The "test_demo_websocket" handler upgrades to a websocket and exchanges
 * greetings with the client.
 */
#[tokio::test]
async fn test_demo_websocket() {
    let api = demo_api();
    let testctx = common::test_setup("demo_websocket", api);

    let path = format!(
        "ws://{}/testing/websocket",
        testctx.client_testctx.bind_address
    );
    let (mut ws, _resp) = tokio_tungstenite::connect_async(path).await.unwrap();

    ws.send(Message::Text("hello server".to_string())).await.unwrap();
    let msg = ws.next().await.unwrap().unwrap();
    assert_eq!(msg, Message::Text("hello client".to_string()));

    testctx.teardown().await;
}

/*
 * Demo handler functions
 */

type RequestCtx = Arc<RequestContext<usize>>;

#[endpoint {
    method = GET,
    path = "/testing/demo1",
}]
async fn demo_handler_args_1(
    _rqctx: RequestCtx,
) -> Result<Response<Body>, HttpError> {
    http_echo(&"demo_handler_args_1")
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct DemoQueryArgs {
    pub test1: String,
    pub test2: Option<u32>,
}
#[endpoint {
    method = GET,
    path = "/testing/demo2query",
}]
async fn demo_handler_args_2query(
    _rqctx: RequestCtx,
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
    _rqctx: RequestCtx,
    json: TypedBody<DemoJsonBody>,
) -> Result<Response<Body>, HttpError> {
    http_echo(&json.into_inner())
}

#[endpoint {
    method = GET,
    path = "/testing/demo2urlencoded",
    content_type = "application/x-www-form-urlencoded",
}]
async fn demo_handler_args_2urlencoded(
    _rqctx: RequestCtx,
    body: TypedBody<DemoJsonBody>,
) -> Result<Response<Body>, HttpError> {
    http_echo(&body.into_inner())
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct DemoJsonAndQuery {
    pub query: DemoQueryArgs,
    pub json: DemoJsonBody,
}
#[endpoint {
    method = GET,
    path = "/testing/demo3",
}]
async fn demo_handler_args_3(
    _rqctx: RequestCtx,
    query: Query<DemoQueryArgs>,
    json: TypedBody<DemoJsonBody>,
) -> Result<Response<Body>, HttpError> {
    let combined =
        DemoJsonAndQuery { query: query.into_inner(), json: json.into_inner() };
    http_echo(&combined)
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct Test1(String);

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct DemoPathString {
    pub test1: Test1,
}
#[endpoint {
    method = GET,
    path = "/testing/demo_path_string/{test1}",
}]
async fn demo_handler_path_param_string(
    _rqctx: RequestCtx,
    path_params: Path<DemoPathString>,
) -> Result<Response<Body>, HttpError> {
    http_echo(&path_params.into_inner())
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct DemoPathUuid {
    pub test1: Uuid,
}
#[endpoint {
    method = GET,
    path = "/testing/demo_path_uuid/{test1}",
}]
async fn demo_handler_path_param_uuid(
    _rqctx: RequestCtx,
    path_params: Path<DemoPathUuid>,
) -> Result<Response<Body>, HttpError> {
    http_echo(&path_params.into_inner())
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct DemoPathU32 {
    pub test1: u32,
}
#[endpoint {
    method = GET,
    path = "/testing/demo_path_u32/{test1}",
}]
async fn demo_handler_path_param_u32(
    _rqctx: RequestCtx,
    path_params: Path<DemoPathU32>,
) -> Result<Response<Body>, HttpError> {
    http_echo(&path_params.into_inner())
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct DemoUntyped {
    pub nbytes: usize,
    pub as_utf8: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
pub struct DemoUntypedQuery {
    pub parse_str: Option<bool>,
}
#[endpoint {
    method = PUT,
    path = "/testing/untyped_body"
}]
async fn demo_handler_untyped_body(
    _rqctx: Arc<RequestContext<usize>>,
    body: UntypedBody,
    query: Query<DemoUntypedQuery>,
) -> Result<HttpResponseOk<DemoUntyped>, HttpError> {
    let nbytes = body.as_bytes().len();
    let as_utf8 = if query.into_inner().parse_str.unwrap_or(false) {
        Some(String::from(body.as_str()?))
    } else {
        None
    };

    Ok(HttpResponseOk(DemoUntyped { nbytes, as_utf8 }))
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct DemoPathImpossible {
    pub test1: String,
}
#[endpoint {
    method = GET,
    path = "/testing/demo_path_impossible/{different_param_name}",
}]
async fn demo_handler_path_param_impossible(
    _rqctx: Arc<RequestContext<usize>>,
    path_params: Path<DemoPathImpossible>,
) -> Result<Response<Body>, HttpError> {
    http_echo(&path_params.into_inner())
}

#[endpoint {
    method = DELETE,
    path = "/testing/delete",
}]
async fn demo_handler_delete(
    _rqctx: RequestCtx,
) -> Result<HttpResponseDeleted, HttpError> {
    Ok(HttpResponseDeleted())
}

#[endpoint {
    method = GET,
    path = "/testing/headers",
}]
async fn demo_handler_headers(
    _rqctx: RequestCtx,
) -> Result<HttpResponseHeaders<HttpResponseUpdatedNoContent>, HttpError> {
    let mut response =
        HttpResponseHeaders::new_unnamed(HttpResponseUpdatedNoContent());
    let headers = response.headers_mut();
    headers.insert(TEST_HEADER_1, http::header::HeaderValue::from_static("hi"));
    headers
        .insert(TEST_HEADER_1, http::header::HeaderValue::from_static("howdy"));
    headers.append(TEST_HEADER_2, http::header::HeaderValue::from_static("hi"));
    headers
        .append(TEST_HEADER_2, http::header::HeaderValue::from_static("howdy"));
    Ok(response)
}

#[channel {
    protocol = WEBSOCKETS,
    path = "/testing/websocket"
}]
async fn demo_handler_websocket(
    rqctx: RequestCtx,
    upgraded: WebsocketConnection,
) -> WebsocketChannelResult {
    use futures_util::stream::StreamExt;
    let mut ws_stream = WebSocketStream::from_raw_socket(
        upgraded.into_inner(),
        Role::Server,
        None,
    )
    .await;
    ws_stream.send(Message::Text("hello client".to_string())).await.unwrap();
    let msg = ws_stream.next().await.unwrap().unwrap();
    slog::info!(rqctx.log, "{}", msg);
    Ok(())
}

fn http_echo<T: Serialize>(t: &T) -> Result<Response<Body>, HttpError> {
    Ok(Response::builder()
        .header(http::header::CONTENT_TYPE, CONTENT_TYPE_JSON)
        .status(StatusCode::OK)
        .body(serde_json::to_string(t).unwrap().into())?)
}
