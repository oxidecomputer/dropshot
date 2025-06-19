// Copyright 2025 Oxide Computer Company

//! Response compression support for Dropshot.

use crate::body::Body;
use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Response};
use http_body_util::BodyExt;
use std::io::Write;

/// Checks if the request accepts gzip encoding based on the Accept-Encoding header.
pub fn accepts_gzip_encoding(headers: &HeaderMap<HeaderValue>) -> bool {
    if let Some(accept_encoding) = headers.get(http::header::ACCEPT_ENCODING) {
        if let Ok(encoding_str) = accept_encoding.to_str() {
            // Simple check for gzip in the Accept-Encoding header
            // This handles cases like "gzip", "gzip, deflate", "deflate, gzip, br", etc.
            return encoding_str
                .split(',')
                .any(|encoding| encoding.trim().eq_ignore_ascii_case("gzip"));
        }
    }
    false
}

/// Applies gzip compression to a response.
/// This is an async function that consumes the entire body, compresses it, and returns a new response.
pub async fn apply_gzip_compression(
    response: Response<Body>,
) -> Result<Response<Body>, Box<dyn std::error::Error + Send + Sync>> {
    let (mut parts, body) = response.into_parts();

    // Collect the entire body into bytes
    let body_bytes = body.collect().await?.to_bytes();

    // Compress the body using gzip
    let mut encoder = flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    );
    encoder.write_all(&body_bytes)?;
    let compressed_bytes = encoder.finish()?;

    // Add gzip content-encoding header
    parts.headers.insert(
        http::header::CONTENT_ENCODING,
        HeaderValue::from_static("gzip"),
    );

    // Remove content-length since it will be different after compression
    parts.headers.remove(http::header::CONTENT_LENGTH);

    // Create a new body with compressed content
    let compressed_body = Body::from(Bytes::from(compressed_bytes));

    Ok(Response::from_parts(parts, compressed_body))
}
