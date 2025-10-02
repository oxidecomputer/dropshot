// Copyright 2025 Oxide Computer Company

//! Response compression support for Dropshot.

use crate::body::Body;
use async_compression::tokio::bufread::GzipEncoder;
use futures::{StreamExt, TryStreamExt};
use http::{HeaderMap, HeaderValue, Response};
use hyper::body::{Body as HttpBodyTrait, Frame};
use tokio_util::io::{ReaderStream, StreamReader};

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
const MIN_COMPRESS_SIZE: u64 = 32; // 32 bytes

/// Applies gzip compression to a response using streaming compression.
/// This function wraps the response body in a gzip encoder that compresses data
/// as it's being sent, avoiding the need to buffer the entire response in memory.
/// If the body has a known exact size smaller than MIN_COMPRESS_SIZE, compression is skipped.
pub fn apply_gzip_compression(
    response: Response<Body>,
) -> Result<Response<Body>, Box<dyn std::error::Error + Send + Sync>> {
    let (mut parts, body) = response.into_parts();

    // Check the size hint to see if we should skip compression
    let size_hint = body.size_hint();
    if let Some(exact_size) = size_hint.exact() {
        if exact_size < MIN_COMPRESS_SIZE {
            // Body is too small, don't compress
            return Ok(Response::from_parts(parts, body));
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

    // Remove content-length since we don't know the compressed size
    parts.headers.remove(http::header::CONTENT_LENGTH);

    Ok(Response::from_parts(parts, compressed_body))
}
