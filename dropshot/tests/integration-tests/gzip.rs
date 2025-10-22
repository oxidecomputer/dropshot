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

// Tiny test payload for testing size threshold
#[derive(Deserialize, Serialize, schemars::JsonSchema)]
struct TinyData {
    x: u8,
}

fn api() -> ApiDescription<usize> {
    let mut api = ApiDescription::new();
    api.register(api_large_response).unwrap();
    api.register(api_image_response).unwrap();
    api.register(api_small_response).unwrap();
    api.register(api_disable_compression_response).unwrap();
    api.register(api_json_suffix_response).unwrap();
    api.register(api_xml_suffix_response).unwrap();
    api.register(api_no_content_response).unwrap();
    api.register(api_not_modified_response).unwrap();
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

/// Returns a binary response (image) that should not be compressed
#[endpoint {
    method = GET,
    path = "/image-response",
}]
async fn api_image_response(
    _rqctx: RequestContext<usize>,
) -> Result<Response<dropshot::Body>, HttpError> {
    // Create a fake image response (just random bytes, but large enough)
    let image_data = vec![0u8; 2048]; // 2KB of binary data

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/png")
        .body(dropshot::Body::from(image_data))
        .map_err(|e| HttpError::for_internal_error(e.to_string()))
}

/// Returns a tiny JSON response (under 512 bytes) that should not be compressed
#[endpoint {
    method = GET,
    path = "/small-response",
}]
async fn api_small_response(
    _rqctx: RequestContext<usize>,
) -> Result<HttpResponseOk<TinyData>, HttpError> {
    // Tiny response under 512 bytes threshold: {"x":0} is only 7 bytes
    Ok(HttpResponseOk(TinyData { x: 0 }))
}

/// Returns a large response with compression disabled
#[endpoint {
    method = GET,
    path = "/disable-compression-response",
}]
async fn api_disable_compression_response(
    _rqctx: RequestContext<usize>,
) -> Result<Response<dropshot::Body>, HttpError> {
    // Create a large response
    let repeated_text = "This is some repetitive text. ".repeat(100);
    let data = LargeTestData {
        message: "Large response with compression disabled".to_string(),
        repeated_data: vec![repeated_text; 10],
    };

    let json_body = serde_json::to_vec(&data)
        .map_err(|e| HttpError::for_internal_error(e.to_string()))?;

    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(dropshot::Body::from(json_body))
        .map_err(|e| HttpError::for_internal_error(e.to_string()))?;

    // Disable compression using the NoCompression extension
    response.extensions_mut().insert(dropshot::NoCompression);

    Ok(response)
}

/// Returns a response with application/problem+json content type
#[endpoint {
    method = GET,
    path = "/json-suffix-response",
}]
async fn api_json_suffix_response(
    _rqctx: RequestContext<usize>,
) -> Result<Response<dropshot::Body>, HttpError> {
    let data = LargeTestData {
        message: "Testing +json suffix".to_string(),
        repeated_data: vec!["data".to_string(); 100],
    };

    let json_body = serde_json::to_vec(&data)
        .map_err(|e| HttpError::for_internal_error(e.to_string()))?;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/problem+json")
        .body(dropshot::Body::from(json_body))
        .map_err(|e| HttpError::for_internal_error(e.to_string()))
}

/// Returns a response with application/soap+xml content type
#[endpoint {
    method = GET,
    path = "/xml-suffix-response",
}]
async fn api_xml_suffix_response(
    _rqctx: RequestContext<usize>,
) -> Result<Response<dropshot::Body>, HttpError> {
    let xml_body = "<root>".repeat(100).into_bytes();

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/soap+xml")
        .body(dropshot::Body::from(xml_body))
        .map_err(|e| HttpError::for_internal_error(e.to_string()))
}

/// Returns a 204 No Content response
#[endpoint {
    method = GET,
    path = "/no-content-response",
}]
async fn api_no_content_response(
    _rqctx: RequestContext<usize>,
) -> Result<Response<dropshot::Body>, HttpError> {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(dropshot::Body::empty())
        .map_err(|e| HttpError::for_internal_error(e.to_string()))
}

/// Returns a 304 Not Modified response
#[endpoint {
    method = GET,
    path = "/not-modified-response",
}]
async fn api_not_modified_response(
    _rqctx: RequestContext<usize>,
) -> Result<Response<dropshot::Body>, HttpError> {
    Response::builder()
        .status(StatusCode::NOT_MODIFIED)
        .body(dropshot::Body::empty())
        .map_err(|e| HttpError::for_internal_error(e.to_string()))
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

#[tokio::test]
async fn test_no_compression_for_streaming_responses() {
    // Test that streaming responses are not compressed even when client accepts gzip
    let api = crate::streaming::api();
    let testctx = common::test_setup("no_compression_streaming", api);
    let client = &testctx.client_testctx;

    // Make request with Accept-Encoding: gzip header
    // Note: We can't use make_request_no_body because it doesn't let us set custom headers
    // So we'll use the RequestBuilder pattern used by the client internally
    let uri = client.url("/streaming");
    let request = hyper::Request::builder()
        .method(http::Method::GET)
        .uri(&uri)
        .header(http::header::ACCEPT_ENCODING, "gzip")
        .body(dropshot::Body::empty())
        .expect("Failed to construct request");

    let mut response = client
        .make_request_with_request(request, http::StatusCode::OK)
        .await
        .expect("Streaming request with gzip accept should succeed");

    // Should have chunked transfer encoding
    let transfer_encoding_header = response.headers().get("transfer-encoding");
    assert_eq!(
        Some(&http::HeaderValue::from_static("chunked")),
        transfer_encoding_header,
        "Streaming response should have transfer-encoding: chunked"
    );

    // Should NOT have gzip content encoding even though client accepts it
    assert_eq!(
        response.headers().get(http::header::CONTENT_ENCODING),
        None,
        "Streaming response should not be compressed even with Accept-Encoding: gzip"
    );

    // Consume the body to verify it works (and to allow teardown to proceed)
    let body_bytes = get_response_bytes(&mut response).await;
    assert!(!body_bytes.is_empty(), "Streaming response should have content");

    testctx.teardown().await;
}

#[tokio::test]
async fn test_no_compression_for_non_compressible_content_types() {
    let api = api();
    let testctx = common::test_setup("no_compression_non_compressible", api);
    let client = &testctx.client_testctx;

    // Request an image with Accept-Encoding: gzip
    let uri = client.url("/image-response");
    let request = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .header(header::ACCEPT_ENCODING, "gzip")
        .body(dropshot::Body::empty())
        .expect("Failed to construct request");

    let response = client
        .make_request_with_request(request, StatusCode::OK)
        .await
        .expect("Image request should succeed");

    // Binary content (images) should NOT be compressed
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING),
        None,
        "Binary content (image/png) should not be compressed even with Accept-Encoding: gzip"
    );

    // Verify content-type is correct
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&header::HeaderValue::from_static("image/png")),
        "Content-Type should be image/png"
    );

    testctx.teardown().await;
}

#[tokio::test]
async fn test_compression_disabled_with_extension() {
    let api = api();
    let testctx = common::test_setup("compression_disabled_extension", api);
    let client = &testctx.client_testctx;

    // Request with Accept-Encoding: gzip, but response has NoCompression extension
    let uri = client.url("/disable-compression-response");
    let request = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .header(header::ACCEPT_ENCODING, "gzip")
        .body(dropshot::Body::empty())
        .expect("Failed to construct request");

    let response = client
        .make_request_with_request(request, StatusCode::OK)
        .await
        .expect("Request should succeed");

    // Should NOT be compressed due to NoCompression extension
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING),
        None,
        "Response with NoCompression extension should not be compressed"
    );

    testctx.teardown().await;
}

#[tokio::test]
async fn test_no_compression_below_size_threshold() {
    let api = api();
    let testctx = common::test_setup("no_compression_small_response", api);
    let client = &testctx.client_testctx;

    // Request a tiny response (under 512 bytes) with Accept-Encoding: gzip
    let uri = client.url("/small-response");
    let request = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .header(header::ACCEPT_ENCODING, "gzip")
        .body(dropshot::Body::empty())
        .expect("Failed to construct request");

    let response = client
        .make_request_with_request(request, StatusCode::OK)
        .await
        .expect("Small response request should succeed");

    // Tiny responses (under 512 bytes) should NOT be compressed
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING),
        None,
        "Responses under 512 bytes should not be compressed"
    );

    testctx.teardown().await;
}

#[tokio::test]
async fn test_reject_gzip_with_quality_zero() {
    let api = api();
    let testctx = common::test_setup("reject_gzip_quality_zero", api);
    let client = &testctx.client_testctx;

    // Request with gzip explicitly rejected (q=0)
    let uri = client.url("/large-response");
    let request = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .header(header::ACCEPT_ENCODING, "gzip;q=0, deflate")
        .body(dropshot::Body::empty())
        .expect("Failed to construct request");

    let response = client
        .make_request_with_request(request, StatusCode::OK)
        .await
        .expect("Request should succeed");

    // Should NOT be compressed since gzip has q=0
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING),
        None,
        "Response should not use gzip when client sets q=0 for gzip"
    );

    testctx.teardown().await;
}

#[tokio::test]
async fn test_vary_header_is_set() {
    let api = api();
    let testctx = common::test_setup("vary_header_set", api);
    let client = &testctx.client_testctx;

    // Request with Accept-Encoding: gzip
    let uri = client.url("/large-response");
    let request = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .header(header::ACCEPT_ENCODING, "gzip")
        .body(dropshot::Body::empty())
        .expect("Failed to construct request");

    let response = client
        .make_request_with_request(request, StatusCode::OK)
        .await
        .expect("Request should succeed");

    // Should have Vary: Accept-Encoding header
    assert!(
        response.headers().contains_key(header::VARY),
        "Response should have Vary header"
    );

    let vary_value =
        response.headers().get(header::VARY).unwrap().to_str().unwrap();
    assert!(
        vary_value.to_lowercase().contains("accept-encoding"),
        "Vary header should include Accept-Encoding, got: {}",
        vary_value
    );

    testctx.teardown().await;
}

#[tokio::test]
async fn test_json_suffix_is_compressed() {
    let api = api();
    let testctx = common::test_setup("json_suffix_compressed", api);
    let client = &testctx.client_testctx;

    // Request with Accept-Encoding: gzip for application/problem+json
    let uri = client.url("/json-suffix-response");
    let request = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .header(header::ACCEPT_ENCODING, "gzip")
        .body(dropshot::Body::empty())
        .expect("Failed to construct request");

    let response = client
        .make_request_with_request(request, StatusCode::OK)
        .await
        .expect("Request should succeed");

    // Should be compressed since application/problem+json has +json suffix
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING),
        Some(&header::HeaderValue::from_static("gzip")),
        "Response with +json suffix should be compressed"
    );

    testctx.teardown().await;
}

#[tokio::test]
async fn test_xml_suffix_is_compressed() {
    let api = api();
    let testctx = common::test_setup("xml_suffix_compressed", api);
    let client = &testctx.client_testctx;

    // Request with Accept-Encoding: gzip for application/soap+xml
    let uri = client.url("/xml-suffix-response");
    let request = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .header(header::ACCEPT_ENCODING, "gzip")
        .body(dropshot::Body::empty())
        .expect("Failed to construct request");

    let response = client
        .make_request_with_request(request, StatusCode::OK)
        .await
        .expect("Request should succeed");

    // Should be compressed since application/soap+xml has +xml suffix
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING),
        Some(&header::HeaderValue::from_static("gzip")),
        "Response with +xml suffix should be compressed"
    );

    testctx.teardown().await;
}

#[tokio::test]
async fn test_no_compression_for_204_no_content() {
    let api = api();
    let testctx = common::test_setup("no_compression_204", api);
    let client = &testctx.client_testctx;

    // Request with Accept-Encoding: gzip for 204 response
    let uri = client.url("/no-content-response");
    let request = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .header(header::ACCEPT_ENCODING, "gzip")
        .body(dropshot::Body::empty())
        .expect("Failed to construct request");

    let response = client
        .make_request_with_request(request, StatusCode::NO_CONTENT)
        .await
        .expect("Request should succeed");

    // Should NOT be compressed (204 must not have body)
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING),
        None,
        "204 No Content should not have Content-Encoding header"
    );

    testctx.teardown().await;
}

#[tokio::test]
async fn test_no_compression_for_304_not_modified() {
    let api = api();
    let testctx = common::test_setup("no_compression_304", api);
    let client = &testctx.client_testctx;

    // Request with Accept-Encoding: gzip for 304 response
    let uri = client.url("/not-modified-response");
    let request = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .header(header::ACCEPT_ENCODING, "gzip")
        .body(dropshot::Body::empty())
        .expect("Failed to construct request");

    let response = client
        .make_request_with_request(request, StatusCode::NOT_MODIFIED)
        .await
        .expect("Request should succeed");

    // Should NOT be compressed (304 must not have body)
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING),
        None,
        "304 Not Modified should not have Content-Encoding header"
    );

    testctx.teardown().await;
}

// Note: HEAD request test is omitted from integration tests because Dropshot
// requires explicit HEAD endpoint registration. The HEAD logic is tested via
// unit tests in should_compress_response.

#[tokio::test]
async fn test_compression_config_disabled() {
    // Test that compression is disabled when config.compression = false (default)
    let api = api();
    let config =
        dropshot::ConfigDropshot { compression: false, ..Default::default() };
    let logctx =
        crate::common::create_log_context("compression_config_disabled");
    let log = logctx.log.new(slog::o!());
    let testctx = dropshot::test_util::TestContext::new(
        api,
        0_usize,
        &config,
        Some(logctx),
        log,
    );
    let client = &testctx.client_testctx;

    // Request WITH Accept-Encoding: gzip but compression disabled in config
    let uri = client.url("/large-response");
    let request = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .header(header::ACCEPT_ENCODING, "gzip")
        .body(dropshot::Body::empty())
        .expect("Failed to construct request");

    let response = client
        .make_request_with_request(request, StatusCode::OK)
        .await
        .expect("Request should succeed");

    // Should NOT be compressed due to config.compression = false
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING),
        None,
        "Response should not be compressed when config.compression = false"
    );

    testctx.teardown().await;
}

#[tokio::test]
async fn test_compression_config_enabled() {
    // Test that compression works when config.compression = true
    let api = api();
    let config =
        dropshot::ConfigDropshot { compression: true, ..Default::default() };
    let logctx =
        crate::common::create_log_context("compression_config_enabled");
    let log = logctx.log.new(slog::o!());
    let testctx = dropshot::test_util::TestContext::new(
        api,
        0_usize,
        &config,
        Some(logctx),
        log,
    );
    let client = &testctx.client_testctx;

    // Request WITH Accept-Encoding: gzip and compression enabled in config
    let uri = client.url("/large-response");
    let request = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .header(header::ACCEPT_ENCODING, "gzip")
        .body(dropshot::Body::empty())
        .expect("Failed to construct request");

    let mut response = client
        .make_request_with_request(request, StatusCode::OK)
        .await
        .expect("Request should succeed");

    // Should be compressed since config.compression = true
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING),
        Some(&header::HeaderValue::from_static("gzip")),
        "Response should be compressed when config.compression = true"
    );

    // Verify the response can be decompressed
    let compressed_body = get_response_bytes(&mut response).await;
    let _decompressed = decompress_gzip(&compressed_body); // Should not panic

    testctx.teardown().await;
}
