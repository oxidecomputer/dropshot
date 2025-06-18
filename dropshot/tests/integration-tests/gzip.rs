// Copyright 2025 Oxide Computer Company

//! Test cases for gzip response compression.

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use http::{header, Method, StatusCode};
use hyper::{Request, Response};
use serde::{Deserialize, Serialize};

use crate::common;

extern crate slog;

// Test payload that's large enough to benefit from compression
#[derive(Deserialize, Serialize, schemars::JsonSchema)]
struct LargeTestData {
    message: String,
    repeated_data: Vec<String>,
}

fn api() -> ApiDescription<usize> {
    let mut api = ApiDescription::new();
    api.register(api_large_response).unwrap();
    api
}

/// Returns a large JSON response that should compress well
#[endpoint {
    method = GET,
    path = "/large-response",
}]
async fn api_large_response(
    _rqctx: RequestContext<usize>,
) -> Result<HttpResponseOk<LargeTestData>, HttpError> {
    // Create a response with repeated data that will compress well
    let repeated_text = "This is some repetitive text that should compress very well with gzip compression. ".repeat(50);
    let repeated_data = vec![repeated_text; 100]; // Make it quite large

    Ok(HttpResponseOk(LargeTestData {
        message: "This is a large response for testing gzip compression"
            .to_string(),
        repeated_data,
    }))
}

async fn get_response_bytes(
    response: &mut Response<dropshot::Body>,
) -> Vec<u8> {
    use http_body_util::BodyExt;

    let body_bytes = response
        .body_mut()
        .collect()
        .await
        .expect("Error reading response body")
        .to_bytes();

    body_bytes.to_vec()
}

fn decompress_gzip(compressed_data: &[u8]) -> Vec<u8> {
    use std::io::Read;

    let mut decoder = flate2::read::GzDecoder::new(compressed_data);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .expect("Failed to decompress gzip data");
    decompressed
}

#[tokio::test]
async fn test_gzip_compression_with_accept_encoding() {
    let api = api();
    let testctx = common::test_setup("gzip_compression_accept_encoding", api);
    let client = &testctx.client_testctx;

    // Make request WITHOUT Accept-Encoding: gzip header
    let uri = client.url("/large-response");
    let request_no_gzip = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .body(dropshot::Body::empty())
        .expect("Failed to construct request");

    let mut response_no_gzip = client
        .make_request_with_request(request_no_gzip, StatusCode::OK)
        .await
        .expect("Request without gzip should succeed");

    // Make request WITH Accept-Encoding: gzip header
    let request_with_gzip = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .header(header::ACCEPT_ENCODING, "gzip")
        .body(dropshot::Body::empty())
        .expect("Failed to construct request");

    let mut response_with_gzip = client
        .make_request_with_request(request_with_gzip, StatusCode::OK)
        .await
        .expect("Request with gzip should succeed");

    // Get response bodies
    let uncompressed_body = get_response_bytes(&mut response_no_gzip).await;
    let compressed_body = get_response_bytes(&mut response_with_gzip).await;

    // When gzip is implemented, the gzipped response should:
    // 1. Have Content-Encoding: gzip header
    assert_eq!(
        response_with_gzip.headers().get(header::CONTENT_ENCODING),
        Some(&header::HeaderValue::from_static("gzip")),
        "Response with Accept-Encoding: gzip should have Content-Encoding: gzip header"
    );

    // 2. Be smaller than the uncompressed response
    assert!(
        compressed_body.len() < uncompressed_body.len(),
        "Gzipped response ({} bytes) should be smaller than uncompressed response ({} bytes)",
        compressed_body.len(),
        uncompressed_body.len()
    );

    // 3. When decompressed, should match the original response
    let decompressed_body = decompress_gzip(&compressed_body);
    assert_eq!(
        decompressed_body, uncompressed_body,
        "Decompressed gzip response should match uncompressed response"
    );

    // The response without Accept-Encoding should NOT have Content-Encoding header
    assert_eq!(
        response_no_gzip.headers().get(header::CONTENT_ENCODING),
        None,
        "Response without Accept-Encoding: gzip should not have Content-Encoding header"
    );

    testctx.teardown().await;
}

#[tokio::test]
async fn test_gzip_compression_accepts_multiple_encodings() {
    let api = api();
    let testctx =
        common::test_setup("gzip_compression_multiple_encodings", api);
    let client = &testctx.client_testctx;

    // Test that gzip works when client accepts multiple encodings including gzip
    let uri = client.url("/large-response");
    let request = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .header(header::ACCEPT_ENCODING, "deflate, gzip, br")
        .body(dropshot::Body::empty())
        .expect("Failed to construct request");

    let mut response = client
        .make_request_with_request(request, StatusCode::OK)
        .await
        .expect("Request with multiple accept encodings should succeed");

    // Should still use gzip compression
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING),
        Some(&header::HeaderValue::from_static("gzip")),
        "Response should use gzip when it's one of multiple accepted encodings"
    );

    // Verify the response can be decompressed
    let compressed_body = get_response_bytes(&mut response).await;
    let _decompressed = decompress_gzip(&compressed_body); // Should not panic

    testctx.teardown().await;
}

#[tokio::test]
async fn test_no_gzip_without_accept_encoding() {
    let api = api();
    let testctx = common::test_setup("no_gzip_without_accept", api);
    let client = &testctx.client_testctx;

    // Request without any Accept-Encoding header should not get compressed response
    let response = client
        .make_request_no_body(Method::GET, "/large-response", StatusCode::OK)
        .await
        .expect("Request without accept encoding should succeed");

    // Should not have Content-Encoding header
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING),
        None,
        "Response without Accept-Encoding should not be compressed"
    );

    testctx.teardown().await;
}
