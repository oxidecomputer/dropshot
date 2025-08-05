// Copyright 2023 Oxide Computer Company

//! Body-related extractor(s)

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
/// `BodyType` from an HTTP request body.  `BodyType` may be any struct of yours
/// that implements [serde::Deserialize] and [schemars::JsonSchema], where
/// primitives and enums have to be wrapped in an outer struct and enums need
/// to be flattened using the `#[serde(flatten)]` attribute.  See this module's
/// documentation formore information.
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

#[async_trait]
impl ExclusiveExtractor for MultipartBody {
    async fn from_request<Context: ServerContext>(
        _rqctx: &RequestContext<Context>,
        request: hyper::Request<crate::Body>,
    ) -> Result<Self, HttpError> {
        let (parts, body) = request.into_parts();
        // Get the content-type header.
        let content_type = parts
            .headers
            .get(http::header::CONTENT_TYPE)
            .ok_or_else(|| {
                HttpError::for_bad_request(
                    None,
                    "missing content-type header".to_string(),
                )
            })?
            .to_str()
            .map_err(|e| {
                HttpError::for_bad_request(
                    None,
                    format!("invalid content type: {}", e),
                )
            })?;
        // The boundary is the string after the "boundary=" part of the
        // content-type header.
        let boundary =
            content_type.split("boundary=").nth(1).ok_or_else(|| {
                HttpError::for_bad_request(
                    None,
                    "missing boundary in content-type header".to_string(),
                )
            })?;
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

/// Given an HTTP request, attempt to read the body, parse it according
/// to the content type, and deserialize it to an instance of `BodyType`.
async fn http_request_load_body<Context: ServerContext, BodyType>(
    rqctx: &RequestContext<Context>,
    request: hyper::Request<crate::Body>,
) -> Result<TypedBody<BodyType>, HttpError>
where
    BodyType: JsonSchema + DeserializeOwned + Send + Sync,
{
    let (parts, body) = request.into_parts();
    let body = StreamingBody::new(body, rqctx.request_body_max_bytes())
        .into_bytes_mut()
        .await?;

    // RFC 7231 §3.1.1.1: media types are case insensitive and may
    // be followed by whitespace and/or a parameter (e.g., charset),
    // which we currently ignore.
    let content_type = parts
        .headers
        .get(http::header::CONTENT_TYPE)
        .map(|hv| {
            hv.to_str().map_err(|e| {
                HttpError::for_bad_request(
                    None,
                    format!("invalid content type: {}", e),
                )
            })
        })
        .unwrap_or(Ok(CONTENT_TYPE_JSON))?;
    let end = content_type.find(';').unwrap_or_else(|| content_type.len());
    let mime_type = content_type[..end].trim_end().to_lowercase();
    let body_content_type =
        ApiEndpointBodyContentType::from_mime_type(&mime_type)
            .map_err(|e| HttpError::for_bad_request(None, e))?;
    let expected_content_type = rqctx.endpoint.body_content_type.clone();

    use ApiEndpointBodyContentType::*;

    let content = match (expected_content_type, body_content_type) {
        (Json, Json) => {
            let jd = &mut serde_json::Deserializer::from_slice(&body);
            serde_path_to_error::deserialize(jd).map_err(|e| {
                HttpError::for_bad_request(
                    None,
                    format!("unable to parse JSON body: {}", e),
                )
            })?
        }
        (UrlEncoded, UrlEncoded) => {
            let ud = serde_urlencoded::Deserializer::new(
                form_urlencoded::parse(&body),
            );
            serde_path_to_error::deserialize(ud).map_err(|e| {
                HttpError::for_bad_request(
                    None,
                    format!("unable to parse URL-encoded body: {}", e),
                )
            })?
        }
        (expected, requested) => {
            return Err(HttpError::for_bad_request(
                None,
                format!(
                    "expected content type \"{}\", got \"{}\"",
                    expected.mime_type(),
                    requested.mime_type()
                ),
            ))
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
    ) -> Result<TypedBody<BodyType>, HttpError> {
        http_request_load_body(rqctx, request).await
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
    ) -> Result<UntypedBody, HttpError> {
        let body = request.into_body();
        let body_bytes =
            StreamingBody::new(body, rqctx.request_body_max_bytes())
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
    /// use dropshot::{HttpError, StreamingBody};
    /// use futures::prelude::*;
    /// # use std::iter::FromIterator;
    ///
    /// async fn into_buf_list(body: StreamingBody) -> Result<BufList, HttpError> {
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
    ) -> impl Stream<Item = Result<Bytes, HttpError>> + Send {
        async_stream::try_stream! {
            let mut bytes_read: usize = 0;
            while let Some(frame_res) = self.body.frame().await {
                let frame = frame_res.map_err(|e| HttpError::for_bad_request(
                    None,
                    format!("error streaming request body: {}", e),
                ))?;
                let Ok(buf) = frame.into_data() else { continue }; // skip trailers
                let len = buf.len();

                if bytes_read + len > self.cap {
                    http_dump_body(&mut self.body).await.map_err(|e| {
                        HttpError::for_bad_request(
                            None,
                            format!("error streaming request body: {}", e),
                        )
                    })?;
                    // TODO-correctness check status code
                    Err(HttpError::for_bad_request(
                        None,
                        format!("request body exceeded maximum size of {} bytes", self.cap),
                    ))?;
                }

                bytes_read += len;
                yield buf;
            }
        }
    }

    /// Converts `self` into a [`BytesMut`], buffering the entire response in
    /// memory. Not public API because most users of this should use
    /// `UntypedBody` instead.
    async fn into_bytes_mut(self) -> Result<BytesMut, HttpError> {
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
    ) -> Result<Self, HttpError> {
        Ok(Self {
            body: request.into_body(),
            cap: rqctx.request_body_max_bytes(),
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
