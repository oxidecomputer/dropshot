// Copyright 2021 Oxide Computer Company

//! Test cases for streaming requests.

use dropshot::Body;
use dropshot::{endpoint, ApiDescription, HttpError, RequestContext};
use http::{Method, Response, StatusCode};
use http_body_util::BodyExt;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::common;

extern crate slog;

pub fn api() -> ApiDescription<usize> {
    let mut api = ApiDescription::new();
    api.register(api_streaming).unwrap();
    api.register(api_not_streaming).unwrap();
    api
}

const BUF_SIZE: usize = 8192;
const BUF_COUNT: usize = 128;

#[endpoint {
    method = GET,
    path = "/streaming",
}]
async fn api_streaming(
    _rqctx: RequestContext<usize>,
) -> Result<Response<Body>, HttpError> {
    let mut file = tempfile::tempfile()
        .map_err(|_| {
            HttpError::for_bad_request(
                None,
                "Cannot create tempfile".to_string(),
            )
        })
        .map(|f| tokio::fs::File::from_std(f))?;

    // Fill the file with some arbitrary contents.
    let mut buf = [0; BUF_SIZE];
    for i in 0..BUF_COUNT {
        file.write_all(&buf).await.unwrap();
        buf.fill((i & 255) as u8);
    }
    file.seek(std::io::SeekFrom::Start(0)).await.unwrap();

    let file_access = hyper_staticfile::vfs::TokioFileAccess::new(file);
    let file_stream = hyper_staticfile::util::FileBytesStream::new(file_access);
    let body = Body::wrap(hyper_staticfile::Body::Full(file_stream));
    Ok(Response::builder().status(StatusCode::OK).body(body)?)
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

fn check_has_transfer_encoding<B>(
    response: &Response<B>,
    expected_value: Option<&str>,
) {
    let transfer_encoding_header = response.headers().get("transfer-encoding");
    match expected_value {
        Some(expected_value) => {
            assert_eq!(
                expected_value,
                transfer_encoding_header.expect("expected value")
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

    let mut response = client
        .make_request_no_body(Method::GET, "/streaming", StatusCode::OK)
        .await
        .expect("Expected GET request to succeed");
    check_has_transfer_encoding(&response, Some("chunked"));

    let mut chunk_count = 0;
    let mut byte_count = 0;
    while let Some(chunk) = response.body_mut().frame().await {
        let chunk = chunk.expect("Should have received chunk without error");
        if let Ok(chunk) = chunk.into_data() {
            byte_count += chunk.len();
            chunk_count += 1;
        }
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
    check_has_transfer_encoding(&response, Some("chunked"));

    let body_bytes = response
        .body_mut()
        .collect()
        .await
        .expect("Error reading body")
        .to_bytes();
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
    check_has_transfer_encoding(&response, None);
    testctx.teardown().await;
}
