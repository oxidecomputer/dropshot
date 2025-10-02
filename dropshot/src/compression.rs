// Copyright 2025 Oxide Computer Company

//! Response compression support for Dropshot.

use crate::body::Body;
use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Response};
use http_body_util::BodyExt;
use hyper::body::Body as HttpBodyTrait;
use std::io::Write;

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
    request_headers: &HeaderMap<HeaderValue>,
    response_headers: &HeaderMap<HeaderValue>,
) -> bool {
    // Don't compress if client doesn't accept gzip
    if !accepts_gzip_encoding(request_headers) {
        return false;
    }

    // Don't compress if already encoded
    if response_headers.contains_key(http::header::CONTENT_ENCODING) {
        return false;
    }

    // Don't compress if explicitly disabled
    if response_headers.contains_key("x-dropshot-disable-compression") {
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

    let is_compressible = ct_str.starts_with("application/json")
        || ct_str.starts_with("text/")
        || ct_str.starts_with("application/xml")
        || ct_str.starts_with("application/javascript")
        || ct_str.starts_with("application/x-javascript");

    is_compressible
}

/// Minimum size in bytes for a response to be compressed.
/// Responses smaller than this won't benefit from compression and may actually get larger.
const MIN_COMPRESS_SIZE: usize = 1024; // 1KB

/// Applies gzip compression to a response.
/// This is an async function that consumes the entire body, compresses it, and returns a new response.
/// If the body is smaller than MIN_COMPRESS_SIZE, compression is skipped.
/// Streaming responses (those without a known size) are not compressed.
pub async fn apply_gzip_compression(
    response: Response<Body>,
) -> Result<Response<Body>, Box<dyn std::error::Error + Send + Sync>> {
    let (mut parts, body) = response.into_parts();

    // Check if this is a streaming response (no exact size known)
    // If so, don't compress it as buffering would defeat the purpose of streaming
    let size_hint = body.size_hint();
    if size_hint.exact().is_none() {
        return Ok(Response::from_parts(parts, body));
    }

    // Collect the entire body into bytes
    let body_bytes = body.collect().await?.to_bytes();

    // Don't compress if the body is too small
    if body_bytes.len() < MIN_COMPRESS_SIZE {
        // Return the original response unchanged
        return Ok(Response::from_parts(parts, Body::from(body_bytes)));
    }

    // Compress the body using gzip
    let mut encoder = flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    );
    encoder.write_all(&body_bytes)?;
    let compressed_bytes = encoder.finish()?;

    // Only use compression if it actually makes the response smaller
    if compressed_bytes.len() < body_bytes.len() {
        // Add gzip content-encoding header
        parts.headers.insert(
            http::header::CONTENT_ENCODING,
            HeaderValue::from_static("gzip"),
        );

        // Set content-length to the compressed size
        parts.headers.insert(
            http::header::CONTENT_LENGTH,
            HeaderValue::from(compressed_bytes.len()),
        );

        // Create a new body with compressed content
        let compressed_body = Body::from(Bytes::from(compressed_bytes));

        Ok(Response::from_parts(parts, compressed_body))
    } else {
        // Compression didn't help, return uncompressed
        Ok(Response::from_parts(parts, Body::from(body_bytes)))
    }
}
