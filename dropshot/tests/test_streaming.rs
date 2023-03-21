// Copyright 2021 Oxide Computer Company

//! Test cases for streaming requests.

use std::convert::Infallible;

use bytes::Bytes;
use dropshot::{
    endpoint, ApiDescription, HttpError, RequestContext, StreamingBody,
};
use futures::TryStreamExt;
use http::{Method, Response, StatusCode};
use hyper::{body::HttpBody, Body};

extern crate slog;

pub mod common;

fn api() -> ApiDescription<usize> {
    let mut api = ApiDescription::new();
    api.register(api_streaming).unwrap();
    api.register(api_client_streaming).unwrap();
    api.register(api_not_streaming).unwrap();
    api
}

const BUF_SIZE: usize = 8192;
const BUF_COUNT: usize = 128;

fn make_chunked_body(buf_count: usize) -> Body {
    let bytes = Bytes::from(vec![0; BUF_SIZE]);
    // This is cheap -- just a bunch of Arc clones.
    let bufs = vec![bytes; buf_count];
    Body::wrap_stream(futures::stream::iter(
        bufs.into_iter().map(|bytes| Ok::<_, Infallible>(bytes)),
    ))
}

#[endpoint {
    method = GET,
    path = "/streaming",
}]
async fn api_streaming(
    _rqctx: RequestContext<usize>,
) -> Result<Response<Body>, HttpError> {
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(make_chunked_body(BUF_COUNT))?)
}

#[endpoint {
    method = PUT,
    path = "/client_streaming",
    // 8192 * 128 = 1_048_576
    request_body_max_bytes = 1_048_576,
}]
async fn api_client_streaming(
    rqctx: RequestContext<usize>,
    body: StreamingBody,
) -> Result<Response<Body>, HttpError> {
    check_has_transfer_encoding(rqctx.request.headers(), Some("chunked"));

    let nbytes = body
        .into_stream()
        .try_fold(0, |acc, v| futures::future::ok(acc + v.len()))
        .await?;
    assert_eq!(nbytes, BUF_SIZE * BUF_COUNT);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(make_chunked_body(BUF_COUNT))?)
}

#[endpoint {
    method = GET,
    path = "/not-streaming",
}]
async fn api_not_streaming(
    _rqctx: RequestContext<usize>,
) -> Result<Response<Body>, HttpError> {
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(serde_json::to_string("not-streaming").unwrap().into())?)
}

fn check_has_transfer_encoding(
    headers: &http::HeaderMap<http::HeaderValue>,
    expected_value: Option<&str>,
) {
    let transfer_encoding_header = headers.get("transfer-encoding");
    match expected_value {
        Some(expected_value) => {
            assert_eq!(
                expected_value,
                transfer_encoding_header.unwrap_or_else(|| panic!(
                    "expected transfer-encoding to be {}, found None",
                    expected_value
                ))
            );
        }
        None => {
            assert!(transfer_encoding_header.is_none())
        }
    }
}

#[tokio::test]
async fn test_streaming_server_streaming_client() {
    let api = api();
    let testctx = common::test_setup("streaming_server_streaming_client", api);
    let client = &testctx.client_testctx;

    async fn check_chunked_response(mut response: Response<Body>) {
        check_has_transfer_encoding(response.headers(), Some("chunked"));

        let mut chunk_count = 0;
        let mut byte_count = 0;
        while let Some(chunk) = response.body_mut().data().await {
            let chunk =
                chunk.expect("Should have received chunk without error");
            byte_count += chunk.len();
            chunk_count += 1;
        }

        assert!(
            chunk_count >= 2,
            "Expected 2+ chunks for streaming, saw: {}",
            chunk_count
        );
        assert_eq!(
            BUF_SIZE * BUF_COUNT,
            byte_count,
            "Mismatch of sent vs received byte count"
        );
    }

    // Success case: GET without body.
    let response = client
        .make_request_no_body(Method::GET, "/streaming", StatusCode::OK)
        .await
        .expect("Expected GET request to succeed");
    check_chunked_response(response).await;

    // Success case: PUT with streaming body.
    let body = make_chunked_body(BUF_COUNT);
    let response = client
        .make_request_with_body(
            Method::PUT,
            "/client_streaming",
            body,
            StatusCode::OK,
        )
        .await
        .expect("Expected PUT request to succeed");
    check_chunked_response(response).await;

    // Error case: streaming body that's slightly too large.
    let body = make_chunked_body(BUF_COUNT + 1);
    let error = client
        .make_request_with_body(
            Method::PUT,
            "/client_streaming",
            body,
            StatusCode::BAD_REQUEST,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "request body exceeded maximum size of 1048576 bytes"
    );

    testctx.teardown().await;
}

#[tokio::test]
async fn test_streaming_server_buffered_client() {
    let api = api();
    let testctx = common::test_setup("streaming_server_buffered_client", api);
    let client = &testctx.client_testctx;

    let mut response = client
        .make_request_no_body(Method::GET, "/streaming", StatusCode::OK)
        .await
        .expect("Expected GET request to succeed");
    check_has_transfer_encoding(response.headers(), Some("chunked"));

    let body_bytes = hyper::body::to_bytes(response.body_mut())
        .await
        .expect("Error reading body");
    assert_eq!(
        BUF_SIZE * BUF_COUNT,
        body_bytes.len(),
        "Mismatch of sent vs received byte count"
    );

    testctx.teardown().await;
}

#[tokio::test]
async fn test_non_streaming_servers_do_not_use_transfer_encoding() {
    let api = api();
    let testctx = common::test_setup(
        "non_streaming_servers_do_not_use_transfer_encoding",
        api,
    );
    let client = &testctx.client_testctx;

    let response = client
        .make_request_no_body(Method::GET, "/not-streaming", StatusCode::OK)
        .await
        .expect("Expected GET request to succeed");
    check_has_transfer_encoding(response.headers(), None);
    testctx.teardown().await;
}
