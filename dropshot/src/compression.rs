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

/// Parses the `Accept-Encoding` header into a list of encodings and their
/// associated quality factors. Returns the encoding names in lowercase for
/// easier comparisons.
fn parse_accept_encoding(header: &HeaderValue) -> Vec<(String, f32)> {
    const DEFAULT_QUALITY: f32 = 1.0;

    let Ok(header_value) = header.to_str() else {
        return Vec::new();
    };

    header_value
        .split(',')
        .filter_map(|directive| {
            let mut parts = directive.trim().split(';');
            let encoding = parts.next()?.trim();
            if encoding.is_empty() {
                return None;
            }

            let mut quality = DEFAULT_QUALITY;
            for param in parts {
                let mut param = param.splitn(2, '=');
                let name = param.next()?.trim();
                let value = param.next()?.trim();

                if name.eq_ignore_ascii_case("q") {
                    if let Ok(parsed) = value.parse::<f32>() {
                        quality = parsed.clamp(0.0, 1.0);
                    }
                }
            }

            Some((encoding.to_ascii_lowercase(), quality))
        })
        .collect()
}

/// Checks if the request accepts gzip encoding based on the Accept-Encoding header.
/// Handles quality values (q parameter) using RFC-compliant preference rules.
pub fn accepts_gzip_encoding(headers: &HeaderMap<HeaderValue>) -> bool {
    let Some(accept_encoding) = headers.get(http::header::ACCEPT_ENCODING)
    else {
        return false;
    };

    let mut best_gzip_quality: Option<f32> = None;
    let mut best_wildcard_quality: Option<f32> = None;

    // RFC 9110 §12.5.3 specifies that the most preferred (highest quality)
    // representation wins, so we retain the maximum q-value we see for each
    // relevant coding.
    for (encoding, quality) in parse_accept_encoding(accept_encoding) {
        match encoding.as_str() {
            "gzip" => {
                best_gzip_quality = Some(
                    best_gzip_quality
                        .map_or(quality, |current| current.max(quality)),
                );
            }
            "*" => {
                best_wildcard_quality = Some(
                    best_wildcard_quality
                        .map_or(quality, |current| current.max(quality)),
                );
            }
            _ => {}
        }
    }

    if let Some(quality) = best_gzip_quality {
        return quality > 0.0;
    }

    if let Some(quality) = best_wildcard_quality {
        return quality > 0.0;
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
    // Responses that must not have a body per HTTP spec
    if response_status.is_informational()
        || response_status == http::StatusCode::NO_CONTENT
        || response_status == http::StatusCode::NOT_MODIFIED
    {
        return false;
    }

    // HEAD responses have no body
    if request_method == http::Method::HEAD {
        return false;
    }

    // Compressing partial content changes the meaning for clients
    if response_status == http::StatusCode::PARTIAL_CONTENT {
        return false;
    }

    if response_headers.contains_key(http::header::CONTENT_RANGE) {
        return false;
    }

    if !accepts_gzip_encoding(request_headers) {
        return false;
    }

    if response_headers.contains_key(http::header::CONTENT_ENCODING) {
        return false;
    }

    if response_extensions.get::<NoCompression>().is_some() {
        return false;
    }

    if let Some(content_length) =
        response_headers.get(http::header::CONTENT_LENGTH)
    {
        if let Ok(length_str) = content_length.to_str() {
            if let Ok(length) = length_str.parse::<u64>() {
                if length < MIN_COMPRESS_SIZE {
                    return false;
                }
            }
        }
    }

    // Only compress when we know the content type
    let Some(content_type) = response_headers.get(http::header::CONTENT_TYPE)
    else {
        return false;
    };
    let Ok(ct_str) = content_type.to_str() else {
        return false;
    };

    let ct_lower = ct_str.to_ascii_lowercase();

    // SSE streams prioritize latency over compression
    if ct_lower.starts_with("text/event-stream") {
        return false;
    }

    let is_compressible = ct_lower.starts_with("application/json")
        || ct_lower.starts_with("text/")
        || ct_lower.starts_with("application/xml")
        || ct_lower.starts_with("application/javascript")
        || ct_lower.starts_with("application/x-javascript");

    // RFC 6839 structured syntax suffixes (+json, +xml)
    let has_compressible_suffix =
        ct_lower.contains("+json") || ct_lower.contains("+xml");

    is_compressible || has_compressible_suffix
}

/// Minimum size in bytes for a response to be compressed.
/// Responses smaller than this won't benefit from compression and may actually get larger.
const MIN_COMPRESS_SIZE: u64 = 512;

/// Applies gzip compression to a response using streaming compression.
/// This function wraps the response body in a gzip encoder that compresses data
/// as it's being sent, avoiding the need to buffer the entire response in memory.
/// If the body has a known exact size smaller than MIN_COMPRESS_SIZE, compression is skipped.
pub fn apply_gzip_compression(response: Response<Body>) -> Response<Body> {
    let (mut parts, body) = response.into_parts();

    let size_hint = body.size_hint();
    if let Some(exact_size) = size_hint.exact() {
        if exact_size == 0 || exact_size < MIN_COMPRESS_SIZE {
            return Response::from_parts(parts, body);
        }
    }

    // Transform body into a compressed stream:
    // Body -> Stream<Bytes> -> AsyncRead -> GzipEncoder -> Stream<Bytes> -> Body
    let data_stream = body.into_data_stream();
    let io_stream = data_stream
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
    let async_read = StreamReader::new(io_stream);
    let gzip_encoder = GzipEncoder::new(tokio::io::BufReader::new(async_read));
    let compressed_stream = ReaderStream::new(gzip_encoder);

    let compressed_body = Body::wrap(http_body_util::StreamBody::new(
        compressed_stream.map(|result| {
            result.map(Frame::data).map_err(|e| {
                Box::new(e) as Box<dyn std::error::Error + Send + Sync>
            })
        }),
    ));

    parts.headers.insert(
        http::header::CONTENT_ENCODING,
        HeaderValue::from_static("gzip"),
    );

    // Vary header is critical for caching - prevents serving compressed
    // responses to clients that don't accept gzip
    let vary_has_accept_encoding = parts
        .headers
        .get_all(http::header::VARY)
        .iter()
        .any(header_value_contains_accept_encoding);

    if !vary_has_accept_encoding {
        parts.headers.append(
            http::header::VARY,
            HeaderValue::from_static("Accept-Encoding"),
        );
    }

    parts.headers.remove(http::header::ACCEPT_RANGES);
    parts.headers.remove(http::header::CONTENT_LENGTH);

    Response::from_parts(parts, compressed_body)
}

fn header_value_contains_accept_encoding(value: &HeaderValue) -> bool {
    value.to_str().is_ok_and(|vary| {
        vary.split(',')
            .any(|v| v.trim().eq_ignore_ascii_case("accept-encoding"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header::{
        ACCEPT_ENCODING, ACCEPT_RANGES, CONTENT_ENCODING, CONTENT_LENGTH,
        CONTENT_RANGE, CONTENT_TYPE, VARY,
    };
    use http::Extensions;

    fn v(s: &'static str) -> HeaderValue {
        HeaderValue::from_static(s)
    }

    #[test]
    fn test_accepts_gzip_encoding_basic() {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, v("gzip"));
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_should_compress_response_rejects_content_range() {
        let request_method = http::Method::GET;
        let mut request_headers = HeaderMap::new();
        request_headers.insert(ACCEPT_ENCODING, v("gzip"));

        let response_status = http::StatusCode::OK;
        let mut response_headers = HeaderMap::new();
        response_headers.insert(CONTENT_TYPE, v("application/json"));
        response_headers.insert(CONTENT_RANGE, v("bytes 0-100/200"));

        let response_extensions = Extensions::new();

        assert!(!should_compress_response(
            &request_method,
            &request_headers,
            response_status,
            &response_headers,
            &response_extensions,
        ));
    }

    #[test]
    fn test_should_compress_response_respects_content_length_threshold() {
        let request_method = http::Method::GET;
        let mut request_headers = HeaderMap::new();
        request_headers.insert(ACCEPT_ENCODING, v("gzip"));

        let response_status = http::StatusCode::OK;
        let mut response_headers = HeaderMap::new();
        response_headers.insert(CONTENT_TYPE, v("application/json"));
        response_headers.insert(
            CONTENT_LENGTH,
            HeaderValue::from_str(&(MIN_COMPRESS_SIZE - 1).to_string())
                .unwrap(),
        );

        let response_extensions = Extensions::new();

        assert!(!should_compress_response(
            &request_method,
            &request_headers,
            response_status,
            &response_headers,
            &response_extensions,
        ));
    }

    #[test]
    fn test_apply_gzip_compression_removes_accept_ranges_and_sets_vary() {
        let body = "x".repeat((MIN_COMPRESS_SIZE + 10) as usize);
        let response = Response::builder()
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT_RANGES, "bytes")
            .body(Body::from(body))
            .unwrap();

        let compressed = apply_gzip_compression(response);
        let headers = compressed.headers();

        let gzip = v("gzip");
        assert_eq!(headers.get(CONTENT_ENCODING), Some(&gzip));
        assert!(!headers.contains_key(ACCEPT_RANGES));

        let vary_values: Vec<_> = headers
            .get_all(VARY)
            .iter()
            .map(|value| value.to_str().unwrap().to_string())
            .collect();
        assert!(vary_values
            .iter()
            .any(|value| value.eq_ignore_ascii_case("accept-encoding")));
    }

    #[test]
    fn test_apply_gzip_compression_avoids_duplicate_vary_entries() {
        let body = "x".repeat((MIN_COMPRESS_SIZE + 10) as usize);
        let response = Response::builder()
            .header(CONTENT_TYPE, "application/json")
            .header(VARY, "Accept-Encoding, Accept-Language")
            .body(Body::from(body))
            .unwrap();

        let compressed = apply_gzip_compression(response);
        let mut accept_encoding_count = 0;
        for value in compressed.headers().get_all(VARY).iter() {
            let text = value.to_str().unwrap();
            accept_encoding_count += text
                .split(',')
                .filter(|v| v.trim().eq_ignore_ascii_case("accept-encoding"))
                .count();
        }

        assert_eq!(accept_encoding_count, 1);
    }

    #[test]
    fn test_accepts_gzip_encoding_with_positive_quality() {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("gzip;q=0.8"));
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_rejects_zero_quality() {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("gzip;q=0"));
        assert!(!accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_wildcard() {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, v("*"));
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_wildcard_with_quality() {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, v("*;q=0.5"));
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_wildcard_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, v("*;q=0"));
        assert!(!accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_multiple_encodings() {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, v("deflate, gzip, br"));
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_gzip_takes_precedence_over_wildcard() {
        // Explicit gzip rejection should override wildcard acceptance
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, v("*;q=1.0, gzip;q=0"));
        assert!(!accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_gzip_acceptance_overrides_wildcard_rejection()
    {
        // Explicit gzip acceptance should work even if wildcard is rejected
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, v("*;q=0, gzip;q=1.0"));
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_prefers_highest_quality() {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, v("gzip;q=0, gzip;q=0.5"));
        assert!(accepts_gzip_encoding(&headers));

        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, v("gzip;q=0.8, gzip;q=0"));
        assert!(accepts_gzip_encoding(&headers));

        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, v("gzip;q=0, *;q=1"));
        assert!(!accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_case_insensitive() {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, v("GZIP"));
        assert!(accepts_gzip_encoding(&headers));

        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, v("GzIp"));
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
        headers.insert(ACCEPT_ENCODING, v("deflate , gzip ; q=0.8 , br"));
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_accepts_gzip_encoding_malformed_quality() {
        // If quality parsing fails, should default to 1.0
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, v("gzip;q=invalid"));
        assert!(accepts_gzip_encoding(&headers));
    }

    #[test]
    fn test_should_compress_response_basic() {
        let method = http::Method::GET;
        let mut request_headers = HeaderMap::new();
        request_headers.insert(ACCEPT_ENCODING, v("gzip"));
        let status = http::StatusCode::OK;
        let mut response_headers = HeaderMap::new();
        response_headers.insert(CONTENT_TYPE, v("application/json"));
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
        request_headers.insert(ACCEPT_ENCODING, v("gzip"));
        let status = http::StatusCode::OK;
        let mut response_headers = HeaderMap::new();
        response_headers.insert(CONTENT_TYPE, v("application/json"));
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
        request_headers.insert(ACCEPT_ENCODING, v("gzip"));
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
        request_headers.insert(ACCEPT_ENCODING, v("gzip"));
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
        request_headers.insert(ACCEPT_ENCODING, v("gzip"));
        let status = http::StatusCode::PARTIAL_CONTENT;
        let mut response_headers = HeaderMap::new();
        response_headers.insert(CONTENT_TYPE, v("application/json"));
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
        response_headers.insert(CONTENT_TYPE, v("application/json"));
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
        request_headers.insert(ACCEPT_ENCODING, v("gzip"));
        let status = http::StatusCode::OK;
        let mut response_headers = HeaderMap::new();
        response_headers.insert(CONTENT_TYPE, v("application/json"));
        response_headers.insert(CONTENT_ENCODING, v("br"));
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
        request_headers.insert(ACCEPT_ENCODING, v("gzip"));
        let status = http::StatusCode::OK;
        let mut response_headers = HeaderMap::new();
        response_headers.insert(CONTENT_TYPE, v("application/json"));
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
        request_headers.insert(ACCEPT_ENCODING, v("gzip"));
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
        request_headers.insert(ACCEPT_ENCODING, v("gzip"));
        let status = http::StatusCode::OK;
        let mut response_headers = HeaderMap::new();
        response_headers.insert(CONTENT_TYPE, v("TEXT/EVENT-STREAM"));
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
        request_headers.insert(ACCEPT_ENCODING, v("gzip"));
        let status = http::StatusCode::OK;
        let extensions = http::Extensions::new();

        // Test various compressible content types
        let compressible_types = vec![
            "application/json",
            "APPLICATION/JSON",
            "text/plain",
            "text/html",
            "text/css",
            "application/xml",
            "application/javascript",
            "application/x-javascript",
            "application/problem+json",
            "application/problem+JSON",
            "application/hal+json",
            "application/soap+xml",
            "application/SOAP+XML",
        ];

        for content_type in compressible_types {
            let mut response_headers = HeaderMap::new();
            response_headers.insert(
                CONTENT_TYPE,
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
        request_headers.insert(ACCEPT_ENCODING, v("gzip"));
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
                CONTENT_TYPE,
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
