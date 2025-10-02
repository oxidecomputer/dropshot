// Copyright 2025 Oxide Computer Company

//! Response compression support for Dropshot.

use crate::body::Body;
use async_compression::tokio::bufread::GzipEncoder;
use futures::{StreamExt, TryStreamExt};
use http::{HeaderMap, HeaderValue, Response};
use hyper::body::{Body as HttpBodyTrait, Frame};
use tokio_util::io::{ReaderStream, StreamReader};

/// Marker type for disabling compression on a response.
/// Insert this into response extensions to prevent compression:
/// ```ignore
/// response.extensions_mut().insert(NoCompression);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct NoCompression;

/// Checks if the request accepts gzip encoding based on the Accept-Encoding header.
/// Handles quality values (q parameter) and rejects gzip if q=0.
pub fn accepts_gzip_encoding(headers: &HeaderMap<HeaderValue>) -> bool {
    let Some(accept_encoding) = headers.get(http::header::ACCEPT_ENCODING)
    else {
        return false;
    };

    let Ok(encoding_str) = accept_encoding.to_str() else {
        return false;
    };

    // Parse each encoding directive
    for directive in encoding_str.split(',') {
        let directive = directive.trim();

        // Split on semicolon to separate encoding from parameters
        let mut parts = directive.split(';');
        let encoding = parts.next().unwrap_or("").trim();

        // Check if this is gzip or * (wildcard)
        let is_gzip = encoding.eq_ignore_ascii_case("gzip");
        let is_wildcard = encoding == "*";

        if !is_gzip && !is_wildcard {
            continue;
        }

        // Parse quality value if present
        let mut quality = 1.0;
        for param in parts {
            let param = param.trim();
            if let Some(q_value) = param.strip_prefix("q=") {
                if let Ok(q) = q_value.parse::<f32>() {
                    quality = q;
                }
            }
        }

        // If quality is 0, this encoding is explicitly rejected
        if quality == 0.0 {
            if is_gzip {
                return false;
            }
            // If wildcard is rejected, continue checking for explicit gzip
            continue;
        }

        // Accept gzip if quality > 0
        if is_gzip && quality > 0.0 {
            return true;
        }

        // Accept wildcard with quality > 0 (but keep looking for explicit gzip)
        if is_wildcard && quality > 0.0 {
            // Wildcard matches, but continue to see if gzip is explicitly mentioned
            // We'll return true after checking all directives
        }
    }

    // Check if wildcard was present with non-zero quality
    for directive in encoding_str.split(',') {
        let directive = directive.trim();
        let mut parts = directive.split(';');
        let encoding = parts.next().unwrap_or("").trim();

        if encoding == "*" {
            let mut quality = 1.0;
            for param in parts {
                let param = param.trim();
                if let Some(q_value) = param.strip_prefix("q=") {
                    if let Ok(q) = q_value.parse::<f32>() {
                        quality = q;
                    }
                }
            }
            if quality > 0.0 {
                return true;
            }
        }
    }

    false
}

/// Determines if a response should be compressed with gzip.
pub fn should_compress_response(
    request_method: &http::Method,
    request_headers: &HeaderMap<HeaderValue>,
    response_status: http::StatusCode,
    response_headers: &HeaderMap<HeaderValue>,
    response_extensions: &http::Extensions,
) -> bool {
    // Don't compress responses that must not have a body
    // (1xx informational, 204 No Content, 304 Not Modified)
    if response_status.is_informational()
        || response_status == http::StatusCode::NO_CONTENT
        || response_status == http::StatusCode::NOT_MODIFIED
    {
        return false;
    }

    // Don't compress HEAD requests (they have no body)
    if request_method == http::Method::HEAD {
        return false;
    }

    // Don't compress partial content responses (206)
    // Compressing already-ranged content changes the meaning for clients
    if response_status == http::StatusCode::PARTIAL_CONTENT {
        return false;
    }

    // Don't compress if client doesn't accept gzip
    if !accepts_gzip_encoding(request_headers) {
        return false;
    }

    // Don't compress if already encoded
    if response_headers.contains_key(http::header::CONTENT_ENCODING) {
        return false;
    }

    // Don't compress if explicitly disabled via extension
    if response_extensions.get::<NoCompression>().is_some() {
        return false;
    }

    // Only compress compressible content types (text-based formats)
    // If there's no content-type or it can't be parsed, don't compress
    let Some(content_type) = response_headers.get(http::header::CONTENT_TYPE)
    else {
        return false;
    };

    let Ok(ct_str) = content_type.to_str() else {
        return false;
    };

    // Don't compress Server-Sent Events (SSE) - these are streaming responses
    // where we want low latency, not compression
    if ct_str.starts_with("text/event-stream") {
        return false;
    }

    // Check for standard compressible content types
    let is_compressible = ct_str.starts_with("application/json")
        || ct_str.starts_with("text/")
        || ct_str.starts_with("application/xml")
        || ct_str.starts_with("application/javascript")
        || ct_str.starts_with("application/x-javascript");

    // Also check for structured syntax suffixes (+json, +xml)
    // This handles media types like application/problem+json, application/hal+json, etc.
    // See RFC 6839 for structured syntax suffix registration
    let has_compressible_suffix =
        ct_str.contains("+json") || ct_str.contains("+xml");

    is_compressible || has_compressible_suffix
}

/// Minimum size in bytes for a response to be compressed.
/// Responses smaller than this won't benefit from compression and may actually get larger.
const MIN_COMPRESS_SIZE: u64 = 512; // 512 bytes

/// Applies gzip compression to a response using streaming compression.
/// This function wraps the response body in a gzip encoder that compresses data
/// as it's being sent, avoiding the need to buffer the entire response in memory.
/// If the body has a known exact size smaller than MIN_COMPRESS_SIZE, compression is skipped.
pub fn apply_gzip_compression(response: Response<Body>) -> Response<Body> {
    let (mut parts, body) = response.into_parts();

    // Check the size hint to see if we should skip compression
    let size_hint = body.size_hint();
    if let Some(exact_size) = size_hint.exact() {
        if exact_size == 0 || exact_size < MIN_COMPRESS_SIZE {
            // Body is empty or too small, don't compress
            return Response::from_parts(parts, body);
        }
    }

    // Convert body to a stream of data chunks
    let data_stream = body.into_data_stream();

    // Map errors to io::Error so StreamReader can use them
    let io_stream = data_stream
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));

    // Convert the stream to an AsyncRead using StreamReader
    let async_read = StreamReader::new(io_stream);

    // Wrap in a buffered reader and then a GzipEncoder for streaming compression
    let gzip_encoder = GzipEncoder::new(tokio::io::BufReader::new(async_read));

    // Convert the encoder back to a stream using ReaderStream
    let compressed_stream = ReaderStream::new(gzip_encoder);

    // Convert the stream back to an HTTP body
    let compressed_body = Body::wrap(http_body_util::StreamBody::new(
        compressed_stream.map(|result| {
            result.map(Frame::data).map_err(|e| {
                Box::new(e) as Box<dyn std::error::Error + Send + Sync>
            })
        }),
    ));

    // Add gzip content-encoding header
    parts.headers.insert(
        http::header::CONTENT_ENCODING,
        HeaderValue::from_static("gzip"),
    );

    // Add or update Vary header to include Accept-Encoding
    // This is critical for HTTP caching - caches must not serve compressed
    // responses to clients that don't accept gzip
    if let Some(existing_vary) = parts.headers.get(http::header::VARY) {
        // Vary header exists, append Accept-Encoding if not already present
        if let Ok(vary_str) = existing_vary.to_str() {
            let has_accept_encoding = vary_str
                .split(',')
                .any(|v| v.trim().eq_ignore_ascii_case("accept-encoding"));

            if !has_accept_encoding {
                // Append Accept-Encoding to existing Vary header
                let new_vary = format!("{}, Accept-Encoding", vary_str);
                // Note: If HeaderValue::from_str fails (e.g., malformed header),
                // we silently skip updating the Vary header to preserve existing behavior
                if let Ok(new_vary_value) = HeaderValue::from_str(&new_vary) {
                    parts.headers.insert(http::header::VARY, new_vary_value);
                }
            }
        }
    } else {
        // No Vary header exists, set it to Accept-Encoding
        parts.headers.insert(
            http::header::VARY,
            HeaderValue::from_static("Accept-Encoding"),
        );
    }

    // Remove content-length since we don't know the compressed size
    parts.headers.remove(http::header::CONTENT_LENGTH);

    Response::from_parts(parts, compressed_body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_accepts_gzip_encoding_basic() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip"),
        );
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_with_positive_quality() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip;q=0.8"),
        );
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_rejects_zero_quality() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip;q=0"),
        );
        assert!(!accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_wildcard() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("*"),
        );
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_wildcard_with_quality() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("*;q=0.5"),
        );
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_wildcard_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("*;q=0"),
        );
        assert!(!accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_multiple_encodings() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("deflate, gzip, br"),
        );
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_gzip_takes_precedence_over_wildcard() {
        // Explicit gzip rejection should override wildcard acceptance
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("*;q=1.0, gzip;q=0"),
        );
        assert!(!accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_gzip_acceptance_overrides_wildcard_rejection() {
        // Explicit gzip acceptance should work even if wildcard is rejected
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("*;q=0, gzip;q=1.0"),
        );
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_case_insensitive() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("GZIP"),
        );
        assert!(accepts_gzip_encoding(&headers));

        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("GzIp"),
        );
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_no_header() {
        let headers = HeaderMap::new();
        assert!(!accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_with_spaces() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("deflate , gzip ; q=0.8 , br"),
        );
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_malformed_quality() {
        // If quality parsing fails, should default to 1.0
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip;q=invalid"),
        );
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_should_compress_response_basic() {
        let method = http::Method::GET;
        let mut request_headers = HeaderMap::new();
        request_headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip"),
        );
        let status = http::StatusCode::OK;
        let mut response_headers = HeaderMap::new();
        response_headers.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        let extensions = http::Extensions::new();

        assert!(should_compress_response(
            &method,
            &request_headers,
            status,
            &response_headers,
            &extensions
        ));
    }

    #[test]
    fn test_should_compress_response_head_method() {
        let method = http::Method::HEAD;
        let mut request_headers = HeaderMap::new();
        request_headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip"),
        );
        let status = http::StatusCode::OK;
        let mut response_headers = HeaderMap::new();
        response_headers.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        let extensions = http::Extensions::new();

        assert!(!should_compress_response(
            &method,
            &request_headers,
            status,
            &response_headers,
            &extensions
        ));
    }

    #[test]
    fn test_should_compress_response_no_content() {
        let method = http::Method::GET;
        let mut request_headers = HeaderMap::new();
        request_headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip"),
        );
        let status = http::StatusCode::NO_CONTENT;
        let response_headers = HeaderMap::new();
        let extensions = http::Extensions::new();

        assert!(!should_compress_response(
            &method,
            &request_headers,
            status,
            &response_headers,
            &extensions
        ));
    }

    #[test]
    fn test_should_compress_response_not_modified() {
        let method = http::Method::GET;
        let mut request_headers = HeaderMap::new();
        request_headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip"),
        );
        let status = http::StatusCode::NOT_MODIFIED;
        let response_headers = HeaderMap::new();
        let extensions = http::Extensions::new();

        assert!(!should_compress_response(
            &method,
            &request_headers,
            status,
            &response_headers,
            &extensions
        ));
    }

    #[test]
    fn test_should_compress_response_partial_content() {
        let method = http::Method::GET;
        let mut request_headers = HeaderMap::new();
        request_headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip"),
        );
        let status = http::StatusCode::PARTIAL_CONTENT;
        let mut response_headers = HeaderMap::new();
        response_headers.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        let extensions = http::Extensions::new();

        assert!(!should_compress_response(
            &method,
            &request_headers,
            status,
            &response_headers,
            &extensions
        ));
    }

    #[test]
    fn test_should_compress_response_no_accept_encoding() {
        let method = http::Method::GET;
        let request_headers = HeaderMap::new();
        let status = http::StatusCode::OK;
        let mut response_headers = HeaderMap::new();
        response_headers.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        let extensions = http::Extensions::new();

        assert!(!should_compress_response(
            &method,
            &request_headers,
            status,
            &response_headers,
            &extensions
        ));
    }

    #[test]
    fn test_should_compress_response_already_encoded() {
        let method = http::Method::GET;
        let mut request_headers = HeaderMap::new();
        request_headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip"),
        );
        let status = http::StatusCode::OK;
        let mut response_headers = HeaderMap::new();
        response_headers.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        response_headers.insert(
            http::header::CONTENT_ENCODING,
            HeaderValue::from_static("br"),
        );
        let extensions = http::Extensions::new();

        assert!(!should_compress_response(
            &method,
            &request_headers,
            status,
            &response_headers,
            &extensions
        ));
    }

    #[test]
    fn test_should_compress_response_no_compression_extension() {
        let method = http::Method::GET;
        let mut request_headers = HeaderMap::new();
        request_headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip"),
        );
        let status = http::StatusCode::OK;
        let mut response_headers = HeaderMap::new();
        response_headers.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        let mut extensions = http::Extensions::new();
        extensions.insert(NoCompression);

        assert!(!should_compress_response(
            &method,
            &request_headers,
            status,
            &response_headers,
            &extensions
        ));
    }

    #[test]
    fn test_should_compress_response_no_content_type() {
        let method = http::Method::GET;
        let mut request_headers = HeaderMap::new();
        request_headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip"),
        );
        let status = http::StatusCode::OK;
        let response_headers = HeaderMap::new();
        let extensions = http::Extensions::new();

        assert!(!should_compress_response(
            &method,
            &request_headers,
            status,
            &response_headers,
            &extensions
        ));
    }

    #[test]
    fn test_should_compress_response_sse() {
        let method = http::Method::GET;
        let mut request_headers = HeaderMap::new();
        request_headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip"),
        );
        let status = http::StatusCode::OK;
        let mut response_headers = HeaderMap::new();
        response_headers.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        let extensions = http::Extensions::new();

        assert!(!should_compress_response(
            &method,
            &request_headers,
            status,
            &response_headers,
            &extensions
        ));
    }

    #[test]
    fn test_should_compress_response_compressible_content_types() {
        let method = http::Method::GET;
        let mut request_headers = HeaderMap::new();
        request_headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip"),
        );
        let status = http::StatusCode::OK;
        let extensions = http::Extensions::new();

        // Test various compressible content types
        let compressible_types = vec![
            "application/json",
            "text/plain",
            "text/html",
            "text/css",
            "application/xml",
            "application/javascript",
            "application/x-javascript",
            "application/problem+json",
            "application/hal+json",
            "application/soap+xml",
        ];

        for content_type in compressible_types {
            let mut response_headers = HeaderMap::new();
            response_headers.insert(
                http::header::CONTENT_TYPE,
                HeaderValue::from_str(content_type).unwrap(),
            );

            assert!(
                should_compress_response(
                    &method,
                    &request_headers,
                    status,
                    &response_headers,
                    &extensions
                ),
                "Expected {} to be compressible",
                content_type
            );
        }
    }

    #[test]
    fn test_should_compress_response_non_compressible_content_types() {
        let method = http::Method::GET;
        let mut request_headers = HeaderMap::new();
        request_headers.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip"),
        );
        let status = http::StatusCode::OK;
        let extensions = http::Extensions::new();

        // Test various non-compressible content types
        let non_compressible_types = vec![
            "image/png",
            "image/jpeg",
            "video/mp4",
            "application/pdf",
            "application/zip",
            "application/gzip",
            "application/octet-stream",
        ];

        for content_type in non_compressible_types {
            let mut response_headers = HeaderMap::new();
            response_headers.insert(
                http::header::CONTENT_TYPE,
                HeaderValue::from_str(content_type).unwrap(),
            );

            assert!(
                !should_compress_response(
                    &method,
                    &request_headers,
                    status,
                    &response_headers,
                    &extensions
                ),
                "Expected {} to not be compressible",
                content_type
            );
        }
    }
}
