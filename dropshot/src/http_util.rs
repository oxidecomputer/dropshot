// Copyright 2020 Oxide Computer Company
//! General-purpose HTTP-related facilities

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Body as HttpBody;
use serde::de::DeserializeOwned;

use super::error::HttpError;
use crate::from_map::from_map;
use crate::router::VariableSet;

/// header name for conveying request ids ("x-request-id")
pub const HEADER_REQUEST_ID: &str = "x-request-id";
/// MIME type for raw bytes
pub const CONTENT_TYPE_OCTET_STREAM: &str = "application/octet-stream";
/// MIME type for plain JSON data
pub const CONTENT_TYPE_JSON: &str = "application/json";
/// MIME type for newline-delimited JSON data
pub const CONTENT_TYPE_NDJSON: &str = "application/x-ndjson";
/// MIME type for form/urlencoded data
pub const CONTENT_TYPE_URL_ENCODED: &str = "application/x-www-form-urlencoded";
/// MIME type for multipart/form-data
pub const CONTENT_TYPE_MULTIPART_FORM_DATA: &str = "multipart/form-data";

/// Reads the rest of the body from the request, dropping all the bytes.  This is
/// useful after encountering error conditions.
pub async fn http_dump_body<T>(body: &mut T) -> Result<usize, T::Error>
where
    T: HttpBody<Data = Bytes> + std::marker::Unpin,
{
    // TODO should this use some Stream interface instead?
    // TODO-hardening: does this actually cap the amount of data that will be
    // read?  What if the underlying implementation chooses to wait for a much
    // larger number of bytes?
    // TODO better understand pin_mut!()
    // TODO do we need to use saturating_add() here?
    let mut nbytesread: usize = 0;
    while let Some(maybefr) = body.frame().await {
        let fr = maybefr?;
        if let Ok(buf) = fr.into_data() {
            nbytesread += buf.len();
        }
    }

    // TODO-correctness why does the is_end_stream() assertion fail?
    // assert!(body.is_end_stream());
    Ok(nbytesread)
}

/// Errors returned by [`http_extract_path_params`].
#[derive(Debug, thiserror::Error)]
#[error("bad parameter in URL path: {0}")]
pub struct PathError(String);

impl From<PathError> for HttpError {
    fn from(error: PathError) -> Self {
        HttpError::for_bad_request(None, error.to_string())
    }
}

/// Given a set of variables (most immediately from a RequestContext, likely
/// generated by the HttpRouter when routing an incoming request), extract them
/// into an instance of type T.  This is a convenience function that reports an
/// appropriate error when the extraction fails.
///
/// Note that if this function fails, either there was a type error (e.g., a path
/// parameter was supposed to be a UUID, but wasn't), in which case we should
/// report a 400-level error; or the caller attempted to extract a parameter
/// (using a field in T) that wasn't populated in `path_params`.  This latter
/// case is a programmer error -- this invocation can never work with this type
/// for this HTTP handler.  Ideally, we'd catch this at build time, but we don't
/// currently do that.  However, we _do_ currently catch this at server startup
/// time, so this case should be impossible.
///
/// TODO-cleanup: It would be better to fail to build when the struct's
/// parameters don't match up precisely with the path parameters
/// TODO-cleanup It would also be nice to know if the struct could not possibly
/// be correctly constructed from path parameters because the struct contains
/// values that could not be represented in path parameters (e.g., nested
/// structs).  One approach to doing this would be to skip serde altogether here
/// for `T` and instead define our own trait.  We could define a "derive" macro
/// that would do something similar to serde, but only allows field values that
/// implement FromStr.  Then we'd at least know at build time that the consumer
/// gave us a type that could conceivably be represented by the path parameters.
/// TODO-testing: Add automated tests.
pub fn http_extract_path_params<T: DeserializeOwned>(
    path_params: &VariableSet,
) -> Result<T, PathError> {
    from_map(path_params).map_err(|message| {
        // TODO-correctness We'd like to assert that the error here is a bad
        // type, not a missing field.  If it's a missing field, then we somehow
        // allowed somebody to register a handler function for a path where the
        // handler function's path parameters are inconsistent with the actual
        // path registered.  Unfortunately, we don't have a way to
        // programmatically distinguish these values at this point.  In fact,
        // even with our own deserializer, we'd also have to build our
        // own serde::de::Error impl in order to distinguish this particular
        // case.  For now, we resort to parsing the error message.
        // TODO-correctness The error message produced in the type-error case
        // (that end users will see) does not indicate which path parameter was
        // invalid.  That's pretty bad for end users.
        assert!(!message.starts_with("missing field: "));
        PathError(message)
    })
}
