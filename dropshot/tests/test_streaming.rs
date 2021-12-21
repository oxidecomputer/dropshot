// Copyright 2021 Oxide Computer Company

//! Test cases for streaming reqeusts.

use dropshot::{endpoint, ApiDescription, HttpError, RequestContext};
use http::{Method, Response, StatusCode};
use hyper::{body::HttpBody, Body};
use hyper_staticfile::FileBytesStream;
use std::sync::Arc;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

#[macro_use]
extern crate slog;

mod common;

fn api() -> ApiDescription<usize> {
    let mut api = ApiDescription::new();
    api.register(api_streaming).unwrap();
    api
}

const BUF_SIZE: usize = 8192;
const BUF_COUNT: usize = 128;

#[endpoint {
    method = GET,
    path = "/streaming",
}]
async fn api_streaming(
    _rqctx: Arc<RequestContext<usize>>,
) -> Result<Response<Body>, HttpError> {
    let mut file = tempfile::tempfile()
        .map_err(|_| HttpError::for_bad_request(None, "EBADF".to_string()))
        .map(|f| tokio::fs::File::from_std(f))?;

    // Fill the file with some arbitrary contents.
    let mut buf = [0; BUF_SIZE];
    for i in 0..BUF_COUNT {
        file.write_all(&buf).await.unwrap();
        buf.fill((i & 255) as u8);
    }
    file.seek(std::io::SeekFrom::Start(0)).await.unwrap();

    let file_stream = FileBytesStream::new(file);
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(file_stream.into_body())?)
}

fn check_has_chunked_headers(response: &Response<Body>) {
    let transfer_encoding_header =
        response.headers().get("transfer-encoding").expect(
            "Expected 'transfer-encoding' header to be set on streaming \
             response",
        );
    assert_eq!("chunked", transfer_encoding_header);
}

#[tokio::test]
async fn test_streaming_server_streaming_client() {
    let api = api();
    let testctx = common::test_setup("streaming_server_streaming_client", api);
    let client = &testctx.client_testctx;

    let mut response = client
        .make_request_no_body(Method::GET, "/streaming", StatusCode::OK)
        .await
        .expect("Expected GET request to succeed");
    check_has_chunked_headers(&response);

    let mut chunk_count = 0;
    let mut byte_count = 0;
    while let Some(chunk) = response.body_mut().data().await {
        let chunk = chunk.expect("Should have received chunk without error");
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
    check_has_chunked_headers(&response);

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
