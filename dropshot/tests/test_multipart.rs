// Copyright 2023 Oxide Computer Company

//! Test cases for multipart form-data.

use dropshot::test_util::read_string;
use dropshot::{
    endpoint, ApiDescription, HttpError, MultipartBody, RequestContext,
};
use http::{Method, Response, StatusCode};
use hyper::Body;

extern crate slog;

pub mod common;

fn api() -> ApiDescription<usize> {
    let mut api = ApiDescription::new();
    api.register(api_multipart).unwrap();
    api
}

#[endpoint {
    method = POST,
    path = "/upload",
}]
async fn api_multipart(
    _rqctx: RequestContext<usize>,
    mut body: MultipartBody,
) -> Result<Response<Body>, HttpError> {
    // Iterate over the fields, use `next_field()` to get the next field.
    let mut contents = Vec::new();
    while let Some(field) = body.content.next_field().await.unwrap() {
        // Process the field data chunks e.g. store them in a file.
        if let Ok(bytes) = field.bytes().await {
            contents.extend(bytes)
        }
    }

    return Ok(Response::builder()
        .status(StatusCode::OK)
        .body(contents.into())?);
}

#[tokio::test]
async fn test_multipart_client() {
    let api = api();
    let testctx = common::test_setup("multipart_client", api);

    let test_string = "abcd";
    let uri = testctx.client_testctx.url("/upload");
    let request = hyper::Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("Content-Type", "multipart/form-data; boundary=Y-BOUNDARY")
        .body(
            format!(
                "--Y-BOUNDARY\r\n\
                Content-Disposition: form-data; name=\"my_text_field\"\r\n\
                \r\n\
                {}\r\n\
                --Y-BOUNDARY\r\n\
                Content-Disposition: form-data; name=\"my_text_field\"\r\n\
                \r\n\
                {}\r\n\
                --Y-BOUNDARY--\r\n\
                ",
                test_string, test_string,
            )
            .into(),
        )
        .expect("attempted to construct invalid request");
    let mut response = testctx
        .client_testctx
        .make_request_with_request(request, http::StatusCode::OK)
        .await
        .expect("expected success");
    let body = read_string(&mut response).await;
    assert_eq!(body, format!("{}{}", test_string, test_string));

    testctx.teardown().await;
}
