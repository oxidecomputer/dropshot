// Copyright 2024 Oxide Computer Company
// Copyright 2017 http-rs authors

//! Newtypes around [`http::StatusCode`] that are limited to status ranges
//! representing errors.

use serde::{Deserialize, Serialize};
use std::fmt;

/// An HTTP 4xx (client error) or 5xx (server error) status code.
///
/// This is a refinement of the [`http::StatusCode`] type that is limited to the
/// error status code ranges. It may be constructed from any
/// [`http::StatusCode`] using the `TryFrom` implementation, which fails if the
/// status is not a 4xx or 5xx status code.
///
/// Alternatively, constants are provided for known error status codes, such as
/// [`ErrorStatusCode::BAD_REQUEST`], [`ErrorStatusCode::NOT_FOUND`],
/// [`ErrorStatusCode::INTERNAL_SERVER_ERROR`], and so on, including those in
/// the IANA HTTP Status Code Registry][iana]. Using these constants avoids the
/// fallible conversion from an [`http::StatusCode`].
///
/// [iana]: https://www.iana.org/assignments/http-status-codes/http-status-codes.xhtml
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(try_from = "u16", into = "u16")]
pub struct ErrorStatusCode(http::StatusCode);

// Generate constants for a `http::StatusCode` wrapper type that re-export the
// provided constants defined by `http::StatusCode.`
macro_rules! error_status_code_constants {
    ( $($(#[$docs:meta])* $name:ident;)+ ) => {
        $(
            $(#[$docs])*
            pub const $name: Self = Self(http::StatusCode::$name);
        )+
    }
}

// Implement conversions and traits for `http::StatusCode` wrapper types. This
// generates conversions between the wrapper type and `http::StatusCode`,
// conversions between the wrapper and types that `StatusCode` has conversions
// between, and equality and fmt implementations.
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

            /// Returns the [`http::StatusCode`] corresponding to this error
            /// status code.
            ///
            /// # Note
            ///
            /// This is the same as the `Into<http::StatusCode>` implementation,
            /// but included as an inherent method because that implementation
            /// doesn't appear in rustdocs, as well as a way to force the type
            /// instead of relying on inference.
            pub fn as_status(&self) -> http::StatusCode {
                self.0
            }

            /// Returns the `u16` corresponding to this `error status code.
            ///
            /// # Note
            ///
            /// This is the same as the `Into<u16>` implementation, but included
            /// as an inherent method because that implementation doesn't appear
            /// in rustdocs, as well as a way to force the type instead of
            /// relying on inference.
            ///
            /// This method wraps the [`http::StatusCode::as_u16`] method.
            pub fn as_u16(&self) -> u16 {
                self.0.as_u16()
            }

            /// Returns a `&str` representation of the `StatusCode`
            ///
            /// The return value only includes a numerical representation of the
            /// status code. The canonical reason is not included.
            ///
            /// This method wraps the [`http::StatusCode::as_str`] method.
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }

            /// Get the standardised `reason-phrase` for this status code.
            ///
            /// This is mostly here for servers writing responses, but could
            /// potentially have application at other times.
            ///
            /// The reason phrase is defined as being exclusively for human
            /// readers. You should avoid deriving any meaning from it at all
            /// costs.
            ///
            /// Bear in mind also that in HTTP/2.0 and HTTP/3.0 the reason
            /// phrase is abolished from transmission, and so this canonical
            /// reason phrase really is the only reason phrase you’ll find.
            ///
            /// This method wraps the [`http::StatusCode::canonical_reason`]
            /// method.
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
    // These constants are copied from the `http` crate's `StatusCode` type.
    // Should new status codes be standardized and added upstream, we should add
    // them to this list.
    error_status_code_constants! {
        /// 400 Bad Request [[RFC7231, Section
        /// 6.5.1](https://tools.ietf.org/html/rfc7231#section-6.5.1)]
        BAD_REQUEST;

        /// 401 Unauthorized [[RFC7235, Section
        /// 3.1](https://tools.ietf.org/html/rfc7235#section-3.1)]
        UNAUTHORIZED;
        /// 402 Payment Required [[RFC7231, Section
        /// 6.5.2](https://tools.ietf.org/html/rfc7231#section-6.5.2)]
        PAYMENT_REQUIRED;
        /// 403 Forbidden [[RFC7231, Section
        /// 6.5.3](https://tools.ietf.org/html/rfc7231#section-6.5.3)]
        FORBIDDEN;
        /// 404 Not Found [[RFC7231, Section
        /// 6.5.4](https://tools.ietf.org/html/rfc7231#section-6.5.4)]
        NOT_FOUND;
        /// 405 Method Not Allowed [[RFC7231, Section
        /// 6.5.5](https://tools.ietf.org/html/rfc7231#section-6.5.5)]
        METHOD_NOT_ALLOWED;
        /// 406 Not Acceptable [[RFC7231, Section
        /// 6.5.6](https://tools.ietf.org/html/rfc7231#section-6.5.6)]
        NOT_ACCEPTABLE;
        /// 407 Proxy Authentication Required [[RFC7235, Section
        /// 3.2](https://tools.ietf.org/html/rfc7235#section-3.2)]
        PROXY_AUTHENTICATION_REQUIRED;
        /// 408 Request Timeout [[RFC7231, Section
        /// 6.5.7](https://tools.ietf.org/html/rfc7231#section-6.5.7)]
        REQUEST_TIMEOUT;
        /// 409 Conflict [[RFC7231, Section
        /// 6.5.8](https://tools.ietf.org/html/rfc7231#section-6.5.8)]
        CONFLICT;
        /// 410 Gone [[RFC7231, Section
        /// 6.5.9](https://tools.ietf.org/html/rfc7231#section-6.5.9)]
        GONE;
        /// 411 Length Required [[RFC7231, Section
        /// 6.5.10](https://tools.ietf.org/html/rfc7231#section-6.5.10)]
        LENGTH_REQUIRED;
        /// 412 Precondition Failed [[RFC7232, Section
        /// 4.2](https://tools.ietf.org/html/rfc7232#section-4.2)]
        PRECONDITION_FAILED;
        /// 413 Payload Too Large [[RFC7231, Section
        /// 6.5.11](https://tools.ietf.org/html/rfc7231#section-6.5.11)]
        PAYLOAD_TOO_LARGE;
        /// 414 URI Too Long [[RFC7231, Section
        /// 6.5.12](https://tools.ietf.org/html/rfc7231#section-6.5.12)]
        URI_TOO_LONG;
        /// 415 Unsupported Media Type [[RFC7231, Section
        /// 6.5.13](https://tools.ietf.org/html/rfc7231#section-6.5.13)]
        UNSUPPORTED_MEDIA_TYPE;
        /// 416 Range Not Satisfiable [[RFC7233, Section
        /// 4.4](https://tools.ietf.org/html/rfc7233#section-4.4)]
        RANGE_NOT_SATISFIABLE;
        /// 417 Expectation Failed [[RFC7231, Section
        /// 6.5.14](https://tools.ietf.org/html/rfc7231#section-6.5.14)]
        EXPECTATION_FAILED;
        /// 418 I'm a teapot [curiously not registered by IANA but
        /// [RFC2324](https://tools.ietf.org/html/rfc2324)]
        IM_A_TEAPOT;

        /// 421 Misdirected Request [RFC7540, Section
        /// 9.1.2](https://tools.ietf.org/html/rfc7540#section-9.1.2)
        MISDIRECTED_REQUEST;
        /// 422 Unprocessable Entity
        /// [[RFC4918](https://tools.ietf.org/html/rfc4918)]
        UNPROCESSABLE_ENTITY;
        /// 423 Locked [[RFC4918](https://tools.ietf.org/html/rfc4918)]
        LOCKED;
        /// 424 Failed Dependency
        /// [[RFC4918](https://tools.ietf.org/html/rfc4918)]
        FAILED_DEPENDENCY;

        /// 426 Upgrade Required [[RFC7231, Section
        /// 6.5.15](https://tools.ietf.org/html/rfc7231#section-6.5.15)]
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

        /// 500 Internal Server Error [[RFC7231, Section
        /// 6.6.1](https://tools.ietf.org/html/rfc7231#section-6.6.1)]
        INTERNAL_SERVER_ERROR;
        /// 501 Not Implemented [[RFC7231, Section
        /// 6.6.2](https://tools.ietf.org/html/rfc7231#section-6.6.2)]
        NOT_IMPLEMENTED;
        /// 502 Bad Gateway [[RFC7231, Section
        /// 6.6.3](https://tools.ietf.org/html/rfc7231#section-6.6.3)]
        BAD_GATEWAY;
        /// 503 Service Unavailable [[RFC7231, Section
        /// 6.6.4](https://tools.ietf.org/html/rfc7231#section-6.6.4)]
        SERVICE_UNAVAILABLE;
        /// 504 Gateway Timeout [[RFC7231, Section
        /// 6.6.5](https://tools.ietf.org/html/rfc7231#section-6.6.5)]
        GATEWAY_TIMEOUT;
        /// 505 HTTP Version Not Supported [[RFC7231, Section
        /// 6.6.6](https://tools.ietf.org/html/rfc7231#section-6.6.6)]
        HTTP_VERSION_NOT_SUPPORTED;
        /// 506 Variant Also Negotiates
        /// [[RFC2295](https://tools.ietf.org/html/rfc2295)]
        VARIANT_ALSO_NEGOTIATES;
        /// 507 Insufficient Storage
        /// [[RFC4918](https://tools.ietf.org/html/rfc4918)]
        INSUFFICIENT_STORAGE;
        /// 508 Loop Detected [[RFC5842](https://tools.ietf.org/html/rfc5842)]
        LOOP_DETECTED;

        /// 510 Not Extended [[RFC2774](https://tools.ietf.org/html/rfc2774)]
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
    /// The function validates the correctness of the supplied `u16` It must be
    /// a HTTP client error (400-499) or server error (500-599).
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
    pub fn from_u16(src: u16) -> Result<Self, InvalidErrorStatusCode> {
        let status = http::StatusCode::from_u16(src)?;
        Self::from_status(status).map_err(Into::into)
    }

    /// Refine this error status code into a [`ClientErrorStatusCode`].
    ///
    /// If this is a client error (4xx) status code, returns a
    /// [`ClientErrorStatusCode`] with that status. Otherwise, this method
    /// returns a [`NotAClientError`] error.
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
    pub fn is_client_error(&self) -> bool {
        self.0.is_client_error()
    }

    /// Check if status is within 500-599.
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

impl schemars::JsonSchema for ErrorStatusCode {
    fn schema_name() -> String {
        "ErrorStatusCode".to_string()
    }
    fn schema_id() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("dropshot::ErrorStatusCode")
    }

    fn json_schema(
        _generator: &mut schemars::gen::SchemaGenerator,
    ) -> schemars::schema::Schema {
        schemars::schema::SchemaObject {
            metadata: Some(Box::new(schemars::schema::Metadata {
                title: Some("An HTTP error status code".to_string()),
                description: Some(
                    "An HTTP status code in the error range (4xx or 5xx)"
                        .to_string(),
                ),
                examples: vec![
                    "400".into(),
                    "404".into(),
                    "500".into(),
                    "503".into(),
                ],
                ..Default::default()
            })),
            instance_type: Some(schemars::schema::InstanceType::Integer.into()),
            number: Some(Box::new(schemars::schema::NumberValidation {
                minimum: Some(400.0),
                maximum: Some(599.0),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

/// An HTTP 4xx client error status code
///
/// This is a refinement of the [`http::StatusCode`] type that is limited to the
/// client error status code range (400-499). It may be constructed from any
/// [`http::StatusCode`] using the `TryFrom` implementation, which fails if the
/// status is not a 4xx status code.
///
/// Alternatively, constants are provided for known error status codes, such as
/// [`ClientErrorStatusCode::BAD_REQUEST`],
/// [`ClientErrorStatusCode::NOT_FOUND`], including those in the IANA HTTP
/// Status Code Registry][iana]. Using these constants avoids the fallible
/// conversion from an [`http::StatusCode`].
///
/// [iana]: https://www.iana.org/assignments/http-status-codes/http-status-codes.xhtml
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(try_from = "u16", into = "u16")]
pub struct ClientErrorStatusCode(http::StatusCode);

impl ClientErrorStatusCode {
    // These constants are copied from the `http` crate's `StatusCode` type.
    // Should new status codes be standardized and added upstream, we should add
    // them to this list.
    error_status_code_constants! {
        /// 400 Bad Request [[RFC7231, Section
        /// 6.5.1](https://tools.ietf.org/html/rfc7231#section-6.5.1)]
        BAD_REQUEST;

        /// 401 Unauthorized [[RFC7235, Section
        /// 3.1](https://tools.ietf.org/html/rfc7235#section-3.1)]
        UNAUTHORIZED;
        /// 402 Payment Required [[RFC7231, Section
        /// 6.5.2](https://tools.ietf.org/html/rfc7231#section-6.5.2)]
        PAYMENT_REQUIRED;
        /// 403 Forbidden [[RFC7231, Section
        /// 6.5.3](https://tools.ietf.org/html/rfc7231#section-6.5.3)]
        FORBIDDEN;
        /// 404 Not Found [[RFC7231, Section
        /// 6.5.4](https://tools.ietf.org/html/rfc7231#section-6.5.4)]
        NOT_FOUND;
        /// 405 Method Not Allowed [[RFC7231, Section
        /// 6.5.5](https://tools.ietf.org/html/rfc7231#section-6.5.5)]
        METHOD_NOT_ALLOWED;
        /// 406 Not Acceptable [[RFC7231, Section
        /// 6.5.6](https://tools.ietf.org/html/rfc7231#section-6.5.6)]
        NOT_ACCEPTABLE;
        /// 407 Proxy Authentication Required [[RFC7235, Section
        /// 3.2](https://tools.ietf.org/html/rfc7235#section-3.2)]
        PROXY_AUTHENTICATION_REQUIRED;
        /// 408 Request Timeout [[RFC7231, Section
        /// 6.5.7](https://tools.ietf.org/html/rfc7231#section-6.5.7)]
        REQUEST_TIMEOUT;
        /// 409 Conflict [[RFC7231, Section
        /// 6.5.8](https://tools.ietf.org/html/rfc7231#section-6.5.8)]
        CONFLICT;
        /// 410 Gone [[RFC7231, Section
        /// 6.5.9](https://tools.ietf.org/html/rfc7231#section-6.5.9)]
        GONE;
        /// 411 Length Required [[RFC7231, Section
        /// 6.5.10](https://tools.ietf.org/html/rfc7231#section-6.5.10)]
        LENGTH_REQUIRED;
        /// 412 Precondition Failed [[RFC7232, Section
        /// 4.2](https://tools.ietf.org/html/rfc7232#section-4.2)]
        PRECONDITION_FAILED;
        /// 413 Payload Too Large [[RFC7231, Section
        /// 6.5.11](https://tools.ietf.org/html/rfc7231#section-6.5.11)]
        PAYLOAD_TOO_LARGE;
        /// 414 URI Too Long [[RFC7231, Section
        /// 6.5.12](https://tools.ietf.org/html/rfc7231#section-6.5.12)]
        URI_TOO_LONG;
        /// 415 Unsupported Media Type [[RFC7231, Section
        /// 6.5.13](https://tools.ietf.org/html/rfc7231#section-6.5.13)]
        UNSUPPORTED_MEDIA_TYPE;
        /// 416 Range Not Satisfiable [[RFC7233, Section
        /// 4.4](https://tools.ietf.org/html/rfc7233#section-4.4)]
        RANGE_NOT_SATISFIABLE;
        /// 417 Expectation Failed [[RFC7231, Section
        /// 6.5.14](https://tools.ietf.org/html/rfc7231#section-6.5.14)]
        EXPECTATION_FAILED;
        /// 418 I'm a teapot [curiously not registered by IANA but
        /// [RFC2324](https://tools.ietf.org/html/rfc2324)]
        IM_A_TEAPOT;

        /// 421 Misdirected Request [RFC7540, Section
        /// 9.1.2](https://tools.ietf.org/html/rfc7540#section-9.1.2)
        MISDIRECTED_REQUEST;
        /// 422 Unprocessable Entity
        /// [[RFC4918](https://tools.ietf.org/html/rfc4918)]
        UNPROCESSABLE_ENTITY;
        /// 423 Locked [[RFC4918](https://tools.ietf.org/html/rfc4918)]
        LOCKED;
        /// 424 Failed Dependency
        /// [[RFC4918](https://tools.ietf.org/html/rfc4918)]
        FAILED_DEPENDENCY;

        /// 426 Upgrade Required [[RFC7231, Section
        /// 6.5.15](https://tools.ietf.org/html/rfc7231#section-6.5.15)]
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
    /// - [`Err`]`(`[`NotAnError`]`)` if the status code is not a 4xx status
    ///   code.
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
    /// The function validates the correctness of the supplied `u16` It must be
    /// a HTTP client error (400-499).
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
    /// let _ok = ClientErrorStatusCode::from_u16(444).unwrap();
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

impl schemars::JsonSchema for ClientErrorStatusCode {
    fn schema_name() -> String {
        "ClientErrorStatusCode".to_string()
    }
    fn schema_id() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("dropshot::ClientErrorStatusCode")
    }

    fn json_schema(
        _generator: &mut schemars::gen::SchemaGenerator,
    ) -> schemars::schema::Schema {
        schemars::schema::SchemaObject {
            metadata: Some(Box::new(schemars::schema::Metadata {
                title: Some("An HTTP client error status code".to_string()),
                description: Some(
                    "An HTTP status code in the client error range (4xx)"
                        .to_string(),
                ),
                examples: vec!["400".into(), "404".into(), "451".into()],
                ..Default::default()
            })),
            instance_type: Some(schemars::schema::InstanceType::Integer.into()),
            number: Some(Box::new(schemars::schema::NumberValidation {
                minimum: Some(400.0),
                maximum: Some(499.0),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
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
    /// The input was not a valid number, was less than 100, or was greater than
    /// 999.
    #[error(transparent)]
    InvalidStatus(#[from] http::status::InvalidStatusCode),
}

/// A possible error value when converting a [`ClientErrorStatusCode`] from a
/// `u16` or `&str`.
#[derive(Debug, thiserror::Error)]
pub enum InvalidClientErrorStatusCode {
    /// The input was not a client error (4xx) status code.
    #[error(transparent)]
    NotAClientError(#[from] NotAClientError),
    /// The input was not a valid number, was less than 100, or was greater than
    /// 999.
    #[error(transparent)]
    InvalidStatus(#[from] http::status::InvalidStatusCode),
}
