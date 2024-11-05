// Copyright 2023 Oxide Computer Company
//! Describes the endpoints and handler functions in your API

use crate::extractor::RequestExtractor;
use crate::handler::HttpHandlerFunc;
use crate::handler::HttpResponse;
use crate::handler::HttpRouteHandler;
use crate::handler::RouteHandler;
use crate::handler::StubRouteHandler;
use crate::router::route_path_to_segments;
use crate::router::HttpRouter;
use crate::router::PathSegment;
use crate::schema_util::j2oas_schema;
use crate::server::ServerContext;
use crate::type_util::type_is_scalar;
use crate::type_util::type_is_string_enum;
use crate::HttpError;
use crate::HttpErrorResponseBody;
use crate::CONTENT_TYPE_JSON;
use crate::CONTENT_TYPE_MULTIPART_FORM_DATA;
use crate::CONTENT_TYPE_OCTET_STREAM;
use crate::CONTENT_TYPE_URL_ENCODED;

use http::Method;
use http::StatusCode;
use serde::de::Error;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

/// A type used to produce an `ApiDescription` without a concrete implementation
/// of an API trait.
///
/// This type is never constructed, and is used only as a type parameter to
/// [`ApiEndpoint::new_for_types`].
#[derive(Copy, Clone, Debug)]
pub enum StubContext {}

/// ApiEndpoint represents a single API endpoint associated with an
/// ApiDescription. It has a handler, HTTP method (e.g. GET, POST), and a path--
/// provided explicitly--as well as parameters and a description which can be
/// inferred from function parameter types and doc comments (respectively).
#[derive(Debug)]
pub struct ApiEndpoint<Context: ServerContext> {
    pub operation_id: String,
    pub handler: Arc<dyn RouteHandler<Context>>,
    pub method: Method,
    pub path: String,
    pub parameters: Vec<ApiEndpointParameter>,
    pub body_content_type: ApiEndpointBodyContentType,
    pub response: ApiEndpointResponse,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub extension_mode: ExtensionMode,
    pub visible: bool,
    pub deprecated: bool,
    pub error_type: ApiEndpointErrorMetadata,
}

impl<'a, Context: ServerContext> ApiEndpoint<Context> {
    pub fn new<HandlerType, FuncParams, ResponseType>(
        operation_id: String,
        handler: HandlerType,
        method: Method,
        content_type: &'a str,
        path: &'a str,
    ) -> Self
    where
        HandlerType: HttpHandlerFunc<Context, FuncParams, ResponseType>,
        FuncParams: RequestExtractor + 'static,
        ResponseType: HttpResponse + Send + Sync + 'static,
    {
        let body_content_type =
            ApiEndpointBodyContentType::from_mime_type(content_type)
                .expect("unsupported mime type");
        let func_parameters = FuncParams::metadata(body_content_type.clone());
        let response = ResponseType::response_metadata();
        ApiEndpoint {
            operation_id,
            handler: HttpRouteHandler::new(handler),
            method,
            path: path.to_string(),
            parameters: func_parameters.parameters,
            body_content_type,
            response,
            summary: None,
            description: None,
            tags: vec![],
            extension_mode: func_parameters.extension_mode,
            visible: true,
            deprecated: false,
            error_type: ApiEndpointErrorMetadata::for_error::<HandlerType::Error>(
            ),
        }
    }

    pub fn summary<T: ToString>(mut self, description: T) -> Self {
        self.summary.replace(description.to_string());
        self
    }

    pub fn description<T: ToString>(mut self, description: T) -> Self {
        self.description.replace(description.to_string());
        self
    }

    pub fn tag<T: ToString>(mut self, tag: T) -> Self {
        self.tags.push(tag.to_string());
        self
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn deprecated(mut self, deprecated: bool) -> Self {
        self.deprecated = deprecated;
        self
    }
}

impl<'a> ApiEndpoint<StubContext> {
    /// Create a new API endpoint without an actual handler behind it, which
    /// panics if called.
    ///
    /// This is useful for generating OpenAPI documentation without having to
    /// implement the actual handler function. In that capacity, it is used for
    /// trait-based dropshot APIs.
    ///
    /// # Example
    ///
    /// This must be invoked by specifying the request and response types as
    /// type parameters.
    ///
    /// ```rust
    /// use dropshot::{ApiDescription, ApiEndpoint, HttpError, HttpResponseOk, Query, StubContext};
    /// use schemars::JsonSchema;
    /// use serde::Deserialize;
    ///
    /// #[derive(Debug, Deserialize, JsonSchema)]
    /// struct GetValueParams {
    ///     key: String,
    /// }
    ///
    /// let mut api: ApiDescription<StubContext> = ApiDescription::new();
    /// let endpoint = ApiEndpoint::new_for_types::<
    ///     // The request type is always a tuple. Note the 1-tuple syntax.
    ///     (Query<GetValueParams>,),
    ///     // The response type is always Result<T, HttpError> where T implements
    ///     // HttpResponse.
    ///     Result<HttpResponseOk<String>, HttpError>,
    /// >(
    ///     "get_value".to_string(),
    ///     http::Method::GET,
    ///     "application/json",
    ///     "/value",
    /// );
    /// api.register(endpoint).unwrap();
    /// ```
    pub fn new_for_types<FuncParams, ResultType>(
        operation_id: String,
        method: Method,
        content_type: &'a str,
        path: &'a str,
    ) -> Self
    where
        FuncParams: RequestExtractor + 'static,
        ResultType: HttpResultType,
    {
        let body_content_type =
            ApiEndpointBodyContentType::from_mime_type(content_type)
                .expect("unsupported mime type");
        let func_parameters = FuncParams::metadata(body_content_type.clone());
        let response = ResultType::Response::response_metadata();
        let handler = StubRouteHandler::new_with_name(&operation_id);
        ApiEndpoint {
            operation_id,
            handler,
            method,
            path: path.to_string(),
            parameters: func_parameters.parameters,
            body_content_type,
            response,
            summary: None,
            description: None,
            tags: vec![],
            extension_mode: func_parameters.extension_mode,
            visible: true,
            deprecated: false,
            error_type: ApiEndpointErrorMetadata::for_error::<ResultType::Error>(
            ),
        }
    }
}

pub trait HttpResultType {
    type Response: HttpResponse + Send + Sync + 'static;
    type Error: crate::error::IntoErrorResponse;
}

impl<T, E> HttpResultType for Result<T, E>
where
    T: HttpResponse + Send + Sync + 'static,
    E: crate::error::IntoErrorResponse,
{
    type Response = T;
    type Error = E;
}

/// ApiEndpointParameter represents the discrete path and query parameters for a
/// given API endpoint. These are typically derived from the members of stucts
/// used as parameters to handler functions.
#[derive(Debug)]
pub struct ApiEndpointParameter {
    pub metadata: ApiEndpointParameterMetadata,
    pub description: Option<String>,
    pub required: bool,
    pub schema: ApiSchemaGenerator,
    pub examples: Vec<String>,
}

impl ApiEndpointParameter {
    pub fn new_named(
        loc: &ApiEndpointParameterLocation,
        name: String,
        description: Option<String>,
        required: bool,
        schema: ApiSchemaGenerator,
        examples: Vec<String>,
    ) -> Self {
        Self {
            metadata: match loc {
                ApiEndpointParameterLocation::Path => {
                    ApiEndpointParameterMetadata::Path(name)
                }
                ApiEndpointParameterLocation::Query => {
                    ApiEndpointParameterMetadata::Query(name)
                }
            },
            description,
            required,
            schema,
            examples,
        }
    }

    pub fn new_body(
        content_type: ApiEndpointBodyContentType,
        required: bool,
        schema: ApiSchemaGenerator,
        examples: Vec<String>,
    ) -> Self {
        Self {
            metadata: ApiEndpointParameterMetadata::Body(content_type),
            required,
            schema,
            examples,
            description: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ApiEndpointParameterLocation {
    Path,
    Query,
}

#[derive(Debug, Clone)]
pub enum ApiEndpointParameterMetadata {
    Path(String),
    Query(String),
    Body(ApiEndpointBodyContentType),
}

#[derive(Debug, Clone)]
pub enum ApiEndpointBodyContentType {
    /// application/octet-stream
    Bytes,
    /// application/json
    Json,
    /// application/x-www-form-urlencoded
    UrlEncoded,
    /// multipart/form-data
    MultipartFormData,
}

impl Default for ApiEndpointBodyContentType {
    fn default() -> Self {
        Self::Json
    }
}

impl ApiEndpointBodyContentType {
    pub fn mime_type(&self) -> &str {
        match self {
            Self::Bytes => CONTENT_TYPE_OCTET_STREAM,
            Self::Json => CONTENT_TYPE_JSON,
            Self::UrlEncoded => CONTENT_TYPE_URL_ENCODED,
            Self::MultipartFormData => CONTENT_TYPE_MULTIPART_FORM_DATA,
        }
    }

    pub fn from_mime_type(mime_type: &str) -> Result<Self, String> {
        match mime_type {
            CONTENT_TYPE_OCTET_STREAM => Ok(Self::Bytes),
            CONTENT_TYPE_JSON => Ok(Self::Json),
            CONTENT_TYPE_URL_ENCODED => Ok(Self::UrlEncoded),
            CONTENT_TYPE_MULTIPART_FORM_DATA => Ok(Self::MultipartFormData),
            _ => Err(mime_type.to_string()),
        }
    }
}

#[derive(Debug)]
pub struct ApiEndpointHeader {
    pub name: String,
    pub description: Option<String>,
    pub schema: ApiSchemaGenerator,
    pub required: bool,
}

/// Metadata for an API endpoint response: type information and status code.
#[derive(Debug, Default)]
pub struct ApiEndpointResponse {
    pub schema: Option<ApiSchemaGenerator>,
    pub headers: Vec<ApiEndpointHeader>,
    pub success: Option<StatusCode>,
    pub description: Option<String>,
}

#[derive(Debug)]
pub struct ApiEndpointErrorMetadata {
    pub name: String,
    pub schema:
        fn(&mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema,
}

/// Wrapper for both dynamically generated and pre-generated schemas.
pub enum ApiSchemaGenerator {
    Gen {
        name: fn() -> String,
        schema:
            fn(&mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema,
    },
    Static {
        schema: Box<schemars::schema::Schema>,
        dependencies: indexmap::IndexMap<String, schemars::schema::Schema>,
    },
}

impl std::fmt::Debug for ApiSchemaGenerator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiSchemaGenerator::Gen { .. } => f.write_str("[schema generator]"),
            ApiSchemaGenerator::Static { schema, .. } => {
                f.write_str(format!("{:?}", schema).as_str())
            }
        }
    }
}

/// An ApiDescription represents the endpoints and handler functions in your API.
/// Other metadata could also be provided here.  This object can be used to
/// generate an OpenAPI spec or to run an HTTP server implementing the API.
pub struct ApiDescription<Context: ServerContext> {
    /// In practice, all the information we need is encoded in the router.
    router: HttpRouter<Context>,
    tag_config: TagConfig,
}

impl<Context: ServerContext> ApiDescription<Context> {
    pub fn new() -> Self {
        ApiDescription {
            router: HttpRouter::new(),
            tag_config: TagConfig::default(),
        }
    }

    pub fn tag_config(mut self, tag_config: TagConfig) -> Self {
        self.tag_config = tag_config;
        self
    }

    // Not (yet) part of the public API, only used for tests. If/when this is
    // made public, we should consider changing the setter above to be named
    // `with_tag_config`, and this to `tag_config`.
    #[doc(hidden)]
    pub fn get_tag_config(&self) -> &TagConfig {
        &self.tag_config
    }

    /// Register a new API endpoint.
    pub fn register<T>(
        &mut self,
        endpoint: T,
    ) -> Result<(), ApiDescriptionRegisterError>
    where
        T: Into<ApiEndpoint<Context>>,
    {
        let e = endpoint.into();
        let operation_id = e.operation_id.clone();

        // manually outline, see https://matklad.github.io/2021/09/04/fast-rust-builds.html#Keeping-Instantiations-In-Check
        fn _register<C: ServerContext>(
            s: &mut ApiDescription<C>,
            e: ApiEndpoint<C>,
        ) -> Result<(), String> {
            s.validate_tags(&e)?;
            s.validate_path_parameters(&e)?;
            s.validate_named_parameters(&e)?;

            s.router.insert(e);

            Ok(())
        }

        _register(self, e).map_err(|error| ApiDescriptionRegisterError {
            operation_id,
            message: error,
        })?;

        Ok(())
    }

    /// Validate that the tags conform to the tags policy.
    fn validate_tags(&self, e: &ApiEndpoint<Context>) -> Result<(), String> {
        // Don't care about endpoints that don't appear in the OpenAPI
        if !e.visible {
            return Ok(());
        }

        match (&self.tag_config.policy, e.tags.len()) {
            (EndpointTagPolicy::AtLeastOne, 0) => {
                return Err("At least one tag is required".to_string())
            }
            (EndpointTagPolicy::ExactlyOne, n) if n != 1 => {
                return Err("Exactly one tag is required".to_string())
            }
            _ => (),
        }

        if !self.tag_config.allow_other_tags {
            for tag in &e.tags {
                if !self.tag_config.tags.contains_key(tag) {
                    return Err(format!("Invalid tag: {}", tag));
                }
            }
        }

        Ok(())
    }

    /// Validate that the parameters specified in the path match the parameters
    /// specified by the path parameter arguments to the handler function.
    fn validate_path_parameters(
        &self,
        e: &ApiEndpoint<Context>,
    ) -> Result<(), String> {
        // Gather up the path parameters and the path variable components, and
        // make sure they're identical.
        let path = route_path_to_segments(&e.path)
            .iter()
            .filter_map(|segment| match PathSegment::from(segment) {
                PathSegment::VarnameSegment(v) => Some(v),
                PathSegment::VarnameWildcard(v) => Some(v),
                PathSegment::Literal(_) => None,
            })
            .collect::<HashSet<_>>();
        let vars = e
            .parameters
            .iter()
            .filter_map(|p| match &p.metadata {
                ApiEndpointParameterMetadata::Path(name) => Some(name.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();

        if path != vars {
            let mut p = path.difference(&vars).collect::<Vec<_>>();
            let mut v = vars.difference(&path).collect::<Vec<_>>();
            p.sort();
            v.sort();

            let pp =
                p.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(",");
            let vv =
                v.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(",");

            match (p.is_empty(), v.is_empty()) {
                (false, true) => Err(format!(
                    "{} ({})",
                    "path parameters are not consumed", pp,
                )),
                (true, false) => Err(format!(
                    "{} ({})",
                    "specified parameters do not appear in the path", vv,
                )),
                _ => Err(format!(
                    "{} ({}) and {} ({})",
                    "path parameters are not consumed",
                    pp,
                    "specified parameters do not appear in the path",
                    vv,
                )),
            }?;
        }

        Ok(())
    }

    /// Validate that named parameters have appropriate types and there are no
    /// duplicates. Parameters must have scalar types except in the case of the
    /// received for a wildcard path which must be an array of String.
    fn validate_named_parameters(
        &self,
        e: &ApiEndpoint<Context>,
    ) -> Result<(), String> {
        enum SegmentOrWildcard {
            Segment,
            Wildcard,
        }
        let path_segments = route_path_to_segments(&e.path)
            .iter()
            .filter_map(|segment| {
                let seg = PathSegment::from(segment);
                match seg {
                    PathSegment::VarnameSegment(v) => {
                        Some((v, SegmentOrWildcard::Segment))
                    }
                    PathSegment::VarnameWildcard(v) => {
                        Some((v, SegmentOrWildcard::Wildcard))
                    }
                    PathSegment::Literal(_) => None,
                }
            })
            .collect::<BTreeMap<_, _>>();

        for param in &e.parameters {
            // Skip anything that's not a path or query parameter (i.e. body)
            match &param.metadata {
                ApiEndpointParameterMetadata::Path(_)
                | ApiEndpointParameterMetadata::Query(_) => (),
                _ => continue,
            }
            // Only body parameters should have unresolved schemas
            let (schema, dependencies) = match &param.schema {
                ApiSchemaGenerator::Static { schema, dependencies } => {
                    (schema, dependencies)
                }
                _ => unreachable!(),
            };

            match &param.metadata {
                ApiEndpointParameterMetadata::Path(ref name) => {
                    match path_segments.get(name) {
                        Some(SegmentOrWildcard::Segment) => {
                            type_is_scalar(
                                &e.operation_id,
                                name,
                                schema,
                                dependencies,
                            )?;
                        }
                        Some(SegmentOrWildcard::Wildcard) => {
                            type_is_string_enum(
                                &e.operation_id,
                                name,
                                schema,
                                dependencies,
                            )?;
                        }
                        None => {
                            panic!("all path variables should be accounted for")
                        }
                    }
                }
                ApiEndpointParameterMetadata::Query(ref name) => {
                    if path_segments.contains_key(name) {
                        return Err(format!(
                            "the parameter '{}' is specified for both query \
                             and path parameters",
                            name
                        ));
                    }
                    type_is_scalar(
                        &e.operation_id,
                        name,
                        schema,
                        dependencies,
                    )?;
                }
                _ => (),
            }
        }

        Ok(())
    }

    /// Build the OpenAPI definition describing this API.  Returns an
    /// [`OpenApiDefinition`] which can be used to specify the contents of the
    /// definition and select an output format.
    ///
    /// The arguments to this function will be used for the mandatory `title` and
    /// `version` properties that the `Info` object in an OpenAPI definition must
    /// contain.
    pub fn openapi<S1, S2>(
        &self,
        title: S1,
        version: S2,
    ) -> OpenApiDefinition<Context>
    where
        S1: AsRef<str>,
        S2: AsRef<str>,
    {
        OpenApiDefinition::new(self, title.as_ref(), version.as_ref())
    }

    /// Internal routine for constructing the OpenAPI definition describing this
    /// API in its JSON form.
    fn gen_openapi(&self, info: openapiv3::Info) -> openapiv3::OpenAPI {
        let mut openapi = openapiv3::OpenAPI::default();

        openapi.openapi = "3.0.3".to_string();
        openapi.info = info;

        // Gather up the ad hoc tags from endpoints
        let endpoint_tags = (&self.router)
            .into_iter()
            .flat_map(|(_, _, endpoint)| {
                endpoint
                    .tags
                    .iter()
                    .filter(|tag| !self.tag_config.tags.contains_key(*tag))
            })
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .map(|tag| openapiv3::Tag { name: tag, ..Default::default() });

        // Bundle those with the explicit tags provided by the consumer
        openapi.tags = self
            .tag_config
            .tags
            .iter()
            .map(|(name, details)| openapiv3::Tag {
                name: name.clone(),
                description: details.description.clone(),
                external_docs: details.external_docs.as_ref().map(|e| {
                    openapiv3::ExternalDocumentation {
                        description: e.description.clone(),
                        url: e.url.clone(),
                        ..Default::default()
                    }
                }),
                ..Default::default()
            })
            .chain(endpoint_tags)
            .collect();

        // Sort the tags for stability
        openapi.tags.sort_by(|a, b| a.name.cmp(&b.name));

        let settings = schemars::gen::SchemaSettings::openapi3();
        let mut generator = schemars::gen::SchemaGenerator::new(settings);
        let mut definitions =
            indexmap::IndexMap::<String, schemars::schema::Schema>::new();
        let mut errors =
            indexmap::IndexMap::<String, schemars::schema::Schema>::new();

        for (path, method, endpoint) in &self.router {
            if !endpoint.visible {
                continue;
            }
            let path = openapi.paths.paths.entry(path).or_insert(
                openapiv3::ReferenceOr::Item(openapiv3::PathItem::default()),
            );

            let pathitem = match path {
                openapiv3::ReferenceOr::Item(ref mut item) => item,
                _ => panic!("reference not expected"),
            };

            let method_ref = match &method[..] {
                "GET" => &mut pathitem.get,
                "PUT" => &mut pathitem.put,
                "POST" => &mut pathitem.post,
                "DELETE" => &mut pathitem.delete,
                "OPTIONS" => &mut pathitem.options,
                "HEAD" => &mut pathitem.head,
                "PATCH" => &mut pathitem.patch,
                "TRACE" => &mut pathitem.trace,
                other => panic!("unexpected method `{}`", other),
            };
            let mut operation = openapiv3::Operation::default();
            operation.operation_id = Some(endpoint.operation_id.clone());
            operation.summary.clone_from(&endpoint.summary);
            operation.description.clone_from(&endpoint.description);
            operation.tags.clone_from(&endpoint.tags);
            operation.deprecated = endpoint.deprecated;

            operation.parameters = endpoint
                .parameters
                .iter()
                .filter_map(|param| {
                    let (name, location) = match &param.metadata {
                        ApiEndpointParameterMetadata::Body(_) => return None,
                        ApiEndpointParameterMetadata::Path(name) => {
                            (name, ApiEndpointParameterLocation::Path)
                        }
                        ApiEndpointParameterMetadata::Query(name) => {
                            (name, ApiEndpointParameterLocation::Query)
                        }
                    };

                    let schema = match &param.schema {
                        ApiSchemaGenerator::Static { schema, dependencies } => {
                            definitions.extend(dependencies.clone());
                            j2oas_schema(None, schema)
                        }
                        _ => {
                            unimplemented!("this may happen for complex types")
                        }
                    };

                    let parameter_data = openapiv3::ParameterData {
                        name: name.clone(),
                        description: param.description.clone(),
                        required: param.required,
                        deprecated: None,
                        format: openapiv3::ParameterSchemaOrContent::Schema(
                            schema,
                        ),
                        example: None,
                        examples: indexmap::IndexMap::new(),
                        extensions: indexmap::IndexMap::new(),
                        explode: None,
                    };
                    match location {
                        ApiEndpointParameterLocation::Query => {
                            Some(openapiv3::ReferenceOr::Item(
                                openapiv3::Parameter::Query {
                                    parameter_data,
                                    allow_reserved: false,
                                    style: openapiv3::QueryStyle::Form,
                                    allow_empty_value: None,
                                },
                            ))
                        }
                        ApiEndpointParameterLocation::Path => {
                            Some(openapiv3::ReferenceOr::Item(
                                openapiv3::Parameter::Path {
                                    parameter_data,
                                    style: openapiv3::PathStyle::Simple,
                                },
                            ))
                        }
                    }
                })
                .collect::<Vec<_>>();

            operation.request_body = endpoint
                .parameters
                .iter()
                .filter_map(|param| {
                    let mime_type = match &param.metadata {
                        ApiEndpointParameterMetadata::Body(ct) => {
                            ct.mime_type()
                        }
                        _ => return None,
                    };

                    let (name, js) = match &param.schema {
                        ApiSchemaGenerator::Gen { name, schema } => {
                            (Some(name()), schema(&mut generator))
                        }
                        ApiSchemaGenerator::Static { schema, dependencies } => {
                            definitions.extend(dependencies.clone());
                            (None, schema.as_ref().clone())
                        }
                    };
                    let schema = j2oas_schema(name.as_ref(), &js);

                    let mut content = indexmap::IndexMap::new();
                    content.insert(
                        mime_type.to_string(),
                        openapiv3::MediaType {
                            schema: Some(schema),
                            ..Default::default()
                        },
                    );

                    Some(openapiv3::ReferenceOr::Item(openapiv3::RequestBody {
                        content,
                        required: true,
                        ..Default::default()
                    }))
                })
                .next();

            match &endpoint.extension_mode {
                ExtensionMode::None => {}
                ExtensionMode::Paginated(first_page_schema) => {
                    operation.extensions.insert(
                        crate::pagination::PAGINATION_EXTENSION.to_string(),
                        first_page_schema.clone(),
                    );
                }
                ExtensionMode::Websocket => {
                    operation.extensions.insert(
                        crate::websocket::WEBSOCKET_EXTENSION.to_string(),
                        serde_json::json!({}),
                    );
                }
            }

            let response = if let Some(schema) = &endpoint.response.schema {
                let (name, js) = match schema {
                    ApiSchemaGenerator::Gen { name, schema } => {
                        (Some(name()), schema(&mut generator))
                    }
                    ApiSchemaGenerator::Static { schema, dependencies } => {
                        definitions.extend(dependencies.clone());
                        (None, schema.as_ref().clone())
                    }
                };
                let mut content = indexmap::IndexMap::new();
                if !is_empty(&js) {
                    content.insert(
                        CONTENT_TYPE_JSON.to_string(),
                        openapiv3::MediaType {
                            schema: Some(j2oas_schema(name.as_ref(), &js)),
                            ..Default::default()
                        },
                    );
                }

                let headers = endpoint
                    .response
                    .headers
                    .iter()
                    .map(|header| {
                        let schema = match &header.schema {
                            ApiSchemaGenerator::Static {
                                schema,
                                dependencies,
                            } => {
                                definitions.extend(dependencies.clone());
                                j2oas_schema(None, schema)
                            }
                            _ => {
                                unimplemented!(
                                    "this may happen for complex types"
                                )
                            }
                        };

                        (
                            header.name.clone(),
                            openapiv3::ReferenceOr::Item(openapiv3::Header {
                                description: header.description.clone(),
                                style: openapiv3::HeaderStyle::Simple,
                                required: header.required,
                                deprecated: None,
                                format:
                                    openapiv3::ParameterSchemaOrContent::Schema(
                                        schema,
                                    ),
                                example: None,
                                examples: indexmap::IndexMap::new(),
                                extensions: indexmap::IndexMap::new(),
                            }),
                        )
                    })
                    .collect();

                let response = openapiv3::Response {
                    description: if let Some(description) =
                        &endpoint.response.description
                    {
                        description.clone()
                    } else {
                        // TODO: perhaps we should require even free-form
                        // responses to have a description since it's required
                        // by OpenAPI.
                        "".to_string()
                    },
                    content,
                    headers,
                    ..Default::default()
                };
                response
            } else {
                // If no schema was specified, the response is hand-rolled. In
                // this case we'll fall back to the default response type which
                // we assume to be inclusive of errors. The media type and
                // and schema will similarly be maximally permissive.
                let mut content = indexmap::IndexMap::new();
                content.insert(
                    "*/*".to_string(),
                    openapiv3::MediaType {
                        schema: Some(openapiv3::ReferenceOr::Item(
                            openapiv3::Schema {
                                schema_data: openapiv3::SchemaData::default(),
                                schema_kind: openapiv3::SchemaKind::Any(
                                    openapiv3::AnySchema::default(),
                                ),
                            },
                        )),
                        ..Default::default()
                    },
                );
                openapiv3::Response {
                    // TODO: perhaps we should require even free-form
                    // responses to have a description since it's required
                    // by OpenAPI.
                    description: "".to_string(),
                    content,
                    ..Default::default()
                }
            };

            if let Some(code) = &endpoint.response.success {
                operation.responses.responses.insert(
                    openapiv3::StatusCode::Code(code.as_u16()),
                    openapiv3::ReferenceOr::Item(response),
                );
                let ApiEndpointErrorMetadata { ref name, ref schema } =
                    endpoint.error_type;
                let err_ref = openapiv3::ReferenceOr::Reference {
                    reference: format!("#/components/responses/{name}"),
                };
                errors
                    .entry(endpoint.error_type.name.clone())
                    .or_insert_with(|| schema(&mut generator));

                operation
                    .responses
                    .responses
                    .insert(openapiv3::StatusCode::Range(4), err_ref.clone());
                operation
                    .responses
                    .responses
                    .insert(openapiv3::StatusCode::Range(5), err_ref);
            } else {
                operation.responses.default =
                    Some(openapiv3::ReferenceOr::Item(response))
            }

            // Drop in the operation.
            method_ref.replace(operation);
        }

        let components = &mut openapi
            .components
            .get_or_insert_with(openapiv3::Components::default);

        let responses = &mut components.responses;
        for (err_name, schema) in errors {
            let mut content = indexmap::IndexMap::new();
            content.insert(
                CONTENT_TYPE_JSON.to_string(),
                openapiv3::MediaType {
                    schema: Some(j2oas_schema(Some(&err_name), &schema)),
                    ..Default::default()
                },
            );

            responses.insert(
                err_name.clone(),
                openapiv3::ReferenceOr::Item(openapiv3::Response {
                    description: err_name.clone(),
                    content,
                    ..Default::default()
                }),
            );
        }

        // Add the schemas for which we generated references.
        let schemas = &mut components.schemas;

        let root_schema = generator.into_root_schema_for::<()>();
        root_schema.definitions.iter().for_each(|(key, schema)| {
            schemas.insert(key.clone(), j2oas_schema(None, schema));
        });

        definitions.into_iter().for_each(|(key, schema)| {
            if !schemas.contains_key(&key) {
                schemas.insert(key, j2oas_schema(None, &schema));
            }
        });

        openapi
    }

    // TODO-cleanup is there a way to make this available only within this
    // crate?  Once we do that, we don't need to consume the ApiDescription to
    // do this.
    pub fn into_router(self) -> HttpRouter<Context> {
        self.router
    }
}

/// A collection of errors that occurred while building an `ApiDescription`.
///
/// Returned by the `api_description` and `stub_api_description` functions
/// generated by the [`api_description`](macro@crate::api_description) macro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiDescriptionBuildErrors {
    errors: Vec<ApiDescriptionRegisterError>,
}

impl ApiDescriptionBuildErrors {
    /// Create a new `ApiDescriptionBuildErrors` with the given errors.
    pub fn new(errors: Vec<ApiDescriptionRegisterError>) -> Self {
        Self { errors }
    }

    /// Return a list of the errors that occurred.
    pub fn errors(&self) -> &[ApiDescriptionRegisterError] {
        &self.errors
    }
}

impl fmt::Display for ApiDescriptionBuildErrors {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "failed to register endpoints: \n")?;
        for error in &self.errors {
            write!(
                f,
                "  - registering '{}' failed: {}\n",
                error.operation_id, error.message
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for ApiDescriptionBuildErrors {}

/// An error that occurred while registering an individual API endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiDescriptionRegisterError {
    operation_id: String,
    message: String,
}

impl ApiDescriptionRegisterError {
    /// Return the name of the endpoint that failed to register.
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// Return a message describing what occurred while registering the endpoint.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ApiDescriptionRegisterError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "failed to register endpoint '{}': {}",
            self.operation_id, self.message
        )
    }
}

impl std::error::Error for ApiDescriptionRegisterError {}

/// Returns true iff the schema represents the void schema that matches no data.
fn is_empty(schema: &schemars::schema::Schema) -> bool {
    if let schemars::schema::Schema::Bool(false) = schema {
        return true;
    }
    if let schemars::schema::Schema::Object(schemars::schema::SchemaObject {
        metadata: _,
        instance_type: None,
        format: None,
        enum_values: None,
        const_value: None,
        subschemas: Some(subschemas),
        number: None,
        string: None,
        array: None,
        object: None,
        reference: None,
        extensions: _,
    }) = schema
    {
        if let schemars::schema::SubschemaValidation {
            all_of: None,
            any_of: None,
            one_of: None,
            not: Some(not),
            if_schema: None,
            then_schema: None,
            else_schema: None,
        } = subschemas.as_ref()
        {
            match not.as_ref() {
                schemars::schema::Schema::Bool(true) => return true,
                schemars::schema::Schema::Object(
                    schemars::schema::SchemaObject {
                        metadata: _,
                        instance_type: None,
                        format: None,
                        enum_values: None,
                        const_value: None,
                        subschemas: None,
                        number: None,
                        string: None,
                        array: None,
                        object: None,
                        reference: None,
                        extensions: _,
                    },
                ) => return true,
                _ => {}
            }
        }
    }

    false
}

/// This object is used to specify configuration for building an OpenAPI
/// definition document.  It is constructed using [`ApiDescription::openapi()`].
/// Additional optional properties may be added and then the OpenAPI definition
/// document may be generated via [`write()`](`OpenApiDefinition::write`) or
/// [`json()`](`OpenApiDefinition::json`).
pub struct OpenApiDefinition<'a, Context: ServerContext> {
    api: &'a ApiDescription<Context>,
    info: openapiv3::Info,
}

impl<'a, Context: ServerContext> OpenApiDefinition<'a, Context> {
    fn new(
        api: &'a ApiDescription<Context>,
        title: &str,
        version: &str,
    ) -> OpenApiDefinition<'a, Context> {
        let info = openapiv3::Info {
            title: title.to_string(),
            version: version.to_string(),
            ..Default::default()
        };
        OpenApiDefinition { api, info }
    }

    /// Provide a short description of the API.  CommonMark syntax may be
    /// used for rich text representation.
    ///
    /// This routine will set the `description` field of the `Info` object in the
    /// OpenAPI definition.
    pub fn description<S: AsRef<str>>(&mut self, description: S) -> &mut Self {
        self.info.description = Some(description.as_ref().to_string());
        self
    }

    /// Include a Terms of Service URL for the API.  Must be in the format of a
    /// URL.
    ///
    /// This routine will set the `termsOfService` field of the `Info` object in
    /// the OpenAPI definition.
    pub fn terms_of_service<S: AsRef<str>>(&mut self, url: S) -> &mut Self {
        self.info.terms_of_service = Some(url.as_ref().to_string());
        self
    }

    fn contact_mut(&mut self) -> &mut openapiv3::Contact {
        if self.info.contact.is_none() {
            self.info.contact = Some(openapiv3::Contact::default());
        }
        self.info.contact.as_mut().unwrap()
    }

    /// Set the identifying name of the contact person or organisation
    /// responsible for the API.
    ///
    /// This routine will set the `name` property of the `Contact` object within
    /// the `Info` object in the OpenAPI definition.
    pub fn contact_name<S: AsRef<str>>(&mut self, name: S) -> &mut Self {
        self.contact_mut().name = Some(name.as_ref().to_string());
        self
    }

    /// Set a contact URL for the API.  Must be in the format of a URL.
    ///
    /// This routine will set the `url` property of the `Contact` object within
    /// the `Info` object in the OpenAPI definition.
    pub fn contact_url<S: AsRef<str>>(&mut self, url: S) -> &mut Self {
        self.contact_mut().url = Some(url.as_ref().to_string());
        self
    }

    /// Set the email address of the contact person or organisation responsible
    /// for the API.  Must be in the format of an email address.
    ///
    /// This routine will set the `email` property of the `Contact` object within
    /// the `Info` object in the OpenAPI definition.
    pub fn contact_email<S: AsRef<str>>(&mut self, email: S) -> &mut Self {
        self.contact_mut().email = Some(email.as_ref().to_string());
        self
    }

    fn license_mut(&mut self, name: &str) -> &mut openapiv3::License {
        if self.info.license.is_none() {
            self.info.license = Some(openapiv3::License {
                name: name.to_string(),
                ..Default::default()
            })
        }
        self.info.license.as_mut().unwrap()
    }

    /// Provide the name of the licence used for the API, and a URL (must be in
    /// URL format) displaying the licence text.
    ///
    /// This routine will set the `name` and optional `url` properties of the
    /// `License` object within the `Info` object in the OpenAPI definition.
    pub fn license<S1, S2>(&mut self, name: S1, url: S2) -> &mut Self
    where
        S1: AsRef<str>,
        S2: AsRef<str>,
    {
        self.license_mut(name.as_ref()).url = Some(url.as_ref().to_string());
        self
    }

    /// Provide the name of the licence used for the API.
    ///
    /// This routine will set the `name` property of the License object within
    /// the `Info` object in the OpenAPI definition.
    pub fn license_name<S: AsRef<str>>(&mut self, name: S) -> &mut Self {
        self.license_mut(name.as_ref());
        self
    }

    /// Build a JSON object containing the OpenAPI definition for this API.
    pub fn json(&self) -> serde_json::Result<serde_json::Value> {
        serde_json::to_value(&self.api.gen_openapi(self.info.clone()))
    }

    /// Build a JSON object containing the OpenAPI definition for this API and
    /// write it to the provided stream.
    pub fn write(
        &self,
        out: &mut dyn std::io::Write,
    ) -> serde_json::Result<()> {
        serde_json::to_writer_pretty(
            &mut *out,
            &self.api.gen_openapi(self.info.clone()),
        )?;
        writeln!(out).map_err(serde_json::Error::custom)?;
        Ok(())
    }
}

impl ApiEndpointErrorMetadata {
    pub(crate) fn for_error<E: crate::error::IntoErrorResponse>() -> Self {
        ApiEndpointErrorMetadata {
            name: <E>::schema_name(),
            schema: crate::schema_util::make_subschema_for::<E>,
        }
    }
}

/// Configuration used describe OpenAPI tags and to validate per-endpoint tags.
/// Consumers may use this ensure that--for example--endpoints pick a tag from a
/// known set, or that each endpoint has at least one tag.
#[derive(Debug, Serialize, Deserialize)]
pub struct TagConfig {
    /// Are endpoints allowed to use tags not specified in this config?
    pub allow_other_tags: bool,

    // The aliases are for backwards compatibility with previous versions of
    // Dropshot.
    #[serde(alias = "endpoint_tag_policy")]
    pub policy: EndpointTagPolicy,
    #[serde(alias = "tag_definitions")]
    pub tags: HashMap<String, TagDetails>,
}

impl Default for TagConfig {
    fn default() -> Self {
        Self {
            allow_other_tags: true,
            policy: EndpointTagPolicy::Any,
            tags: HashMap::new(),
        }
    }
}

/// Endpoint tagging policy
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum EndpointTagPolicy {
    /// Any number of tags is permitted
    Any,
    /// At least one tag is required and more are allowed
    AtLeastOne,
    /// There must be exactly one tag
    ExactlyOne,
}

/// Details for a named tag
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TagDetails {
    pub description: Option<String>,
    pub external_docs: Option<TagExternalDocs>,
}

/// External docs description
#[derive(Debug, Serialize, Deserialize)]
pub struct TagExternalDocs {
    pub description: Option<String>,
    pub url: String,
}

/// Dropshot/Progenitor features used by endpoints which are not a part of the base OpenAPI spec.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ExtensionMode {
    #[default]
    None,
    Paginated(serde_json::Value),
    Websocket,
}

#[cfg(test)]
mod test {
    use crate::endpoint;
    use crate::error::HttpError;
    use crate::handler::RequestContext;
    use crate::ApiDescription;
    use crate::ApiEndpoint;
    use crate::Body;
    use crate::EndpointTagPolicy;
    use crate::Path;
    use crate::Query;
    use crate::TagConfig;
    use crate::TagDetails;
    use crate::CONTENT_TYPE_JSON;
    use http::Method;
    use hyper::Response;
    use openapiv3::OpenAPI;
    use schemars::JsonSchema;
    use serde::Deserialize;
    use std::collections::HashSet;
    use std::str::from_utf8;

    use crate as dropshot; // for "endpoint" macro

    #[derive(Deserialize, JsonSchema)]
    #[allow(dead_code)]
    struct TestPath {
        a: String,
        b: String,
    }

    async fn test_badpath_handler(
        _: RequestContext<()>,
        _: Path<TestPath>,
    ) -> Result<Response<Body>, HttpError> {
        panic!("test handler is not supposed to run");
    }

    #[test]
    fn test_badpath1() {
        let mut api = ApiDescription::new();
        let ret = api.register(ApiEndpoint::new(
            "test_badpath_handler".to_string(),
            test_badpath_handler,
            Method::GET,
            CONTENT_TYPE_JSON,
            "/",
        ));
        let error = ret.unwrap_err();
        assert_eq!(
            error.message(),
            "specified parameters do not appear in the path (a,b)",
        );
    }

    #[test]
    fn test_badpath2() {
        let mut api = ApiDescription::new();
        let ret = api.register(ApiEndpoint::new(
            "test_badpath_handler".to_string(),
            test_badpath_handler,
            Method::GET,
            CONTENT_TYPE_JSON,
            "/{a}/{aa}/{b}/{bb}",
        ));
        let error = ret.unwrap_err();
        assert_eq!(error.message(), "path parameters are not consumed (aa,bb)");
    }

    #[test]
    fn test_badpath3() {
        let mut api = ApiDescription::new();
        let ret = api.register(ApiEndpoint::new(
            "test_badpath_handler".to_string(),
            test_badpath_handler,
            Method::GET,
            CONTENT_TYPE_JSON,
            "/{c}/{d}",
        ));
        let error = ret.unwrap_err();
        assert_eq!(
            error.message(),
            "path parameters are not consumed (c,d) and \
             specified parameters do not appear in the path (a,b)"
        );
    }

    #[should_panic(expected = "route paths must begin with a '/': 'I don't \
                               start with a slash'")]
    #[test]
    fn test_badpath4() {
        #[endpoint {
            method = PUT,
            path = "I don't start with a slash"
        }]
        async fn test_badpath_handler(
            _: RequestContext<()>,
        ) -> Result<Response<Body>, HttpError> {
            unimplemented!();
        }

        let mut api = ApiDescription::new();
        api.register(test_badpath_handler).unwrap();
    }

    #[test]
    fn test_dup_names() {
        #[derive(Deserialize, JsonSchema)]
        struct AStruct {}

        #[allow(dead_code)]
        #[derive(Deserialize, JsonSchema)]
        struct TheThing {
            thing: String,
        }

        #[endpoint {
            method = PUT,
            path = "/testing/{thing}"
        }]
        async fn test_dup_names_handler(
            _: RequestContext<()>,
            _: Query<TheThing>,
            _: Path<TheThing>,
        ) -> Result<Response<Body>, HttpError> {
            unimplemented!();
        }

        let mut api = ApiDescription::new();
        let error = api.register(test_dup_names_handler).unwrap_err();
        assert_eq!(
            error.message(),
            "the parameter 'thing' is specified for both query and path \
             parameters",
        );
    }

    #[test]
    fn test_tags_need_one() {
        let mut api = ApiDescription::new().tag_config(TagConfig {
            allow_other_tags: true,
            policy: EndpointTagPolicy::AtLeastOne,
            ..Default::default()
        });
        let ret = api.register(ApiEndpoint::new(
            "test_badpath_handler".to_string(),
            test_badpath_handler,
            Method::GET,
            CONTENT_TYPE_JSON,
            "/{a}/{b}",
        ));
        let error = ret.unwrap_err();
        assert_eq!(error.message(), "At least one tag is required".to_string());
    }

    #[test]
    fn test_tags_too_many() {
        let mut api = ApiDescription::new().tag_config(TagConfig {
            allow_other_tags: true,
            policy: EndpointTagPolicy::ExactlyOne,
            ..Default::default()
        });
        let ret = api.register(
            ApiEndpoint::new(
                "test_badpath_handler".to_string(),
                test_badpath_handler,
                Method::GET,
                CONTENT_TYPE_JSON,
                "/{a}/{b}",
            )
            .tag("howdy")
            .tag("pardner"),
        );
        let error = ret.unwrap_err();
        assert_eq!(error.message(), "Exactly one tag is required");
    }

    #[test]
    fn test_tags_just_right() {
        let mut api = ApiDescription::new().tag_config(TagConfig {
            allow_other_tags: true,
            policy: EndpointTagPolicy::ExactlyOne,
            ..Default::default()
        });
        let ret = api.register(
            ApiEndpoint::new(
                "test_badpath_handler".to_string(),
                test_badpath_handler,
                Method::GET,
                CONTENT_TYPE_JSON,
                "/{a}/{b}",
            )
            .tag("a-tag"),
        );

        assert_eq!(ret, Ok(()));
    }

    #[test]
    fn test_tags_set() {
        // Validate that pre-defined tags and ad-hoc tags are all accounted
        // for and aren't duplicated.
        let mut api = ApiDescription::new().tag_config(TagConfig {
            allow_other_tags: true,
            policy: EndpointTagPolicy::AtLeastOne,
            tags: vec![
                ("a-tag".to_string(), TagDetails::default()),
                ("b-tag".to_string(), TagDetails::default()),
                ("c-tag".to_string(), TagDetails::default()),
            ]
            .into_iter()
            .collect(),
        });
        api.register(
            ApiEndpoint::new(
                "test_badpath_handler".to_string(),
                test_badpath_handler,
                Method::GET,
                CONTENT_TYPE_JSON,
                "/xx/{a}/{b}",
            )
            .tag("a-tag")
            .tag("z-tag"),
        )
        .unwrap();
        api.register(
            ApiEndpoint::new(
                "test_badpath_handler".to_string(),
                test_badpath_handler,
                Method::GET,
                CONTENT_TYPE_JSON,
                "/yy/{a}/{b}",
            )
            .tag("b-tag")
            .tag("y-tag"),
        )
        .unwrap();

        let mut out = Vec::new();
        api.openapi("", "").write(&mut out).unwrap();
        let out = from_utf8(&out).unwrap();
        let spec = serde_json::from_str::<OpenAPI>(out).unwrap();

        let tags = spec
            .tags
            .iter()
            .map(|tag| tag.name.as_str())
            .collect::<HashSet<_>>();

        assert_eq!(
            tags,
            vec!["a-tag", "b-tag", "c-tag", "y-tag", "z-tag",]
                .into_iter()
                .collect::<HashSet<_>>()
        )
    }

    #[test]
    fn test_tag_config_deserialize_old() {
        let config = r#"{
            "allow_other_tags": true,
            "endpoint_tag_policy": "AtLeastOne",
            "tag_definitions": {
                "a-tag": {},
                "b-tag": {}
            }
        }"#;
        let config: TagConfig = serde_json::from_str(config).unwrap();
        assert_eq!(config.policy, EndpointTagPolicy::AtLeastOne);
        assert_eq!(config.tags.len(), 2);
    }
}
