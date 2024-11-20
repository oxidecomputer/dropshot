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
//! * API handlers should be able to return user-defined error types, provided
//!   that those types can be serialized as response bodies, and user-defined
//!   error types should be part of a generated OpenAPI document when they have
//!   a known `JsonSchema`.
//!
//! Dropshot itself is concerned only with HTTP errors.  We define `HttpError`,
//! which provides a status code, error code (via an Enum), external message (for
//! sending in the response), optional metadata, and an internal message (for the
//! log file or other instrumentation).  The HTTP layers of the request-handling
//! stack may use this struct directly.  Furthermore, we also define the
//! `HttpResponseError` trait (in `handler.rs`).  This trait provides an
//! interface for error types to indicate how they may be converted into an HTTP
//! response body.  In particular, such types must be capable of providing a
//! status code for the response, and that status code must be a 4xx or 5xx
//! status. **The set of possible error codes here is part of a service's
//! OpenAPI contract, as is the schema for any metadata.**
//! By the time an error bubbles up to the top of the request handling stack, it
//! must be a type which implements `HttpResponseError`, either via an
//! implementation for a user-defined type, or by converting it into a Dropshot
//! `HttpError`.
//!
//! Because we generate separate error responses in the OpenAPI document for an
//! endpoint function's error types, the code that produces HTTP responses for
//! those errors may only produce a response with a 4xx or 5xx status code.
//! Therefore, we define the `ErrorStatusCode` and `ClientErrorStatusCode` types
//! in this module, which represent refinements of the `http::StatusCode` type.
//! While `http::StatusCode` can represent any status code, `ErrorStatusCode`
//! is validated upon construction to represent only 4xx and 5xx status codes,
//! and `ClientErrorStatusCode` is validated to only be a 4xx.  These types may
//! be constructed from any status code at runtime, failing if the status code
//! is not in the respective error status code ranges; alternatively, constants
//! are provided for well-known error status codes, so that users may reference
//! them by name without requiring fallible runtime valdiation.
//!
//! For the HTTP-agnostic layers of an API server (i.e., consumers of Dropshot),
//! we recommend a separate enum to represent their errors in an HTTP-agnostic
//! way.  Consumers can provide a `From` implementation that converts these
//! errors into `HttpError`s, or implement the [`HttpResponseError`] trait to
//! provide their own mechanism

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
    // TODO-robustness should error_code just be required?  It'll be confusing
    // to clients if it's missing sometimes.  Should this class be parametrized
    // by some enum type?
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
    ///
    /// The status code is provided as a [`ClientErrorStatusCode`], which may
    /// only be constructed for 4xx client errors.
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
    pub fn for_status(
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

/// An HTTP 4xx (client error) or 5xx (server error) status code.
///
/// This is a refinement of the [`http::StatusCode`] type that is limited to the
/// error status code ranges. It may be constructed from any
/// [`http::StatusCode`] using the `TryFrom` implementation, which fails if the
/// status is not a 4xx or 5xx status code.
///
/// Alternatively, constants are provided for known error status codes,
/// such as [`ErrorStatusCode::BAD_REQUEST`], [`ErrorStatusCode::NOT_FOUND`],
/// [`ErrorStatusCode::INTERNAL_SERVER_ERROR`], and so on, including those in
/// the IANA HTTP Status Code Registry](
/// https://www.iana.org/assignments/http-status-codes/http-status-codes.xhtml).
/// Using these constants avoids the fallible conversion from an [`http::StatusCode`].
///
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ErrorStatusCode(http::StatusCode);

macro_rules! error_status_code_constants {
    ( $($(#[$docs:meta])* $name:ident;)+ ) => {
        $(
            $(#[$docs])*
            pub const $name: Self = Self(http::StatusCode::$name);
        )+
    }
}

macro_rules! impl_status_code_wrapper {
    (impl StatusCode for $T:ident {
        type NotAnError = $NotAnError:ident;
        type Invalid = $Invalid:ident;
    }) => {
        impl $T {
            /// Converts a `&[u8]` to an error status code
            pub fn from_bytes(src: &[u8]) -> Result<Self, $Invalid> {
                let status = http::StatusCode::from_bytes(src)?;
                Self::from_status(status).map_err(Into::into)
            }

            /// Returns the [`http::StatusCode`] corresponding to this error status code.
            ///
            /// # Note
            ///
            /// This is the same as the `Into<http::StatusCode>` implementation, but
            /// included as an inherent method because that implementation doesn't
            /// appear in rustdocs, as well as a way to force the type instead of
            /// relying on inference.
            #[inline]
            pub fn as_status(&self) -> http::StatusCode {
                self.0
            }

            /// Returns the `u16` corresponding to this `error status code.
            ///
            /// # Note
            ///
            /// This is the same as the `Into<u16>` implementation, but
            /// included as an inherent method because that implementation doesn't
            /// appear in rustdocs, as well as a way to force the type instead of
            /// relying on inference.
            ///
            /// This method wraps the [`http::StatusCode::as_u16`] method.
            #[inline]
            pub fn as_u16(&self) -> u16 {
                self.0.as_u16()
            }

            /// Returns a `&str` representation of the `StatusCode`
            ///
            /// The return value only includes a numerical representation of the
            /// status code. The canonical reason is not included.
            ///
            /// This method wraps the [`http::StatusCode::as_str`] method.
            #[inline]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }

            /// Get the standardised `reason-phrase` for this status code.
            ///
            /// This is mostly here for servers writing responses, but could potentially have application
            /// at other times.
            ///
            /// The reason phrase is defined as being exclusively for human readers. You should avoid
            /// deriving any meaning from it at all costs.
            ///
            /// Bear in mind also that in HTTP/2.0 and HTTP/3.0 the reason phrase is abolished from
            /// transmission, and so this canonical reason phrase really is the only reason phrase you’ll
            /// find.
            ///
            /// This method wraps the [`http::StatusCode::canonical_reason`] method.
            #[inline]
            pub fn canonical_reason(&self) -> Option<&'static str> {
                self.0.canonical_reason()
            }
        }

        impl fmt::Debug for $T {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Debug::fmt(&self.0, f)
            }
        }

        /// Formats the status code, *including* the canonical reason.
        impl fmt::Display for $T {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl PartialEq<u16> for $T {
            #[inline]
            fn eq(&self, other: &u16) -> bool {
                self.as_u16() == *other
            }
        }

        impl PartialEq<$T> for u16 {
            #[inline]
            fn eq(&self, other: &$T) -> bool {
                *self == other.as_u16()
            }
        }

        impl PartialEq<http::StatusCode> for $T {
            #[inline]
            fn eq(&self, other: &http::StatusCode) -> bool {
                self.0 == *other
            }
        }

        impl PartialEq<$T> for http::StatusCode {
            #[inline]
            fn eq(&self, other: &$T) -> bool {
                *self == other.0
            }
        }

        impl From<$T> for u16 {
            #[inline]
            fn from(status: $T) -> u16 {
                status.as_u16()
            }
        }

        impl std::str::FromStr for $T {
            type Err = $Invalid;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(http::StatusCode::from_str(s)?.try_into()?)
            }
        }

        impl From<&'_ $T> for $T {
            #[inline]
            fn from(t: &$T) -> Self {
                t.to_owned()
            }
        }

        impl TryFrom<&'_ [u8]> for $T {
            type Error = $Invalid;

            #[inline]
            fn try_from(t: &[u8]) -> Result<Self, Self::Error> {
                $T::from_bytes(t)
            }
        }

        impl TryFrom<&'_ str> for $T {
            type Error = $Invalid;

            #[inline]
            fn try_from(t: &str) -> Result<Self, Self::Error> {
                t.parse()
            }
        }

        impl TryFrom<u16> for $T {
            type Error = $Invalid;

            #[inline]
            fn try_from(t: u16) -> Result<Self, Self::Error> {
                Self::from_u16(t)
            }
        }

        impl TryFrom<http::StatusCode> for $T {
            type Error = $NotAnError;

            fn try_from(value: http::StatusCode) -> Result<Self, Self::Error> {
                Self::from_status(value)
            }
        }
    };
}

impl ErrorStatusCode {
    error_status_code_constants! {
        /// 400 Bad Request
        /// [[RFC7231, Section 6.5.1](https://tools.ietf.org/html/rfc7231#section-6.5.1)]
        BAD_REQUEST;

        /// 401 Unauthorized
        /// [[RFC7235, Section 3.1](https://tools.ietf.org/html/rfc7235#section-3.1)]
        UNAUTHORIZED;
        /// 402 Payment Required
        /// [[RFC7231, Section 6.5.2](https://tools.ietf.org/html/rfc7231#section-6.5.2)]
        PAYMENT_REQUIRED;
        /// 403 Forbidden
        /// [[RFC7231, Section 6.5.3](https://tools.ietf.org/html/rfc7231#section-6.5.3)]
        FORBIDDEN;
        /// 404 Not Found
        /// [[RFC7231, Section 6.5.4](https://tools.ietf.org/html/rfc7231#section-6.5.4)]
        NOT_FOUND;
        /// 405 Method Not Allowed
        /// [[RFC7231, Section 6.5.5](https://tools.ietf.org/html/rfc7231#section-6.5.5)]
        METHOD_NOT_ALLOWED;
        /// 406 Not Acceptable
        /// [[RFC7231, Section 6.5.6](https://tools.ietf.org/html/rfc7231#section-6.5.6)]
        NOT_ACCEPTABLE;
        /// 407 Proxy Authentication Required
        /// [[RFC7235, Section 3.2](https://tools.ietf.org/html/rfc7235#section-3.2)]
        PROXY_AUTHENTICATION_REQUIRED;
        /// 408 Request Timeout
        /// [[RFC7231, Section 6.5.7](https://tools.ietf.org/html/rfc7231#section-6.5.7)]
        REQUEST_TIMEOUT;
        /// 409 Conflict
        /// [[RFC7231, Section 6.5.8](https://tools.ietf.org/html/rfc7231#section-6.5.8)]
        CONFLICT;
        /// 410 Gone
        /// [[RFC7231, Section 6.5.9](https://tools.ietf.org/html/rfc7231#section-6.5.9)]
        GONE;
        /// 411 Length Required
        /// [[RFC7231, Section 6.5.10](https://tools.ietf.org/html/rfc7231#section-6.5.10)]
        LENGTH_REQUIRED;
        /// 412 Precondition Failed
        /// [[RFC7232, Section 4.2](https://tools.ietf.org/html/rfc7232#section-4.2)]
        PRECONDITION_FAILED;
        /// 413 Payload Too Large
        /// [[RFC7231, Section 6.5.11](https://tools.ietf.org/html/rfc7231#section-6.5.11)]
        PAYLOAD_TOO_LARGE;
        /// 414 URI Too Long
        /// [[RFC7231, Section 6.5.12](https://tools.ietf.org/html/rfc7231#section-6.5.12)]
        URI_TOO_LONG;
        /// 415 Unsupported Media Type
        /// [[RFC7231, Section 6.5.13](https://tools.ietf.org/html/rfc7231#section-6.5.13)]
        UNSUPPORTED_MEDIA_TYPE;
        /// 416 Range Not Satisfiable
        /// [[RFC7233, Section 4.4](https://tools.ietf.org/html/rfc7233#section-4.4)]
        RANGE_NOT_SATISFIABLE;
        /// 417 Expectation Failed
        /// [[RFC7231, Section 6.5.14](https://tools.ietf.org/html/rfc7231#section-6.5.14)]
        EXPECTATION_FAILED;
        /// 418 I'm a teapot
        /// [curiously not registered by IANA but [RFC2324](https://tools.ietf.org/html/rfc2324)]
        IM_A_TEAPOT;

        /// 421 Misdirected Request
        /// [RFC7540, Section 9.1.2](https://tools.ietf.org/html/rfc7540#section-9.1.2)
        MISDIRECTED_REQUEST;
        /// 422 Unprocessable Entity
        /// [[RFC4918](https://tools.ietf.org/html/rfc4918)]
        UNPROCESSABLE_ENTITY;
        /// 423 Locked
        /// [[RFC4918](https://tools.ietf.org/html/rfc4918)]
        LOCKED;
        /// 424 Failed Dependency
        /// [[RFC4918](https://tools.ietf.org/html/rfc4918)]
        FAILED_DEPENDENCY;

        /// 426 Upgrade Required
        /// [[RFC7231, Section 6.5.15](https://tools.ietf.org/html/rfc7231#section-6.5.15)]
        UPGRADE_REQUIRED;

        /// 428 Precondition Required
        /// [[RFC6585](https://tools.ietf.org/html/rfc6585)]
        PRECONDITION_REQUIRED;
        /// 429 Too Many Requests
        /// [[RFC6585](https://tools.ietf.org/html/rfc6585)]
        TOO_MANY_REQUESTS;

        /// 431 Request Header Fields Too Large
        /// [[RFC6585](https://tools.ietf.org/html/rfc6585)]
        REQUEST_HEADER_FIELDS_TOO_LARGE;

        /// 451 Unavailable For Legal Reasons
        /// [[RFC7725](https://tools.ietf.org/html/rfc7725)]
        UNAVAILABLE_FOR_LEGAL_REASONS;

        /// 500 Internal Server Error
        /// [[RFC7231, Section 6.6.1](https://tools.ietf.org/html/rfc7231#section-6.6.1)]
        INTERNAL_SERVER_ERROR;
        /// 501 Not Implemented
        /// [[RFC7231, Section 6.6.2](https://tools.ietf.org/html/rfc7231#section-6.6.2)]
        NOT_IMPLEMENTED;
        /// 502 Bad Gateway
        /// [[RFC7231, Section 6.6.3](https://tools.ietf.org/html/rfc7231#section-6.6.3)]
        BAD_GATEWAY;
        /// 503 Service Unavailable
        /// [[RFC7231, Section 6.6.4](https://tools.ietf.org/html/rfc7231#section-6.6.4)]
        SERVICE_UNAVAILABLE;
        /// 504 Gateway Timeout
        /// [[RFC7231, Section 6.6.5](https://tools.ietf.org/html/rfc7231#section-6.6.5)]
        GATEWAY_TIMEOUT;
        /// 505 HTTP Version Not Supported
        /// [[RFC7231, Section 6.6.6](https://tools.ietf.org/html/rfc7231#section-6.6.6)]
        HTTP_VERSION_NOT_SUPPORTED;
        /// 506 Variant Also Negotiates
        /// [[RFC2295](https://tools.ietf.org/html/rfc2295)]
        VARIANT_ALSO_NEGOTIATES;
        /// 507 Insufficient Storage
        /// [[RFC4918](https://tools.ietf.org/html/rfc4918)]
        INSUFFICIENT_STORAGE;
        /// 508 Loop Detected
        /// [[RFC5842](https://tools.ietf.org/html/rfc5842)]
        LOOP_DETECTED;

        /// 510 Not Extended
        /// [[RFC2774](https://tools.ietf.org/html/rfc2774)]
        NOT_EXTENDED;
        /// 511 Network Authentication Required
        /// [[RFC6585](https://tools.ietf.org/html/rfc6585)]
        NETWORK_AUTHENTICATION_REQUIRED;
    }

    /// Converts an [`http::StatusCode`] into an [`ErrorStatusCode`].
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[`ErrorStatusCode`]`)` if the status code is a 4xx or 5xx
    ///   status code.
    /// - [`Err`]`(`[`NotAnError`]`)` if the status code is a 1xx, 2xx, or 3xx
    ///   status code.
    pub fn from_status(status: http::StatusCode) -> Result<Self, NotAnError> {
        if status.is_client_error() || status.is_server_error() {
            Ok(ErrorStatusCode(status))
        } else {
            Err(NotAnError(status))
        }
    }

    /// Converts a u16 to a status code.
    ///
    /// The function validates the correctness of the supplied `u16` It must
    /// be a HTTP client error (400-499) or server error (500-599).
    ///
    /// # Example
    ///
    /// ```
    /// use dropshot::ErrorStatusCode;
    ///
    /// // 404 is a client error
    /// let ok = ErrorStatusCode::from_u16(404).unwrap();
    /// assert_eq!(ok, ErrorStatusCode::NOT_FOUND);
    ///
    /// // 555 is a server error (although it lacks a well known meaning)
    /// let _ok = ErrorStatusCode::from_u16(555).unwrap();
    ///
    /// // 200 is a status code, but not an error.
    /// let err = ErrorStatusCode::from_u16(200);
    /// assert!(err.is_err());
    ///
    /// // 99 is out of range for any status code
    /// let err = ErrorStatusCode::from_u16(99);
    /// assert!(err.is_err());
    /// ```
    #[inline]
    pub fn from_u16(src: u16) -> Result<Self, InvalidErrorStatusCode> {
        let status = http::StatusCode::from_u16(src)?;
        Self::from_status(status).map_err(Into::into)
    }

    /// Refine this error status code into a [`ClientErrorStatusCode`].
    ///
    /// If this is a client error (4xx) status code, returns a
    /// [`ClientErrorStatusCode`] with that status. Otherwise, this method
    /// returns a [`NotAClientError`] error.
    #[inline]
    pub fn as_client_error(
        &self,
    ) -> Result<ClientErrorStatusCode, NotAClientError> {
        if self.is_client_error() {
            Ok(ClientErrorStatusCode(self.0))
        } else {
            Err(NotAClientError(self.0))
        }
    }

    /// Check if status is within 400-499.
    #[inline]
    pub fn is_client_error(&self) -> bool {
        self.0.is_client_error()
    }

    /// Check if status is within 500-599.
    #[inline]
    pub fn is_server_error(&self) -> bool {
        self.0.is_server_error()
    }
}

impl_status_code_wrapper! {
    impl StatusCode for ErrorStatusCode {
        type NotAnError = NotAnError;
        type Invalid = InvalidErrorStatusCode;
    }
}

/// An HTTP 4xx client error status code
///
/// This is a refinement of the [`http::StatusCode`] type that is limited to the
/// client error status code range (400-499). It may be constructed from any
/// [`http::StatusCode`] using the `TryFrom` implementation, which fails if the
/// status is not a 4xx status code.
///
/// Alternatively, constants are provided for known error status codes,
/// such as [`ClientErrorStatusCode::BAD_REQUEST`],
/// [`ClientErrorStatusCode::NOT_FOUND`], including those in
/// the IANA HTTP Status Code Registry](
/// https://www.iana.org/assignments/http-status-codes/http-status-codes.xhtml).
/// Using these constants avoids the fallible conversion from an [`http::StatusCode`].
///
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientErrorStatusCode(http::StatusCode);

impl ClientErrorStatusCode {
    error_status_code_constants! {
        /// 400 Bad Request
        /// [[RFC7231, Section 6.5.1](https://tools.ietf.org/html/rfc7231#section-6.5.1)]
        BAD_REQUEST;

        /// 401 Unauthorized
        /// [[RFC7235, Section 3.1](https://tools.ietf.org/html/rfc7235#section-3.1)]
        UNAUTHORIZED;
        /// 402 Payment Required
        /// [[RFC7231, Section 6.5.2](https://tools.ietf.org/html/rfc7231#section-6.5.2)]
        PAYMENT_REQUIRED;
        /// 403 Forbidden
        /// [[RFC7231, Section 6.5.3](https://tools.ietf.org/html/rfc7231#section-6.5.3)]
        FORBIDDEN;
        /// 404 Not Found
        /// [[RFC7231, Section 6.5.4](https://tools.ietf.org/html/rfc7231#section-6.5.4)]
        NOT_FOUND;
        /// 405 Method Not Allowed
        /// [[RFC7231, Section 6.5.5](https://tools.ietf.org/html/rfc7231#section-6.5.5)]
        METHOD_NOT_ALLOWED;
        /// 406 Not Acceptable
        /// [[RFC7231, Section 6.5.6](https://tools.ietf.org/html/rfc7231#section-6.5.6)]
        NOT_ACCEPTABLE;
        /// 407 Proxy Authentication Required
        /// [[RFC7235, Section 3.2](https://tools.ietf.org/html/rfc7235#section-3.2)]
        PROXY_AUTHENTICATION_REQUIRED;
        /// 408 Request Timeout
        /// [[RFC7231, Section 6.5.7](https://tools.ietf.org/html/rfc7231#section-6.5.7)]
        REQUEST_TIMEOUT;
        /// 409 Conflict
        /// [[RFC7231, Section 6.5.8](https://tools.ietf.org/html/rfc7231#section-6.5.8)]
        CONFLICT;
        /// 410 Gone
        /// [[RFC7231, Section 6.5.9](https://tools.ietf.org/html/rfc7231#section-6.5.9)]
        GONE;
        /// 411 Length Required
        /// [[RFC7231, Section 6.5.10](https://tools.ietf.org/html/rfc7231#section-6.5.10)]
        LENGTH_REQUIRED;
        /// 412 Precondition Failed
        /// [[RFC7232, Section 4.2](https://tools.ietf.org/html/rfc7232#section-4.2)]
        PRECONDITION_FAILED;
        /// 413 Payload Too Large
        /// [[RFC7231, Section 6.5.11](https://tools.ietf.org/html/rfc7231#section-6.5.11)]
        PAYLOAD_TOO_LARGE;
        /// 414 URI Too Long
        /// [[RFC7231, Section 6.5.12](https://tools.ietf.org/html/rfc7231#section-6.5.12)]
        URI_TOO_LONG;
        /// 415 Unsupported Media Type
        /// [[RFC7231, Section 6.5.13](https://tools.ietf.org/html/rfc7231#section-6.5.13)]
        UNSUPPORTED_MEDIA_TYPE;
        /// 416 Range Not Satisfiable
        /// [[RFC7233, Section 4.4](https://tools.ietf.org/html/rfc7233#section-4.4)]
        RANGE_NOT_SATISFIABLE;
        /// 417 Expectation Failed
        /// [[RFC7231, Section 6.5.14](https://tools.ietf.org/html/rfc7231#section-6.5.14)]
        EXPECTATION_FAILED;
        /// 418 I'm a teapot
        /// [curiously not registered by IANA but [RFC2324](https://tools.ietf.org/html/rfc2324)]
        IM_A_TEAPOT;

        /// 421 Misdirected Request
        /// [RFC7540, Section 9.1.2](https://tools.ietf.org/html/rfc7540#section-9.1.2)
        MISDIRECTED_REQUEST;
        /// 422 Unprocessable Entity
        /// [[RFC4918](https://tools.ietf.org/html/rfc4918)]
        UNPROCESSABLE_ENTITY;
        /// 423 Locked
        /// [[RFC4918](https://tools.ietf.org/html/rfc4918)]
        LOCKED;
        /// 424 Failed Dependency
        /// [[RFC4918](https://tools.ietf.org/html/rfc4918)]
        FAILED_DEPENDENCY;

        /// 426 Upgrade Required
        /// [[RFC7231, Section 6.5.15](https://tools.ietf.org/html/rfc7231#section-6.5.15)]
        UPGRADE_REQUIRED;

        /// 428 Precondition Required
        /// [[RFC6585](https://tools.ietf.org/html/rfc6585)]
        PRECONDITION_REQUIRED;
        /// 429 Too Many Requests
        /// [[RFC6585](https://tools.ietf.org/html/rfc6585)]
        TOO_MANY_REQUESTS;

        /// 431 Request Header Fields Too Large
        /// [[RFC6585](https://tools.ietf.org/html/rfc6585)]
        REQUEST_HEADER_FIELDS_TOO_LARGE;

        /// 451 Unavailable For Legal Reasons
        /// [[RFC7725](https://tools.ietf.org/html/rfc7725)]
        UNAVAILABLE_FOR_LEGAL_REASONS;
    }

    /// Converts an [`http::StatusCode`] into a [`ClientErrorStatusCode`].
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[`ClientErrorStatusCode`]`)` if the status code is a 4xx
    ///   status code.
    /// - [`Err`]`(`[`NotAnError`]`)` if the status code is not a 4xx status code.
    pub fn from_status(
        status: http::StatusCode,
    ) -> Result<Self, NotAClientError> {
        if status.is_client_error() {
            Ok(Self(status))
        } else {
            Err(NotAClientError(status))
        }
    }

    /// Converts a `u16` to a [`ClientErrorStatusCode`]
    ///
    /// The function validates the correctness of the supplied `u16` It must
    /// be a HTTP client error (400-499).
    ///
    /// # Example
    ///
    /// ```
    /// use dropshot::ClientErrorStatusCode;
    ///
    /// // 404 is a client error
    /// let ok = ClientErrorStatusCode::from_u16(404).unwrap();
    /// assert_eq!(ok, ClientErrorStatusCode::NOT_FOUND);
    ///
    /// // 444 is a client error (although it lacks a well known meaning)
    /// let _ok = ClientErrorStatusCode::from_u16(555).unwrap();
    ///
    /// // 500 is a status code, but not an error.
    /// let err = ClientErrorStatusCode::from_u16(200);
    /// assert!(err.is_err());
    ///
    /// // 99 is out of range for any status code
    /// let err = ClientErrorStatusCode::from_u16(99);
    /// assert!(err.is_err());
    /// ```
    #[inline]
    pub fn from_u16(src: u16) -> Result<Self, InvalidClientErrorStatusCode> {
        let status = http::StatusCode::from_u16(src)?;
        Self::from_status(status).map_err(Into::into)
    }
}

impl_status_code_wrapper! {
    impl StatusCode for ClientErrorStatusCode {
        type NotAnError = NotAClientError;
        type Invalid = InvalidClientErrorStatusCode;
    }
}

impl TryFrom<ErrorStatusCode> for ClientErrorStatusCode {
    type Error = NotAClientError;
    fn try_from(error: ErrorStatusCode) -> Result<Self, Self::Error> {
        error.as_client_error()
    }
}

impl From<ClientErrorStatusCode> for ErrorStatusCode {
    #[inline]
    fn from(error: ClientErrorStatusCode) -> Self {
        Self(error.0)
    }
}

impl TryFrom<&'_ ErrorStatusCode> for ClientErrorStatusCode {
    type Error = NotAClientError;
    fn try_from(error: &ErrorStatusCode) -> Result<Self, Self::Error> {
        error.as_client_error()
    }
}

impl From<&'_ ClientErrorStatusCode> for ErrorStatusCode {
    #[inline]
    fn from(error: &ClientErrorStatusCode) -> Self {
        Self(error.0)
    }
}

impl PartialEq<ErrorStatusCode> for ClientErrorStatusCode {
    #[inline]
    fn eq(&self, other: &ErrorStatusCode) -> bool {
        self.0 == other.0
    }
}

impl PartialEq<ClientErrorStatusCode> for ErrorStatusCode {
    #[inline]
    fn eq(&self, other: &ClientErrorStatusCode) -> bool {
        self.0 == other.0
    }
}

#[derive(Debug, thiserror::Error)]
#[error("status code {0} is not a 4xx or 5xx error")]
pub struct NotAnError(http::StatusCode);

#[derive(Debug, thiserror::Error)]
#[error("status code {0} is not a 4xx client error")]
pub struct NotAClientError(http::StatusCode);

/// A possible error value when converting an [`ErrorStatusCode`] from a `u16`
/// or `&str`.
#[derive(Debug, thiserror::Error)]
pub enum InvalidErrorStatusCode {
    /// The input was not an error (4xx or 5xx) status code.
    #[error(transparent)]
    NotAnError(#[from] NotAnError),
    /// The nput was not a valid number, was less than 100, or was greater than
    /// 999.
    #[error(transparent)]
    InvalidStatus(#[from] http::status::InvalidStatusCode),
}

/// A possible error value when converting a [`ClientErrorStatusCode`] from a `u16`
/// or `&str`.
#[derive(Debug, thiserror::Error)]
pub enum InvalidClientErrorStatusCode {
    /// The input was not a client error (4xx) status code.
    #[error(transparent)]
    NotAClientError(#[from] NotAClientError),
    /// The nput was not a valid number, was less than 100, or was greater than
    /// 999.
    #[error(transparent)]
    InvalidStatus(#[from] http::status::InvalidStatusCode),
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
