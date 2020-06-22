// Copyright 2020 Oxide Computer Company
/*!
 * Generic server error handling facilities
 *
 * Error handling in an API
 * ------------------------
 *
 * Our approach for managing errors within the API server balances several
 * goals:
 *
 * * Every incoming HTTP request should conclude with a response, which is
 *   either successful (200-level or 300-level status code) or a failure
 *   (400-level for client errors, 500-level for server errors).
 * * There are several different sources of errors within an API server:
 *     * The HTTP layer of the server may generate an error.  In this case, it
 *       may be just as easy to generate the appropriate HTTP response (with a
 *       400-level or 500-level status code) as it would be to generate an Error
 *       object of some kind.
 *     * An HTTP-agnostic layer of the API server code base may generate an
 *       error.  It would be nice (but not essential) if these layers did not
 *       need to know about HTTP-specific things like status codes, particularly
 *       since they may not map straightforwardly.  For example, a NotFound
 *       error from the model may not result in a 404 out the API -- it might
 *       just mean that something in the model layer needs to create an object
 *       before using it.
 *     * A library that's not part of the API server code base may generate an
 *       error.  This would include standard library interfaces returning
 *       `std::io::Error` and Hyper returning `hyper::Error`, for examples.
 * * We'd like to take advantage of Rust's built-in error handling control flow
 *   tools, like Results and the '?' operator.
 *
 * Dropshot itself is concerned only with HTTP errors.  We define `HttpError`,
 * which provides a status code, error code (via an Enum), external message (for
 * sending in the response), optional metadata, and an internal message (for the
 * log file or other instrumentation).  The HTTP layers of the request-handling
 * stack may use this struct directly.  **The set of possible error codes here
 * is part of a service's OpenAPI contract, as is the schema for any metadata.**
 * By the time an error bubbles up to the top of the request handling stack, it
 * must be an HttpError.
 *
 * For the HTTP-agnostic layers of an API server (i.e., consumers of Dropshot),
 * we recommend a separate enum to represent their errors in an HTTP-agnostic
 * way.  Consumers can provide a `From` implementation that converts these
 * errors into HttpErrors.
 */

use hyper::error::Error as HyperError;
use serde::Deserialize;
use serde::Serialize;
use serde_json::error::Error as SerdeError;

/**
 * `HttpError` represents an error generated as part of handling an API
 * request.  When these bubble up to the top of the request handling stack
 * (which is most of the time that they're generated), these are turned into an
 * HTTP response, which includes:
 *
 *   * a status code, which is likely either 400-level (indicating a client
 *     error, like bad input) or 500-level (indicating a server error).
 *   * a structured (JSON) body, which includes:
 *       * a string error code, which identifies the underlying error condition
 *         so that clients can potentially make programmatic decisions based on
 *         the error type
 *       * a string error message, which is the human-readable summary of the
 *         issue, intended to make sense for API users (i.e., not API server
 *         developers)
 *       * optionally: additional metadata describing the issue.  For a
 *         validation error, this could include information about which
 *         parameter was invalid and why.  This should conform to a schema
 *         associated with the error code.
 *
 * It's easy to go overboard with the error codes and metadata.  Generally, we
 * should avoid creating specific codes and metadata unless there's a good
 * reason for a client to care.
 *
 * Besides that, `HttpError`s also have an internal error message, which may
 * differ from the error message that gets reported to users.  For example, if
 * the request fails because an internal database is unreachable, the client may
 * just see "internal error", while the server log would include more details
 * like "failed to acquire connection to database at 10.1.2.3".
 */
#[derive(Debug)]
pub struct HttpError {
    /*
     * TODO-coverage add coverage in the test suite for error_code
     * TODO-robustness should error_code just be required?  It'll be confusing
     * to clients if it's missing sometimes.  Should this class be parametrized
     * by some enum type?
     * TODO-polish add cause chain for a complete log message?
     */
    /** HTTP status code for this error */
    pub status_code: http::StatusCode,
    /**
     * Optional string error code for this error.  Callers are advised to
     * use an enum to populate this field.
     */
    pub error_code: Option<String>,
    /** Error message to be sent to API client for this error */
    pub external_message: String,
    /** Error message recorded in the log for this error */
    pub internal_message: String,
}

/**
 * Body of an HTTP response for an `HttpError`.  This type can be used to
 * deserialize an HTTP response corresponding to an error in order to access the
 * error code, message, etc.
 */
#[derive(Debug, Deserialize, Serialize)]
pub struct HttpErrorResponseBody {
    pub request_id: String,
    pub error_code: Option<String>,
    pub message: String,
}

impl From<SerdeError> for HttpError {
    fn from(error: SerdeError) -> Self {
        /*
         * TODO-polish it would really be much better to annotate this with
         * context about what we were parsing.
         */
        HttpError::for_bad_request(None, format!("invalid input: {}", error))
    }
}

impl From<HyperError> for HttpError {
    fn from(error: HyperError) -> Self {
        /*
         * TODO-correctness dig deeper into the various cases to make sure this
         * is a valid way to represent it.
         */
        HttpError::for_bad_request(
            None,
            format!("error processing request: {}", error),
        )
    }
}

impl From<http::Error> for HttpError {
    fn from(error: http::Error) -> Self {
        /*
         * TODO-correctness dig deeper into the various cases to make sure this
         * is a valid way to represent it.
         */
        HttpError::for_bad_request(
            None,
            format!("error processing request: {}", error),
        )
    }
}

impl HttpError {
    /**
     * Generates an `HttpError` for any 400-level client error with a custom
     * `message` used for both the internal and external message.  The
     * expectation here is that for most 400-level errors, there's no need for a
     * separate internal message.
     */
    pub fn for_client_error(
        error_code: Option<String>,
        status_code: http::StatusCode,
        message: String,
    ) -> Self {
        assert!(status_code.is_client_error());
        HttpError {
            status_code,
            error_code,
            internal_message: message.clone(),
            external_message: message,
        }
    }

    /**
     * Generates an `HttpError` for a 500 "Internal Server Error" error with the
     * given `internal_message` for the internal message.
     */
    pub fn for_internal_error(internal_message: String) -> Self {
        let status_code = http::StatusCode::INTERNAL_SERVER_ERROR;
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

    /**
     * Generates an `HttpError` for a 503 "Service Unavailable" error with the
     * given `internal_message` for the internal message.
     */
    pub fn for_unavail(
        error_code: Option<String>,
        internal_message: String,
    ) -> Self {
        let status_code = http::StatusCode::SERVICE_UNAVAILABLE;
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

    /**
     * Generates a 400 "Bad Request" error with the given `message` used for
     * both the internal and external message.  This is a convenience wrapper
     * around [`HttpError::for_client_error`].
     */
    pub fn for_bad_request(
        error_code: Option<String>,
        message: String,
    ) -> Self {
        HttpError::for_client_error(
            error_code,
            http::StatusCode::BAD_REQUEST,
            message,
        )
    }

    /**
     * Generates an `HttpError` for the given HTTP `status_code` where the
     * internal and external messages for the error come from the standard label
     * for this status code (e.g., the message for status code 404 is "Not
     * Found").
     */
    pub fn for_status(
        error_code: Option<String>,
        status_code: http::StatusCode,
    ) -> Self {
        /* TODO-polish This should probably be our own message. */
        let message = status_code.canonical_reason().unwrap().to_string();
        HttpError::for_client_error(error_code, status_code, message)
    }

    /**
     * Generates an `HttpError` for a 404 "Not Found" error with a custom
     * internal message `internal_message`.  The external message will be "Not
     * Found" (i.e., the standard label for status code 404).
     */
    pub fn for_not_found(
        error_code: Option<String>,
        internal_message: String,
    ) -> Self {
        let status_code = http::StatusCode::NOT_FOUND;
        let external_message =
            status_code.canonical_reason().unwrap().to_string();
        HttpError {
            status_code,
            error_code,
            internal_message,
            external_message,
        }
    }

    /**
     * Generates an HTTP response for the given `HttpError`, using `request_id`
     * for the response's request id.
     */
    pub fn into_response(
        self,
        request_id: &str,
    ) -> hyper::Response<hyper::Body> {
        /*
         * TODO-hardening: consider handling the operational errors that the
         * Serde serialization fails or the response construction fails.  In
         * those cases, we should probably try to report this as a serious
         * problem (e.g., to the log) and send back a 500-level response.  (Of
         * course, that could fail in the same way, but it's less likely because
         * there's only one possible set of input and we can test it.  We'll
         * probably have to use unwrap() there and make sure we've tested that
         * code at least once!)
         */
        hyper::Response::builder()
            .status(self.status_code)
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
