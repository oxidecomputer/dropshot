// Copyright 2023 Oxide Computer Company

//! Example use of Dropshot supporting requests with an HTTP range headers
//!
//! This example is based on the "basic.rs" one.  See that one for more detailed
//! comments on the common code.

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::HttpServerStarter;
use dropshot::RequestContext;
use http::header;
use http::status::StatusCode;
use hyper::Body;
use hyper::Response;
use std::ops::RangeInclusive;

#[allow(clippy::let_unit_value)] // suppress warnings on our empty api_context
#[tokio::main]
async fn main() -> Result<(), String> {
    let config_dropshot: ConfigDropshot = Default::default();
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;
    let mut api = ApiDescription::new();
    api.register(example_get_with_range_support).unwrap();
    api.register(example_head_with_range_support).unwrap();

    let api_context = ();
    let server =
        HttpServerStarter::new(&config_dropshot, api, api_context, &log)
            .map_err(|error| format!("failed to create server: {}", error))?
            .start();
    server.await
}

/// Return "lorem ipsum" text, optionally with a range provided by the client.
///
/// Does not support multiple range specifications (and will therefore never
/// send `content-type: multipart/byteranges` responses).
#[endpoint {
    method = GET,
    path = "/lorem-ipsum",
}]
async fn example_get_with_range_support(
    rqctx: RequestContext<()>,
) -> Result<Response<Body>, HttpError> {
    get_or_head_with_range_support(
        rqctx,
        LOREM_IPSUM.len() as u64,
        &|maybe_range| {
            match maybe_range {
                None => Body::from(LOREM_IPSUM),
                Some(range) => {
                    // We should only be called with ranges that fit into
                    // `0..LOREM_IPSUM.len()` (the data length we pass above),
                    // so we'll panic here if we get an out of bounds range.
                    let data = LOREM_IPSUM
                        .get(*range.start() as usize..=*range.end() as usize)
                        .expect("invalid range returned by validate");
                    Body::from(data)
                }
            }
        },
    )
    .await
}

/// Implement `HEAD` for our download endpoint above.
#[endpoint {
    method = HEAD,
    path = "/lorem-ipsum",
}]
async fn example_head_with_range_support(
    rqctx: RequestContext<()>,
) -> Result<Response<Body>, HttpError> {
    get_or_head_with_range_support(rqctx, LOREM_IPSUM.len() as u64, &|_| {
        Body::empty()
    })
    .await
}

async fn get_or_head_with_range_support(
    rqctx: RequestContext<()>,
    data_len: u64,
    make_body: &(dyn Fn(Option<RangeInclusive<u64>>) -> Body + Send + Sync),
) -> Result<Response<Body>, HttpError> {
    let headers = rqctx.request.headers();

    let Some(range) = headers.get(header::RANGE) else {
        // No range specification; return the full data, but set the
        // `accept-ranges` header to indicate the client _could_ have asked for
        // a subset.
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CONTENT_TYPE, "text/plain")
            .header(header::CONTENT_LENGTH, data_len.to_string())
            .body(make_body(None))?);
    };

    // Parse the range header value.
    let range = match range
        .to_str()
        .map_err(|err| format!("invalid range header: {err}"))
        .and_then(|range| {
            http_range_header::parse_range_header(range)
                .map_err(|err| format!("invalid range header: {err}"))
        }) {
        Ok(range) => range,
        Err(err) => return range_not_satisfiable(data_len, err),
    };

    // Ensure the requested ranges are valid for our data.
    let mut ranges = match range.validate(data_len) {
        Ok(ranges) => ranges,
        Err(err) => {
            return range_not_satisfiable(data_len, err.to_string());
        }
    };

    // If the client requested multiple ranges, we ought to send back a
    // `multipart/byteranges` payload with delimiters and separate content-type
    // / content-range headers on each part (see RFC 7233 § 4.1). We do not
    // support that in this example, so we'll send back a RANGE_NOT_SATISFIABLE
    // if more than one range was requested. This seems to be allowed by a
    // loose reading of RFC 7233:
    //
    // > The 416 (Range Not Satisfiable) status code indicates that none of
    // > the ranges in the request's Range header field (Section 3.1) overlap
    // > the current extent of the selected resource or that the set of ranges
    // > requested has been rejected due to invalid ranges or an excessive
    // > request of small or overlapping ranges.
    //
    // if we consider two or more ranges of any size to be "an excessive request
    // of small or overlapping ranges".
    if ranges.len() > 1 {
        return range_not_satisfiable(
            data_len,
            "server only supports a single range".to_string(),
        );
    }
    let range = ranges.remove(0);

    // Call `make_body` with the requested range, and trust that it returns a
    // body of exactly the requested length.
    let content_length = range.end() - range.start() + 1;
    let content_range =
        format!("bytes {}-{}/{data_len}", range.start(), range.end());
    let body = make_body(Some(range));

    // Send back an HTTP 206 (Partial Content) with the required headers
    // (content-type, content-range, content-length). Also set the accept-ranges
    // header; it's unclear whether this is expected in a 206 response.
    Ok(Response::builder()
        .status(StatusCode::PARTIAL_CONTENT)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::CONTENT_LENGTH, content_length.to_string())
        .header(header::CONTENT_RANGE, content_range)
        .body(body)?)
}

fn range_not_satisfiable(
    data_len: u64,
    err: String,
) -> Result<Response<Body>, HttpError> {
    // TODO: It's weird that we're returning `Ok(_)` with a status code that
    // indicates an error (RANGE_NOT_SATISFIABLE is HTTP 416), but we need to
    // set the above headers on the response, which HttpError currently doesn't
    // support. We build a custom Response instead.
    //
    // Replace this with a header-bearing HttpError once we can:
    // https://github.com/oxidecomputer/dropshot/issues/644
    Ok(Response::builder()
        .status(StatusCode::RANGE_NOT_SATISFIABLE)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_RANGE, format!("bytes */{}", data_len))
        .body(err.into())?)
}

const LOREM_IPSUM: &[u8] = b"\
Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod \
tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, \
quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo \
consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse \
cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat \
non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.\
";
