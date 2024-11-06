// Copyright 2023 Oxide Computer Company

//! Body-related extractor(s)

use super::ExtractorError;
use crate::api_description::ApiEndpointParameter;
use crate::api_description::ApiSchemaGenerator;
use crate::api_description::{ApiEndpointBodyContentType, ExtensionMode};
use crate::error::HttpError;
use crate::http_util::http_dump_body;
use crate::http_util::CONTENT_TYPE_JSON;
use crate::schema_util::make_subschema_for;
use crate::server::ServerContext;
use crate::ExclusiveExtractor;
use crate::ExtractorMetadata;
use crate::RequestContext;
use async_trait::async_trait;
use bytes::BufMut;
use bytes::Bytes;
use bytes::BytesMut;
use futures::Stream;
use futures::TryStreamExt;
use http_body_util::BodyExt;
use schemars::schema::InstanceType;
use schemars::schema::SchemaObject;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use std::fmt::Debug;

// TypedBody: body extractor for formats that can be deserialized to a specific
// type.  Only JSON is currently supported.

/// `TypedBody<BodyType>` is an extractor used to deserialize an instance of
/// `BodyType` from an HTTP request body.  `BodyType` is any structure of yours
/// that implements `serde::Deserialize`.  See this module's documentation for
/// more information.
#[derive(Debug)]
pub struct TypedBody<BodyType: JsonSchema + DeserializeOwned + Send + Sync> {
    inner: BodyType,
}

impl<BodyType: JsonSchema + DeserializeOwned + Send + Sync>
    TypedBody<BodyType>
{
    // TODO drop this in favor of Deref?  + Display and Debug for convenience?
    pub fn into_inner(self) -> BodyType {
        self.inner
    }
}

#[derive(Debug)]
pub struct MultipartBody {
    pub content: multer::Multipart<'static>,
}

/// Errors returned by the [`MultipartBody`] extractor.
#[derive(Debug, thiserror::Error)]
pub enum MultipartBodyError {
    /// The request was missing a `Content-Type` header.
    #[error("missing content-type header")]
    MissingContentType,
    /// The request's `Content-Type` header was not a valid UTF-8 string.
    #[error("invalid content type: {0}")]
    InvalidContentType(http::header::ToStrError),
    /// The request's `Content-Type` header was missing the `boundary=` part.
    #[error("missing boundary in content-type header")]
    MissingBoundary,
}

impl From<MultipartBodyError> for HttpError {
    fn from(error: MultipartBodyError) -> Self {
        HttpError::for_client_error(
            None,
            error.recommended_status_code(),
            error.to_string(),
        )
    }
}

impl MultipartBodyError {
    /// Returns the recommended status code for this error.
    ///
    /// This can be used when constructing a HTTP response for this error. These
    /// are the status codes used by the `From<MultipartBodyError>` implementation
    /// for [`HttpError`].
    pub fn recommended_status_code(&self) -> http::StatusCode {
        match self {
            // Invalid or unsupported content-type headers should return 415
            // Unsupported Media Type
            Self::MissingContentType | Self::InvalidContentType(_) => {
                http::StatusCode::UNSUPPORTED_MEDIA_TYPE
            }
            // Everything else gets a generic `400 Bad Request`.
            _ => http::StatusCode::BAD_REQUEST,
        }
    }
}

#[async_trait]
impl ExclusiveExtractor for MultipartBody {
    async fn from_request<Context: ServerContext>(
        _rqctx: &RequestContext<Context>,
        request: hyper::Request<crate::Body>,
    ) -> Result<Self, ExtractorError> {
        let (parts, body) = request.into_parts();
        // Get the content-type header.
        let content_type = parts
            .headers
            .get(http::header::CONTENT_TYPE)
            .ok_or(MultipartBodyError::MissingContentType)?
            .to_str()
            .map_err(MultipartBodyError::InvalidContentType)?;
        // The boundary is the string after the "boundary=" part of the
        // content-type header.
        let boundary = content_type
            .split("boundary=")
            .nth(1)
            .ok_or(MultipartBodyError::MissingBoundary)?;
        Ok(MultipartBody {
            content: multer::Multipart::new(
                body.into_data_stream(),
                boundary.to_string(),
            ),
        })
    }

    fn metadata(
        _content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata {
        let body = ApiEndpointParameter::new_body(
            ApiEndpointBodyContentType::MultipartFormData,
            true,
            ApiSchemaGenerator::Static {
                schema: Box::new(
                    SchemaObject {
                        instance_type: Some(InstanceType::String.into()),
                        format: Some(String::from("binary")),
                        ..Default::default()
                    }
                    .into(),
                ),
                dependencies: indexmap::IndexMap::default(),
            },
            vec![],
        );
        ExtractorMetadata {
            extension_mode: ExtensionMode::None,
            parameters: vec![body],
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TypedBodyError {
    #[error("invalid content type: {0}")]
    InvalidContentType(#[source] http::header::ToStrError),
    #[error("unsupported mime type: {0:?}")]
    UnsupportedMimeType(String),
    #[error("expected content type \"{}\", got \"{}\"", expected.mime_type(), requested.mime_type())]
    UnexpectedMimeType {
        expected: ApiEndpointBodyContentType,
        requested: ApiEndpointBodyContentType,
    },
    #[error("unable to parse JSON body: {0}")]
    JsonParse(#[from] serde_path_to_error::Error<serde_json::Error>),
    #[error("unable to parse URL-encoded body: {0}")]
    UrlEncodedParse(
        #[from] serde_path_to_error::Error<serde_urlencoded::de::Error>,
    ),
    #[error(transparent)]
    StreamingBody(#[from] StreamingBodyError),
}

impl From<TypedBodyError> for HttpError {
    fn from(error: TypedBodyError) -> Self {
        match error {
            TypedBodyError::StreamingBody(e) => e.into(),
            _ => HttpError::for_client_error(
                None,
                error.recommended_status_code(),
                error.to_string(),
            ),
        }
    }
}

impl TypedBodyError {
    /// Returns the recommended status code for this error.
    ///
    /// This can be used when constructing a HTTP response for this error. These
    /// are the status codes used by the `From<TypedBodyError>` implementation
    /// for [`HttpError`].
    pub fn recommended_status_code(&self) -> http::StatusCode {
        match self {
            // Invalid or unsupported content-type headers should return 415
            // Unsupported Media Type
            Self::InvalidContentType(_)
            | Self::UnsupportedMimeType(_)
            | Self::UnexpectedMimeType { .. } => {
                http::StatusCode::UNSUPPORTED_MEDIA_TYPE
            }
            Self::StreamingBody(e) => e.recommended_status_code(),
            // Everything else gets a generic `400 Bad Request`.
            _ => http::StatusCode::BAD_REQUEST,
        }
    }
}

/// Given an HTTP request, attempt to read the body, parse it according
/// to the content type, and deserialize it to an instance of `BodyType`.
async fn http_request_load_body<Context: ServerContext, BodyType>(
    rqctx: &RequestContext<Context>,
    request: hyper::Request<crate::Body>,
) -> Result<TypedBody<BodyType>, TypedBodyError>
where
    BodyType: JsonSchema + DeserializeOwned + Send + Sync,
{
    let server = &rqctx.server;
    let (parts, body) = request.into_parts();
    let body = StreamingBody::new(body, server.config.request_body_max_bytes)
        .into_bytes_mut()
        .await?;

    // RFC 7231 §3.1.1.1: media types are case insensitive and may
    // be followed by whitespace and/or a parameter (e.g., charset),
    // which we currently ignore.
    let content_type = parts
        .headers
        .get(http::header::CONTENT_TYPE)
        .map(|hv| hv.to_str().map_err(TypedBodyError::InvalidContentType))
        .unwrap_or(Ok(CONTENT_TYPE_JSON))?;
    let end = content_type.find(';').unwrap_or_else(|| content_type.len());
    let mime_type = content_type[..end].trim_end().to_lowercase();
    let body_content_type =
        ApiEndpointBodyContentType::from_mime_type(&mime_type)
            .map_err(TypedBodyError::UnsupportedMimeType)?;
    let expected_content_type = rqctx.body_content_type.clone();

    use ApiEndpointBodyContentType::*;

    let content = match (expected_content_type, body_content_type) {
        (Json, Json) => {
            let jd = &mut serde_json::Deserializer::from_slice(&body);
            serde_path_to_error::deserialize(jd)?
        }
        (UrlEncoded, UrlEncoded) => {
            let ud = serde_urlencoded::Deserializer::new(
                form_urlencoded::parse(&body),
            );
            serde_path_to_error::deserialize(ud)?
        }
        (expected, requested) => {
            return Err(TypedBodyError::UnexpectedMimeType {
                expected,
                requested,
            })
        }
    };
    Ok(TypedBody { inner: content })
}

// The `ExclusiveExtractor` implementation for TypedBody<BodyType> describes how
// to construct an instance of `TypedBody<BodyType>` from an HTTP request:
// namely, by reading the request body and parsing it as JSON into type
// `BodyType`.  TODO-cleanup We shouldn't have to use the "'static" bound on
// `BodyType` here.  It seems like we ought to be able to use 'async_trait, but
// that doesn't seem to be defined.
#[async_trait]
impl<BodyType> ExclusiveExtractor for TypedBody<BodyType>
where
    BodyType: JsonSchema + DeserializeOwned + Send + Sync + 'static,
{
    async fn from_request<Context: ServerContext>(
        rqctx: &RequestContext<Context>,
        request: hyper::Request<crate::Body>,
    ) -> Result<TypedBody<BodyType>, ExtractorError> {
        http_request_load_body(rqctx, request).await.map_err(Into::into)
    }

    fn metadata(content_type: ApiEndpointBodyContentType) -> ExtractorMetadata {
        let body = ApiEndpointParameter::new_body(
            content_type,
            true,
            ApiSchemaGenerator::Gen {
                name: BodyType::schema_name,
                schema: make_subschema_for::<BodyType>,
            },
            vec![],
        );
        ExtractorMetadata {
            extension_mode: ExtensionMode::None,
            parameters: vec![body],
        }
    }
}

// UntypedBody: body extractor for a plain array of bytes of a body.

/// `UntypedBody` is an extractor for reading in the contents of the HTTP request
/// body and making the raw bytes directly available to the consumer.
#[derive(Debug)]
pub struct UntypedBody {
    content: Bytes,
}

impl UntypedBody {
    /// Returns a byte slice of the underlying body content.
    // TODO drop this in favor of Deref?  + Display and Debug for convenience?
    pub fn as_bytes(&self) -> &[u8] {
        &self.content
    }

    /// Convenience wrapper to convert the body to a UTF-8 string slice,
    /// returning a 400-level error if the body is not valid UTF-8.
    pub fn as_str(&self) -> Result<&str, HttpError> {
        std::str::from_utf8(self.as_bytes()).map_err(|e| {
            HttpError::for_bad_request(
                None,
                format!("failed to parse body as UTF-8 string: {}", e),
            )
        })
    }
}

#[async_trait]
impl ExclusiveExtractor for UntypedBody {
    async fn from_request<Context: ServerContext>(
        rqctx: &RequestContext<Context>,
        request: hyper::Request<crate::Body>,
    ) -> Result<UntypedBody, ExtractorError> {
        let server = &rqctx.server;
        let body = request.into_body();
        let body_bytes =
            StreamingBody::new(body, server.config.request_body_max_bytes)
                .into_bytes_mut()
                .await?;
        Ok(UntypedBody { content: body_bytes.freeze() })
    }

    fn metadata(
        _content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata {
        untyped_metadata()
    }
}

// StreamingBody: body extractor that provides a streaming representation of the body.

/// An extractor for streaming the contents of the HTTP request body, making the
/// raw bytes available to the consumer.
#[derive(Debug)]
pub struct StreamingBody {
    body: crate::Body,
    cap: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum StreamingBodyError {
    #[error("error streaming request body: {0}")]
    Stream(#[from] Box<dyn std::error::Error + Send + Sync + 'static>),
    #[error("request body exceeded maximum size of {0} bytes")]
    MaxSizeExceeded(usize),
}

impl From<StreamingBodyError> for HttpError {
    fn from(error: StreamingBodyError) -> Self {
        HttpError::for_client_error(
            None,
            error.recommended_status_code(),
            error.to_string(),
        )
    }
}

impl StreamingBodyError {
    /// Returns the recommended status code for this error.
    ///
    /// This can be used when constructing a HTTP response for this error. These
    /// are the status codes used by the `From<StreamingBodyError>` implementation
    /// for [`HttpError`].
    pub fn recommended_status_code(&self) -> http::StatusCode {
        match self {
            // If the max body size was exceeded, return 413 Payload Too Large
            // (nee Content Too Large).
            Self::MaxSizeExceeded(_) => http::StatusCode::PAYLOAD_TOO_LARGE,
            // Everything else gets a generic `400 Bad Request`.
            _ => http::StatusCode::BAD_REQUEST,
        }
    }
}

impl StreamingBody {
    fn new(body: crate::Body, cap: usize) -> Self {
        Self { body, cap }
    }

    /// Not part of the public API. Used only for doctests.
    #[doc(hidden)]
    pub fn __from_bytes(data: Bytes) -> Self {
        let cap = data.len();
        let body = crate::Body::from(data);
        Self { body, cap }
    }

    /// Converts `self` into a stream.
    ///
    /// The `Stream` produces values of type `Result<Bytes, HttpError>`.
    ///
    /// # Errors
    ///
    /// The stream produces an [`HttpError`] if any of the following cases occur:
    ///
    /// * A network error occurred.
    /// * `request_body_max_bytes` was exceeded for this request.
    ///
    /// # Examples
    ///
    /// Buffer a `StreamingBody` in-memory, into a
    /// [`BufList`](https://docs.rs/buf-list/latest/buf_list/struct.BufList.html)
    /// (a segmented list of [`Bytes`] chunks).
    ///
    /// ```
    /// use buf_list::BufList;
    /// use dropshot::{StreamingBody, error::StreamingBodyError};
    /// use futures::prelude::*;
    /// # use std::iter::FromIterator;
    ///
    /// async fn into_buf_list(body: StreamingBody) -> Result<BufList, StreamingBodyError> {
    ///     body.into_stream().try_collect().await
    /// }
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #    let body = StreamingBody::__from_bytes(bytes::Bytes::from("foobar"));
    /// #    assert_eq!(
    /// #        into_buf_list(body).await.unwrap().into_iter().next(),
    /// #        Some(bytes::Bytes::from("foobar")),
    /// #    );
    /// # }
    /// ```
    ///
    /// ---
    ///
    /// Write a `StreamingBody` to an [`AsyncWrite`](tokio::io::AsyncWrite),
    /// for example a [`tokio::fs::File`], without buffering it into memory:
    ///
    /// ```
    /// use dropshot::{HttpError, StreamingBody};
    /// use futures::prelude::*;
    /// use tokio::io::{AsyncWrite, AsyncWriteExt};
    ///
    /// async fn write_all<W: AsyncWrite + Unpin>(
    ///     body: StreamingBody,
    ///     writer: &mut W,
    /// ) -> Result<(), HttpError> {
    ///     let stream = body.into_stream();
    ///     tokio::pin!(stream);
    ///
    ///     while let Some(res) = stream.next().await {
    ///         let mut data = res?;
    ///         writer.write_all_buf(&mut data).await.map_err(|error| {
    ///             HttpError::for_unavail(None, format!("write failed: {error}"))
    ///         })?;
    ///     }
    ///
    ///     Ok(())
    /// }
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #    let body = StreamingBody::__from_bytes(bytes::Bytes::from("foobar"));
    /// #    let mut writer = vec![];
    /// #    write_all(body, &mut writer).await.unwrap();
    /// #    assert_eq!(writer, &b"foobar"[..]);
    /// # }
    /// ```
    pub fn into_stream(
        mut self,
    ) -> impl Stream<Item = Result<Bytes, StreamingBodyError>> + Send {
        async_stream::try_stream! {
            let mut bytes_read: usize = 0;
            while let Some(frame_res) = self.body.frame().await {
                let frame = frame_res?;
                let Ok(buf) = frame.into_data() else { continue }; // skip trailers
                let len = buf.len();

                if bytes_read + len > self.cap {
                    http_dump_body(&mut self.body).await?;
                    // TODO-correctness check status code
                    Err(StreamingBodyError::MaxSizeExceeded(self.cap))?;
                }

                bytes_read += len;
                yield buf;
            }
        }
    }

    /// Converts `self` into a [`BytesMut`], buffering the entire response in
    /// memory. Not public API because most users of this should use
    /// `UntypedBody` instead.
    async fn into_bytes_mut(self) -> Result<BytesMut, StreamingBodyError> {
        self.into_stream()
            .try_fold(BytesMut::new(), |mut out, chunk| {
                out.put(chunk);
                futures::future::ok(out)
            })
            .await
    }
}

#[async_trait]
impl ExclusiveExtractor for StreamingBody {
    async fn from_request<Context: ServerContext>(
        rqctx: &RequestContext<Context>,
        request: hyper::Request<crate::Body>,
    ) -> Result<Self, ExtractorError> {
        let server = &rqctx.server;

        Ok(Self {
            body: request.into_body(),
            cap: server.config.request_body_max_bytes,
        })
    }

    fn metadata(
        _content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata {
        untyped_metadata()
    }
}

fn untyped_metadata() -> ExtractorMetadata {
    ExtractorMetadata {
        parameters: vec![ApiEndpointParameter::new_body(
            ApiEndpointBodyContentType::Bytes,
            true,
            ApiSchemaGenerator::Static {
                schema: Box::new(
                    SchemaObject {
                        instance_type: Some(InstanceType::String.into()),
                        format: Some(String::from("binary")),
                        ..Default::default()
                    }
                    .into(),
                ),
                dependencies: indexmap::IndexMap::default(),
            },
            vec![],
        )],
        extension_mode: ExtensionMode::None,
    }
}
