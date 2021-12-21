// Copyright 2021 Oxide Computer Company

//! Test cases for streaming reqeusts.

use dropshot::{
    endpoint, ApiDescription, HttpError, RequestContext
};
use http::{Method, Response, StatusCode};
use hyper::Body;
use hyper_staticfile::FileBytesStream;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

#[macro_use]
extern crate slog;

mod common;

fn api() -> ApiDescription<usize> {
    let mut api = ApiDescription::new();
    api.register(api_streaming).unwrap();
    api
}

#[endpoint {
    method = GET,
    path = "/streaming",
}]
async fn api_streaming(
    _rqctx: Arc<RequestContext<usize>>,
) -> Result<Response<Body>, HttpError> {
    let mut file = tempfile::tempfile().map_err(|_ | {
        HttpError::for_bad_request(None, "EBADF".to_string())
    }).map(|f| {
        tokio::fs::File::from_std(f)
    })?;

    let mut buf = [0; 8192];
    for i in 0..255 {
        file.write_all(&buf).await.unwrap();
        buf.fill(i);
    }
    let file_stream = FileBytesStream::new(file);
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(file_stream.into_body())?)
}

#[tokio::test]
async fn test_streaming_basic() {
    let api = api();
    let testctx = common::test_setup("streaming_basic", api);
    let client = &testctx.client_testctx;

    let response = client
        .make_request_no_body(
            Method::GET,
            "/streaming",
            StatusCode::OK,
        )
        .await
        .expect("Expected success");

    let transfer_encoding_header = response
        .headers()
        .get("transfer-encoding")
        .expect("Expected 'transfer-encoding' header to be set on streaming response");

    assert_eq!("foo", transfer_encoding_header);

    testctx.teardown().await;
}
