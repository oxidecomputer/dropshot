// Copyright 2024 Oxide Computer Company

//! Generic server error handling facilities
//!
//! Error handling in an API
//! ------------------------
//!
//! Our approach for managing errors within the API server balances several
//! goals:
//!
//! * Every incoming HTTP request should conclude with a response, which is
//!   either successful (200-level or 300-level status code) or a failure
//!   (400-level for client errors, 500-level for server errors).
//! * There are several different sources of errors within an API server:
//!     * The HTTP layer of the server may generate an error.  In this case, it
//!       may be just as easy to generate the appropriate HTTP response (with a
//!       400-level or 500-level status code) as it would be to generate an Error
//!       object of some kind.
//!     * An HTTP-agnostic layer of the API server code base may generate an
//!       error.  It would be nice (but not essential) if these layers did not
//!       need to know about HTTP-specific things like status codes, particularly
//!       since they may not map straightforwardly.  For example, a NotFound
//!       error from the model may not result in a 404 out the API -- it might
//!       just mean that something in the model layer needs to create an object
//!       before using it.
//!     * A library that's not part of the API server code base may generate an
//!       error.  This would include standard library interfaces returning
//!       `std::io::Error` and Hyper returning `hyper::Error`, for examples.
//! * We'd like to take advantage of Rust's built-in error handling control flow
//!   tools, like Results and the '?' operator.
//!
//! Dropshot itself is concerned only with HTTP errors.  We define an
//! [`HttpResponseError`] trait (in `handler.rs`), that provides an interface
//! for error types to indicate how they may be converted into an HTTP response.
//! In particular, such types must be capable of providing a 4xx or 5xx status
//! code for the response; an implementation of the `HttpResponseContent` trait,
//! for producing the response body; response metadata for the OpenAPI document;
//! and an implementation of `std::fmt::Display`, so that dropshot can log the
//! error.  **The set of possible error codes here is part of a service's
//! OpenAPI contract, as is the schema for any metadata.**  In addition, we
//! define an `HttpError` struct in this module, which provides a status code,
//! error code, external message (for sending in the response), optional
//! metadata, and an internal message (for the log file or other
//! instrumentation).  This type is used for errors produced within Dropshot,
//! such as by extractors, and implements `HttpResponseError`, so that the HTTP
//! layers of the request-handling stack may use this struct directly when
//! specific error presentation is not needed.  By the time an error bubbles up
//! to the top of the request handling stack, it must be a type that implements
//! `HttpResponseError`, either via an implementation for a user-defined type,
//! or by converting it into a Dropshot `HttpError`.
//!
//! We require that status codes provided by the [`HttpResponseError`] trait are
//! either client errors (4xx) or server errors (5xx), so that error responses
//! can be differentiated from successful responses in the generated OpenAPI
//! document.  As the `http` crate's `StatusCode` type can represent any status
//! code, the `error_status_code` module defines `ErrorStatusCode` and
//! `ClientErrorStatusCode` newtypes around [`http::StatusCode`] that are
//! validated upon construction to contain only errors.  An `ErrorStatusCode`
//! may be constructed from any 4xx or 5xx status code, while
//! `ClientErrorStatusCode` may only be constructed from a 4xx.  In addition to
//! fallible conversions from any `StatusCode`, associated constants are
//! provided for well-known error status codes, so user code may reference them
//! by name without requiring fallible runtime valdiation.
//!
//! For the HTTP-agnostic layers of an API server (i.e., consumers of Dropshot),
//! we recommend a separate enum to represent their errors in an HTTP-agnostic
//! way.  Consumers can provide a `From` implementation that converts these
//! errors into `HttpError`s, or implement the [`HttpResponseError`] trait to
//! provide their own mechanism.

use crate::ClientErrorStatusCode;
use crate::ErrorStatusCode;
use hyper::Error as HyperError;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::error::Error;
use std::fmt;

/// `HttpError` represents an error generated as part of handling an API
/// request.  When these bubble up to the top of the request handling stack
/// (which is most of the time that they're generated), these are turned into an
/// HTTP response, which includes:
///
///   * a status code, which is likely either 400-level (indicating a client
///     error, like bad input) or 500-level (indicating a server error).
///   * a structured (JSON) body, which includes:
///       * a string error code, which identifies the underlying error condition
///         so that clients can potentially make programmatic decisions based on
///         the error type
///       * a string error message, which is the human-readable summary of the
///         issue, intended to make sense for API users (i.e., not API server
///         developers)
///       * optionally: additional metadata describing the issue.  For a
///         validation error, this could include information about which
///         parameter was invalid and why.  This should conform to a schema
///         associated with the error code.
///
/// It's easy to go overboard with the error codes and metadata.  Generally, we
/// should avoid creating specific codes and metadata unless there's a good
/// reason for a client to care.
///
/// Besides that, `HttpError`s also have an internal error message, which may
/// differ from the error message that gets reported to users.  For example, if
/// the request fails because an internal database is unreachable, the client may
/// just see "internal error", while the server log would include more details
/// like "failed to acquire connection to database at 10.1.2.3".
#[derive(Debug)]
pub struct HttpError {
    // TODO-coverage add coverage in the test suite for error_code
    // TODO-polish add cause chain for a complete log message?
    /// HTTP status code for this error
    pub status_code: ErrorStatusCode,
    /// Optional string error code for this error.  Callers are advised to
    /// use an enum to populate this field.
    pub error_code: Option<String>,
    /// Error message to be sent to API client for this error
    pub external_message: String,
    /// Error message recorded in the log for this error
    pub internal_message: String,
}

/// Body of an HTTP response for an `HttpError`.  This type can be used to
/// deserialize an HTTP response corresponding to an error in order to access the
/// error code, message, etc.
#[derive(Debug, Deserialize, Serialize)]
pub struct HttpErrorResponseBody {
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub message: String,
}

// We hand-roll our JSON schema to avoid `error_code` being "nullable".
impl JsonSchema for HttpErrorResponseBody {
    fn schema_name() -> String {
        "Error".to_string()
    }

    fn json_schema(
        gen: &mut schemars::gen::SchemaGenerator,
    ) -> schemars::schema::Schema {
        let str_schema = String::json_schema(gen);

        schemars::schema::SchemaObject {
            metadata: Some(
                schemars::schema::Metadata {
                    description: Some(
                        "Error information from a response.".into(),
                    ),
                    ..Default::default()
                }
                .into(),
            ),
            instance_type: Some(schemars::schema::InstanceType::Object.into()),
            object: Some(
                schemars::schema::ObjectValidation {
                    required: ["message".into(), "request_id".into()]
                        .into_iter()
                        .collect(),
                    properties: [
                        ("error_code".into(), str_schema.clone()),
                        ("message".into(), str_schema.clone()),
                        ("request_id".into(), str_schema.clone()),
                    ]
                    .into_iter()
                    .collect(),
                    ..Default::default()
                }
                .into(),
            ),
            ..Default::default()
        }
        .into()
    }
}

impl From<HyperError> for HttpError {
    fn from(error: HyperError) -> Self {
        // TODO-correctness dig deeper into the various cases to make sure this
        // is a valid way to represent it.
        HttpError::for_bad_request(
            None,
            format!("error processing request: {}", error),
        )
    }
}

impl From<http::Error> for HttpError {
    fn from(error: http::Error) -> Self {
        // TODO-correctness dig deeper into the various cases to make sure this
        // is a valid way to represent it.
        HttpError::for_bad_request(
            None,
            format!("error processing request: {}", error),
        )
    }
}

impl HttpError {
    /// Generates an `HttpError` for any 400-level client error with a custom
    /// `message` used for both the internal and external message.  The
    /// expectation here is that for most 400-level errors, there's no need for a
    /// separate internal message.
    pub fn for_client_error(
        error_code: Option<String>,
        status_code: ClientErrorStatusCode,
        message: String,
    ) -> Self {
        HttpError {
            status_code: status_code.into(),
            error_code,
            internal_message: message.clone(),
            external_message: message,
        }
    }

    /// Generates an `HttpError` for a 500 "Internal Server Error" error with the
    /// given `internal_message` for the internal message.
    pub fn for_internal_error(internal_message: String) -> Self {
        let status_code = ErrorStatusCode::INTERNAL_SERVER_ERROR;
        HttpError {
            status_code,
            error_code: Some(String::from("Internal")),
            external_message: status_code
                .canonical_reason()
                .unwrap()
                .to_string(),
            internal_message,
        }
    }

    /// Generates an `HttpError` for a 503 "Service Unavailable" error with the
    /// given `internal_message` for the internal message.
    pub fn for_unavail(
        error_code: Option<String>,
        internal_message: String,
    ) -> Self {
        let status_code = ErrorStatusCode::SERVICE_UNAVAILABLE;
        HttpError {
            status_code,
            error_code,
            external_message: status_code
                .canonical_reason()
                .unwrap()
                .to_string(),
            internal_message,
        }
    }

    /// Generates a 400 "Bad Request" error with the given `message` used for
    /// both the internal and external message.  This is a convenience wrapper
    /// around [`HttpError::for_client_error`].
    pub fn for_bad_request(
        error_code: Option<String>,
        message: String,
    ) -> Self {
        HttpError::for_client_error(
            error_code,
            ClientErrorStatusCode::BAD_REQUEST,
            message,
        )
    }

    /// Generates an `HttpError` for the given HTTP `status_code` where the
    /// internal and external messages for the error come from the standard label
    /// for this status code (e.g., the message for status code 404 is "Not
    /// Found").
    pub fn for_client_error_with_status(
        error_code: Option<String>,
        status_code: ClientErrorStatusCode,
    ) -> Self {
        // TODO-polish This should probably be our own message.
        let message = status_code.canonical_reason().unwrap().to_string();
        HttpError::for_client_error(error_code, status_code, message)
    }

    /// Generates an `HttpError` for a 404 "Not Found" error with a custom
    /// internal message `internal_message`.  The external message will be "Not
    /// Found" (i.e., the standard label for status code 404).
    pub fn for_not_found(
        error_code: Option<String>,
        internal_message: String,
    ) -> Self {
        let status_code = ErrorStatusCode::NOT_FOUND;
        let external_message =
            status_code.canonical_reason().unwrap().to_string();
        HttpError {
            status_code,
            error_code,
            internal_message,
            external_message,
        }
    }

    /// Generates an HTTP response for the given `HttpError`, using `request_id`
    /// for the response's request id.
    pub fn into_response(
        self,
        request_id: &str,
    ) -> hyper::Response<crate::Body> {
        // TODO-hardening: consider handling the operational errors that the
        // Serde serialization fails or the response construction fails.  In
        // those cases, we should probably try to report this as a serious
        // problem (e.g., to the log) and send back a 500-level response.  (Of
        // course, that could fail in the same way, but it's less likely because
        // there's only one possible set of input and we can test it.  We'll
        // probably have to use unwrap() there and make sure we've tested that
        // code at least once!)
        hyper::Response::builder()
            .status(self.status_code.as_status())
            .header(
                http::header::CONTENT_TYPE,
                super::http_util::CONTENT_TYPE_JSON,
            )
            .header(super::http_util::HEADER_REQUEST_ID, request_id)
            .body(
                serde_json::to_string_pretty(&HttpErrorResponseBody {
                    request_id: request_id.to_string(),
                    message: self.external_message,
                    error_code: self.error_code,
                })
                .unwrap()
                .into(),
            )
            .unwrap()
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HttpError({}): {}", self.status_code, self.external_message)
    }
}

impl Error for HttpError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

#[cfg(test)]
mod test {
    use crate::HttpErrorResponseBody;

    #[test]
    fn test_serialize_error_response_body() {
        let err = HttpErrorResponseBody {
            request_id: "123".to_string(),
            error_code: None,
            message: "oy!".to_string(),
        };
        let out = serde_json::to_string(&err).unwrap();
        assert_eq!(out, r#"{"request_id":"123","message":"oy!"}"#);

        let err = HttpErrorResponseBody {
            request_id: "123".to_string(),
            error_code: Some("err".to_string()),
            message: "oy!".to_string(),
        };
        let out = serde_json::to_string(&err).unwrap();
        assert_eq!(
            out,
            r#"{"request_id":"123","error_code":"err","message":"oy!"}"#
        );
    }
}
