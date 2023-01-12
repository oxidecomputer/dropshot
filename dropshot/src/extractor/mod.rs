// Copyright 2023 Oxide Computer Company

//! Extractor-related traits
//!
//! See top-level crate documentation for details

use crate::api_description::ApiEndpointParameter;
use crate::api_description::ApiEndpointParameterLocation;
use crate::api_description::ApiSchemaGenerator;
use crate::api_description::{ApiEndpointBodyContentType, ExtensionMode};
use crate::error::HttpError;
use crate::http_util::http_extract_path_params;
use crate::http_util::http_read_body;
use crate::http_util::CONTENT_TYPE_JSON;
use crate::pagination::PAGINATION_PARAM_SENTINEL;
use crate::schema_util::make_subschema_for;
use crate::schema_util::schema2struct;
use crate::schema_util::schema_extensions;
use crate::schema_util::ReferenceVisitor;
use crate::server::ServerContext;
use crate::websocket::WEBSOCKET_PARAM_SENTINEL;
use crate::RequestContext;

use async_trait::async_trait;
use bytes::Bytes;
use hyper::Body;
use hyper::Request;
use schemars::schema::InstanceType;
use schemars::schema::SchemaObject;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use std::fmt::Debug;

mod common;

pub use common::ExclusiveExtractor;
pub use common::ExtractorMetadata;
pub use common::RequestExtractor;
pub use common::SharedExtractor;

// Query: query string extractor

/// `Query<QueryType>` is an extractor used to deserialize an instance of
/// `QueryType` from an HTTP request's query string.  `QueryType` is any
/// structure of yours that implements `serde::Deserialize`.  See this module's
/// documentation for more information.
#[derive(Debug)]
pub struct Query<QueryType: DeserializeOwned + JsonSchema + Send + Sync> {
    inner: QueryType,
}

impl<QueryType: DeserializeOwned + JsonSchema + Send + Sync> Query<QueryType> {
    // TODO drop this in favor of Deref?  + Display and Debug for convenience?
    pub fn into_inner(self) -> QueryType {
        self.inner
    }
}

/// Given an HTTP request, pull out the query string and attempt to deserialize
/// it as an instance of `QueryType`.
fn http_request_load_query<QueryType>(
    request: &Request<Body>,
) -> Result<Query<QueryType>, HttpError>
where
    QueryType: DeserializeOwned + JsonSchema + Send + Sync,
{
    let raw_query_string = request.uri().query().unwrap_or("");
    // TODO-correctness: are query strings defined to be urlencoded in this way?
    match serde_urlencoded::from_str(raw_query_string) {
        Ok(q) => Ok(Query { inner: q }),
        Err(e) => Err(HttpError::for_bad_request(
            None,
            format!("unable to parse query string: {}", e),
        )),
    }
}

// The `SharedExtractor` implementation for Query<QueryType> describes how to
// construct an instance of `Query<QueryType>` from an HTTP request: namely, by
// parsing the query string to an instance of `QueryType`.
// TODO-cleanup We shouldn't have to use the "'static" bound on `QueryType`
// here.  It seems like we ought to be able to use 'async_trait, but that
// doesn't seem to be defined.
#[async_trait]
impl<QueryType> SharedExtractor for Query<QueryType>
where
    QueryType: JsonSchema + DeserializeOwned + Send + Sync + 'static,
{
    async fn from_request<Context: ServerContext>(
        rqctx: &RequestContext<Context>,
    ) -> Result<Query<QueryType>, HttpError> {
        let request = rqctx.request.lock().await;
        http_request_load_query(&request)
    }

    fn metadata(
        _body_content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata {
        get_metadata::<QueryType>(&ApiEndpointParameterLocation::Query)
    }
}

// Path: path parameter string extractor

/// `Path<PathType>` is an extractor used to deserialize an instance of
/// `PathType` from an HTTP request's path parameters.  `PathType` is any
/// structure of yours that implements `serde::Deserialize`.  See this module's
/// documentation for more information.
#[derive(Debug)]
pub struct Path<PathType: JsonSchema + Send + Sync> {
    inner: PathType,
}

impl<PathType: JsonSchema + Send + Sync> Path<PathType> {
    // TODO drop this in favor of Deref?  + Display and Debug for convenience?
    pub fn into_inner(self) -> PathType {
        self.inner
    }
}

// The `SharedExtractor` implementation for Path<PathType> describes how to
// construct an instance of `Path<QueryType>` from an HTTP request: namely, by
// extracting parameters from the query string.
#[async_trait]
impl<PathType> SharedExtractor for Path<PathType>
where
    PathType: DeserializeOwned + JsonSchema + Send + Sync + 'static,
{
    async fn from_request<Context: ServerContext>(
        rqctx: &RequestContext<Context>,
    ) -> Result<Path<PathType>, HttpError> {
        let params: PathType = http_extract_path_params(&rqctx.path_variables)?;
        Ok(Path { inner: params })
    }

    fn metadata(
        _body_content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata {
        get_metadata::<PathType>(&ApiEndpointParameterLocation::Path)
    }
}

/// Convenience function to generate parameter metadata from types implementing
/// `JsonSchema` for use with `Query` and `Path` `Extractors`.
fn get_metadata<ParamType>(
    loc: &ApiEndpointParameterLocation,
) -> ExtractorMetadata
where
    ParamType: JsonSchema,
{
    // Generate the type for `ParamType` then pluck out each member of
    // the structure to encode as an individual parameter.
    let mut generator = schemars::gen::SchemaGenerator::new(
        schemars::gen::SchemaSettings::openapi3(),
    );
    let schema = generator.root_schema_for::<ParamType>().schema.into();

    let extension_mode = match schema_extensions(&schema) {
        Some(extensions) => {
            let paginated = extensions
                .get(&PAGINATION_PARAM_SENTINEL.to_string())
                .is_some();
            let websocket =
                extensions.get(&WEBSOCKET_PARAM_SENTINEL.to_string()).is_some();
            match (paginated, websocket) {
                (false, false) => ExtensionMode::None,
                (false, true) => ExtensionMode::Websocket,
                (true, false) => ExtensionMode::Paginated,
                (true, true) => panic!(
                    "Cannot use websocket and pagination in the same endpoint!"
                ),
            }
        }
        None => ExtensionMode::None,
    };

    // Convert our collection of struct members list of parameters.
    let parameters = schema2struct(&schema, &generator, true)
        .into_iter()
        .map(|struct_member| {
            let mut s = struct_member.schema;
            let mut visitor = ReferenceVisitor::new(&generator);
            schemars::visit::visit_schema(&mut visitor, &mut s);

            ApiEndpointParameter::new_named(
                loc,
                struct_member.name,
                struct_member.description,
                struct_member.required,
                ApiSchemaGenerator::Static {
                    schema: Box::new(s),
                    dependencies: visitor.dependencies(),
                },
                Vec::new(),
            )
        })
        .collect::<Vec<_>>();

    ExtractorMetadata { extension_mode, parameters }
}

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

/// Given an HTTP request, attempt to read the body, parse it according
/// to the content type, and deserialize it to an instance of `BodyType`.
async fn http_request_load_body<Context: ServerContext, BodyType>(
    rqctx: &RequestContext<Context>,
) -> Result<TypedBody<BodyType>, HttpError>
where
    BodyType: JsonSchema + DeserializeOwned + Send + Sync,
{
    let server = &rqctx.server;
    let mut request = rqctx.request.lock().await;
    let body = http_read_body(
        request.body_mut(),
        server.config.request_body_max_bytes,
    )
    .await?;

    // RFC 7231 §3.1.1.1: media types are case insensitive and may
    // be followed by whitespace and/or a parameter (e.g., charset),
    // which we currently ignore.
    let content_type = request
        .headers()
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
    let expected_content_type = rqctx.body_content_type.clone();

    use ApiEndpointBodyContentType::*;
    let content: BodyType = match (expected_content_type, body_content_type) {
        (Json, Json) => serde_json::from_slice(&body).map_err(|e| {
            HttpError::for_bad_request(
                None,
                format!("unable to parse JSON body: {}", e),
            )
        })?,
        (UrlEncoded, UrlEncoded) => serde_urlencoded::from_bytes(&body)
            .map_err(|e| {
                HttpError::for_bad_request(
                    None,
                    format!("unable to parse URL-encoded body: {}", e),
                )
            })?,
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
    ) -> Result<TypedBody<BodyType>, HttpError> {
        http_request_load_body(rqctx).await
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
    ) -> Result<UntypedBody, HttpError> {
        let server = &rqctx.server;
        let mut request = rqctx.request.lock().await;
        let body_bytes = http_read_body(
            request.body_mut(),
            server.config.request_body_max_bytes,
        )
        .await?;
        Ok(UntypedBody { content: body_bytes })
    }

    fn metadata(
        _content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata {
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
}

#[cfg(test)]
mod test {
    use crate::api_description::ExtensionMode;
    use crate::{
        api_description::ApiEndpointParameterMetadata, ApiEndpointParameter,
        ApiEndpointParameterLocation, PaginationParams,
    };
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    use super::get_metadata;
    use super::ExtractorMetadata;

    #[derive(Deserialize, Serialize, JsonSchema)]
    #[allow(dead_code)]
    struct A {
        foo: String,
        bar: u32,
        baz: Option<String>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct B<T> {
        #[serde(flatten)]
        page: T,

        limit: Option<u64>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    #[schemars(untagged)]
    enum C<T> {
        First(T),
        Next { page_token: String },
    }

    fn compare(
        actual: ExtractorMetadata,
        extension_mode: ExtensionMode,
        parameters: Vec<(&str, bool)>,
    ) {
        assert_eq!(actual.extension_mode, extension_mode);

        // This is order-dependent. We might not really care if the order
        // changes, but it will be interesting to understand why if it does.
        actual.parameters.iter().zip(parameters.iter()).for_each(
            |(param, (name, required))| {
                if let ApiEndpointParameter {
                    metadata: ApiEndpointParameterMetadata::Path(aname),
                    required: arequired,
                    ..
                } = param
                {
                    assert_eq!(aname, name);
                    assert_eq!(arequired, required, "mismatched for {}", name);
                } else {
                    panic!();
                }
            },
        );
    }

    #[test]
    fn test_metadata_simple() {
        let params = get_metadata::<A>(&ApiEndpointParameterLocation::Path);
        let expected = vec![("bar", true), ("baz", false), ("foo", true)];

        compare(params, ExtensionMode::None, expected);
    }

    #[test]
    fn test_metadata_flattened() {
        let params = get_metadata::<B<A>>(&ApiEndpointParameterLocation::Path);
        let expected = vec![
            ("bar", true),
            ("baz", false),
            ("foo", true),
            ("limit", false),
        ];

        compare(params, ExtensionMode::None, expected);
    }

    #[test]
    fn test_metadata_flattened_enum() {
        let params =
            get_metadata::<B<C<A>>>(&ApiEndpointParameterLocation::Path);
        let expected = vec![
            ("limit", false),
            ("bar", false),
            ("baz", false),
            ("foo", false),
            ("page_token", false),
        ];

        compare(params, ExtensionMode::None, expected);
    }

    #[test]
    fn test_metadata_pagination() {
        let params = get_metadata::<PaginationParams<A, A>>(
            &ApiEndpointParameterLocation::Path,
        );
        let expected = vec![
            ("bar", false),
            ("baz", false),
            ("foo", false),
            ("limit", false),
            ("page_token", false),
        ];

        compare(params, ExtensionMode::Paginated, expected);
    }
}
