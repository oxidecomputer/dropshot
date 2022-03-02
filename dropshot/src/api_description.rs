// Copyright 2021 Oxide Computer Company
/*!
 * Describes the endpoints and handler functions in your API
 */

use crate::handler::HttpHandlerFunc;
use crate::handler::HttpResponse;
use crate::handler::HttpRouteHandler;
use crate::handler::RouteHandler;
use crate::router::route_path_to_segments;
use crate::router::HttpRouter;
use crate::router::PathSegment;
use crate::server::ServerContext;
use crate::type_util::type_is_scalar;
use crate::type_util::type_is_string_enum;
use crate::Extractor;
use crate::HttpErrorResponseBody;
use crate::CONTENT_TYPE_JSON;
use crate::CONTENT_TYPE_OCTET_STREAM;

use http::Method;
use http::StatusCode;
use std::collections::BTreeMap;
use std::collections::HashSet;

/**
 * ApiEndpoint represents a single API endpoint associated with an
 * ApiDescription. It has a handler, HTTP method (e.g. GET, POST), and a path--
 * provided explicitly--as well as parameters and a description which can be
 * inferred from function parameter types and doc comments (respectively).
 */
#[derive(Debug)]
pub struct ApiEndpoint<Context: ServerContext> {
    pub operation_id: String,
    pub handler: Box<dyn RouteHandler<Context>>,
    pub method: Method,
    pub path: String,
    pub parameters: Vec<ApiEndpointParameter>,
    pub response: ApiEndpointResponse,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub paginated: bool,
    pub visible: bool,
}

impl<'a, Context: ServerContext> ApiEndpoint<Context> {
    pub fn new<HandlerType, FuncParams, ResponseType>(
        operation_id: String,
        handler: HandlerType,
        method: Method,
        path: &'a str,
    ) -> Self
    where
        HandlerType: HttpHandlerFunc<Context, FuncParams, ResponseType>,
        FuncParams: Extractor + 'static,
        ResponseType: HttpResponse + Send + Sync + 'static,
    {
        let func_parameters = FuncParams::metadata();
        let response = ResponseType::metadata();
        ApiEndpoint {
            operation_id,
            handler: HttpRouteHandler::new(handler),
            method,
            path: path.to_string(),
            parameters: func_parameters.parameters,
            response,
            summary: None,
            description: None,
            tags: vec![],
            paginated: func_parameters.paginated,
            visible: true,
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
}

/**
 * ApiEndpointParameter represents the discrete path and query parameters for a
 * given API endpoint. These are typically derived from the members of stucts
 * used as parameters to handler functions.
 */
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
    /** application/octet-stream */
    Bytes,
    /** application/json */
    Json,
}

impl ApiEndpointBodyContentType {
    fn mime_type(&self) -> &str {
        match self {
            ApiEndpointBodyContentType::Bytes => CONTENT_TYPE_OCTET_STREAM,
            ApiEndpointBodyContentType::Json => CONTENT_TYPE_JSON,
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

/**
 * Metadata for an API endpoint response: type information and status code.
 */
#[derive(Debug, Default)]
pub struct ApiEndpointResponse {
    pub schema: Option<ApiSchemaGenerator>,
    pub headers: Vec<ApiEndpointHeader>,
    pub success: Option<StatusCode>,
    pub description: Option<String>,
}

/**
 * Wrapper for both dynamically generated and pre-generated schemas.
 */
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

/**
 * An ApiDescription represents the endpoints and handler functions in your API.
 * Other metadata could also be provided here.  This object can be used to
 * generate an OpenAPI spec or to run an HTTP server implementing the API.
 */
pub struct ApiDescription<Context: ServerContext> {
    /** In practice, all the information we need is encoded in the router. */
    router: HttpRouter<Context>,
}

impl<Context: ServerContext> ApiDescription<Context> {
    pub fn new() -> Self {
        ApiDescription { router: HttpRouter::new() }
    }

    /**
     * Register a new API endpoint.
     */
    pub fn register<T>(&mut self, endpoint: T) -> Result<(), String>
    where
        T: Into<ApiEndpoint<Context>>,
    {
        let e = endpoint.into();

        self.validate_path_parameters(&e)?;
        self.validate_body_parameters(&e)?;
        self.validate_named_parameters(&e)?;

        self.router.insert(e);

        Ok(())
    }

    /**
     * Validate that the parameters specified in the path match the parameters
     * specified by the path parameter arguments to the handler function.
     */
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

    /**
     * Validate that we have a single body parameter.
     */
    fn validate_body_parameters(
        &self,
        e: &ApiEndpoint<Context>,
    ) -> Result<(), String> {
        // Explicitly disallow any attempt to consume the body twice.
        let nbodyextractors = e
            .parameters
            .iter()
            .filter(|p| match p.metadata {
                ApiEndpointParameterMetadata::Body(..) => true,
                _ => false,
            })
            .count();
        if nbodyextractors > 1 {
            return Err(format!(
                "only one body extractor can be used in a handler (this \
                 function has {})",
                nbodyextractors
            ));
        }

        Ok(())
    }

    /**
     * Validate that named parameters have appropriate types and their aren't
     * duplicates. Parameters must have scalar types except in the case of the
     * received for a wildcard path which must be an array of String.
     */
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
            /* Skip anything that's not a path or query parameter (i.e. body) */
            match &param.metadata {
                ApiEndpointParameterMetadata::Path(_)
                | ApiEndpointParameterMetadata::Query(_) => (),
                _ => continue,
            }
            /* Only body parameters should have unresolved schemas */
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
                            type_is_scalar(name, schema, dependencies)?;
                        }
                        Some(SegmentOrWildcard::Wildcard) => {
                            type_is_string_enum(name, schema, dependencies)?;
                        }
                        None => {
                            panic!("all path variables should be accounted for")
                        }
                    }
                }
                ApiEndpointParameterMetadata::Query(ref name) => {
                    if path_segments.get(name).is_some() {
                        return Err(format!(
                            "the parameter '{}' is specified for both query \
                             and path parameters",
                            name
                        ));
                    }
                    type_is_scalar(name, schema, dependencies)?;
                }
                _ => (),
            }
        }

        Ok(())
    }

    /**
     * Build the OpenAPI definition describing this API.  Returns an
     * [`OpenApiDefinition`] which can be used to specify the contents of the
     * definition and select an output format.
     *
     * The arguments to this function will be used for the mandatory `title` and
     * `version` properties that the `Info` object in an OpenAPI definition must
     * contain.
     */
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

    /**
     * Internal routine for constructing the OpenAPI definition describing this
     * API in its JSON form.
     */
    fn gen_openapi(&self, info: openapiv3::Info) -> openapiv3::OpenAPI {
        let mut openapi = openapiv3::OpenAPI::default();

        openapi.openapi = "3.0.3".to_string();
        openapi.info = info;

        let settings = schemars::gen::SchemaSettings::openapi3();
        let mut generator = schemars::gen::SchemaGenerator::new(settings);
        let mut definitions =
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
            operation.summary = endpoint.summary.clone();
            operation.description = endpoint.description.clone();
            operation.tags = endpoint.tags.clone();

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
                                    parameter_data: parameter_data,
                                    allow_reserved: false,
                                    style: openapiv3::QueryStyle::Form,
                                    allow_empty_value: None,
                                },
                            ))
                        }
                        ApiEndpointParameterLocation::Path => {
                            Some(openapiv3::ReferenceOr::Item(
                                openapiv3::Parameter::Path {
                                    parameter_data: parameter_data,
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
                        content: content,
                        required: true,
                        ..Default::default()
                    }))
                })
                .next();

            if endpoint.paginated {
                operation.extensions.insert(
                    crate::pagination::PAGINATION_EXTENSION.to_string(),
                    serde_json::json! {true},
                );
            }

            if let Some(schema) = &endpoint.response.schema {
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

                match &endpoint.response.success {
                    None => {
                        // Without knowing the specific status code we use the
                        // 2xx range.
                        operation.responses.responses.insert(
                            openapiv3::StatusCode::Range(2),
                            openapiv3::ReferenceOr::Item(response),
                        );
                    }
                    Some(code) => {
                        operation.responses.responses.insert(
                            openapiv3::StatusCode::Code(code.as_u16()),
                            openapiv3::ReferenceOr::Item(response),
                        );
                    }
                }

                // 4xx and 5xx responses all use the same error information
                let err_ref = openapiv3::ReferenceOr::ref_(
                    "#/components/responses/Error",
                );
                operation
                    .responses
                    .responses
                    .insert(openapiv3::StatusCode::Range(4), err_ref.clone());
                operation
                    .responses
                    .responses
                    .insert(openapiv3::StatusCode::Range(5), err_ref);
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
                operation.responses.default =
                    Some(openapiv3::ReferenceOr::Item(openapiv3::Response {
                        // TODO: perhaps we should require even free-form
                        // responses to have a description since it's required
                        // by OpenAPI.
                        description: "".to_string(),
                        content,
                        ..Default::default()
                    }));
            }

            // Drop in the operation.
            method_ref.replace(operation);
        }

        let components = &mut openapi
            .components
            .get_or_insert_with(openapiv3::Components::default);

        // All endpoints share an error response
        let responses = &mut components.responses;
        let mut content = indexmap::IndexMap::new();
        content.insert(
            CONTENT_TYPE_JSON.to_string(),
            openapiv3::MediaType {
                schema: Some(j2oas_schema(
                    None,
                    &generator.subschema_for::<HttpErrorResponseBody>(),
                )),
                ..Default::default()
            },
        );

        responses.insert(
            "Error".to_string(),
            openapiv3::ReferenceOr::Item(openapiv3::Response {
                description: "Error".to_string(),
                content: content,
                ..Default::default()
            }),
        );

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

    /*
     * TODO-cleanup is there a way to make this available only within this
     * crate?  Once we do that, we don't need to consume the ApiDescription to
     * do this.
     */
    pub fn into_router(self) -> HttpRouter<Context> {
        self.router
    }
}

/**
 * Returns true iff the schema represents the void schema that matches no data.
 */
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

/**
 * Convert from JSON Schema into OpenAPI.
 */
/*
 * TODO Initially this seemed like it was going to be a win, but the versions
 * of JSON Schema that the schemars and openapiv3 crates adhere to are just
 * different enough to make the conversion a real pain in the neck. A better
 * approach might be a derive(OpenAPI)-like thing, or even a generic
 * derive(schema) that we could then marshall into OpenAPI.
 * The schemars crate also seems a bit inflexible when it comes to how the
 * schema is generated wrt references vs. inline types.
 */
fn j2oas_schema(
    name: Option<&String>,
    schema: &schemars::schema::Schema,
) -> openapiv3::ReferenceOr<openapiv3::Schema> {
    match schema {
        /*
         * The permissive, "match anything" schema. We'll typically see this
         * when consumers use a type such as serde_json::Value.
         */
        schemars::schema::Schema::Bool(true) => {
            openapiv3::ReferenceOr::Item(openapiv3::Schema {
                schema_data: openapiv3::SchemaData::default(),
                schema_kind: openapiv3::SchemaKind::Any(
                    openapiv3::AnySchema::default(),
                ),
            })
        }
        schemars::schema::Schema::Bool(false) => {
            panic!("We don't expect to see a schema that matches the null set")
        }
        schemars::schema::Schema::Object(obj) => j2oas_schema_object(name, obj),
    }
}

fn j2oas_schema_object(
    name: Option<&String>,
    obj: &schemars::schema::SchemaObject,
) -> openapiv3::ReferenceOr<openapiv3::Schema> {
    if let Some(reference) = &obj.reference {
        return openapiv3::ReferenceOr::Reference {
            reference: reference.clone(),
        };
    }

    let ty = match &obj.instance_type {
        Some(schemars::schema::SingleOrVec::Single(ty)) => Some(ty.as_ref()),
        Some(schemars::schema::SingleOrVec::Vec(_)) => {
            panic!("unsupported by openapiv3")
        }
        None => None,
    };

    let kind = match (ty, &obj.subschemas) {
        (Some(schemars::schema::InstanceType::Null), None) => {
            openapiv3::SchemaKind::Type(openapiv3::Type::String(
                openapiv3::StringType {
                    enumeration: vec![None],
                    ..Default::default()
                },
            ))
        }
        (Some(schemars::schema::InstanceType::Boolean), None) => {
            openapiv3::SchemaKind::Type(openapiv3::Type::Boolean {})
        }
        (Some(schemars::schema::InstanceType::Object), None) => {
            j2oas_object(&obj.object)
        }
        (Some(schemars::schema::InstanceType::Array), None) => {
            j2oas_array(&obj.array)
        }
        (Some(schemars::schema::InstanceType::Number), None) => {
            j2oas_number(&obj.format, &obj.number, &obj.enum_values)
        }
        (Some(schemars::schema::InstanceType::String), None) => {
            j2oas_string(&obj.format, &obj.string, &obj.enum_values)
        }
        (Some(schemars::schema::InstanceType::Integer), None) => {
            j2oas_integer(&obj.format, &obj.number, &obj.enum_values)
        }
        (None, Some(subschema)) => j2oas_subschemas(subschema),
        (None, None) => {
            openapiv3::SchemaKind::Any(openapiv3::AnySchema::default())
        }
        _ => panic!("invalid {:#?}", obj),
    };

    let mut data = openapiv3::SchemaData::default();

    if matches!(
        &obj.extensions.get("nullable"),
        Some(serde_json::Value::Bool(true))
    ) {
        data.nullable = true;
    }

    if let Some(metadata) = &obj.metadata {
        data.title = metadata.title.clone();
        data.description = metadata.description.clone();
        // TODO skipping `default` since it's a little tricky to handle
        data.deprecated = metadata.deprecated;
        data.read_only = metadata.read_only;
        data.write_only = metadata.write_only;
    }

    if let Some(name) = name {
        data.title = Some(name.clone());
    }

    openapiv3::ReferenceOr::Item(openapiv3::Schema {
        schema_data: data,
        schema_kind: kind,
    })
}

fn j2oas_subschemas(
    subschemas: &schemars::schema::SubschemaValidation,
) -> openapiv3::SchemaKind {
    match (
        &subschemas.all_of,
        &subschemas.any_of,
        &subschemas.one_of,
        &subschemas.not,
    ) {
        (Some(all_of), None, None, None) => openapiv3::SchemaKind::AllOf {
            all_of: all_of
                .iter()
                .map(|schema| j2oas_schema(None, schema))
                .collect::<Vec<_>>(),
        },
        (None, Some(any_of), None, None) => openapiv3::SchemaKind::AnyOf {
            any_of: any_of
                .iter()
                .map(|schema| j2oas_schema(None, schema))
                .collect::<Vec<_>>(),
        },
        (None, None, Some(one_of), None) => openapiv3::SchemaKind::OneOf {
            one_of: one_of
                .iter()
                .map(|schema| j2oas_schema(None, schema))
                .collect::<Vec<_>>(),
        },
        (None, None, None, Some(not)) => openapiv3::SchemaKind::Not {
            not: Box::new(j2oas_schema(None, not)),
        },
        _ => panic!("invalid subschema {:#?}", subschemas),
    }
}

fn j2oas_integer(
    format: &Option<String>,
    number: &Option<Box<schemars::schema::NumberValidation>>,
    enum_values: &Option<Vec<serde_json::value::Value>>,
) -> openapiv3::SchemaKind {
    let format = match format.as_ref().map(|s| s.as_str()) {
        None => openapiv3::VariantOrUnknownOrEmpty::Empty,
        Some("int32") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::IntegerFormat::Int32,
        ),
        Some("int64") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::IntegerFormat::Int64,
        ),
        Some(other) => {
            openapiv3::VariantOrUnknownOrEmpty::Unknown(other.to_string())
        }
    };

    let (multiple_of, minimum, exclusive_minimum, maximum, exclusive_maximum) =
        match number {
            None => (None, None, false, None, false),
            Some(number) => {
                let multiple_of = number.multiple_of.map(|f| f as i64);
                let (minimum, exclusive_minimum) =
                    match (number.minimum, number.exclusive_minimum) {
                        (None, None) => (None, false),
                        (Some(f), None) => (Some(f as i64), false),
                        (None, Some(f)) => (Some(f as i64), true),
                        _ => panic!("invalid"),
                    };
                let (maximum, exclusive_maximum) =
                    match (number.maximum, number.exclusive_maximum) {
                        (None, None) => (None, false),
                        (Some(f), None) => (Some(f as i64), false),
                        (None, Some(f)) => (Some(f as i64), true),
                        _ => panic!("invalid"),
                    };

                (
                    multiple_of,
                    minimum,
                    exclusive_minimum,
                    maximum,
                    exclusive_maximum,
                )
            }
        };

    let enumeration = enum_values
        .iter()
        .flat_map(|v| {
            v.iter().map(|vv| match vv {
                serde_json::Value::Null => None,
                serde_json::Value::Number(value) => {
                    Some(value.as_i64().unwrap())
                }
                _ => panic!("unexpected enumeration value {:?}", vv),
            })
        })
        .collect::<Vec<_>>();

    openapiv3::SchemaKind::Type(openapiv3::Type::Integer(
        openapiv3::IntegerType {
            format,
            multiple_of,
            exclusive_minimum,
            exclusive_maximum,
            minimum,
            maximum,
            enumeration,
        },
    ))
}

fn j2oas_number(
    format: &Option<String>,
    number: &Option<Box<schemars::schema::NumberValidation>>,
    enum_values: &Option<Vec<serde_json::value::Value>>,
) -> openapiv3::SchemaKind {
    let format = match format.as_ref().map(|s| s.as_str()) {
        None => openapiv3::VariantOrUnknownOrEmpty::Empty,
        Some("float") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::NumberFormat::Float,
        ),
        Some("double") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::NumberFormat::Double,
        ),
        Some(other) => {
            openapiv3::VariantOrUnknownOrEmpty::Unknown(other.to_string())
        }
    };

    let (multiple_of, minimum, exclusive_minimum, maximum, exclusive_maximum) =
        match number {
            None => (None, None, false, None, false),
            Some(number) => {
                let multiple_of = number.multiple_of;
                let (minimum, exclusive_minimum) =
                    match (number.minimum, number.exclusive_minimum) {
                        (None, None) => (None, false),
                        (s @ Some(_), None) => (s, false),
                        (None, s @ Some(_)) => (s, true),
                        _ => panic!("invalid"),
                    };
                let (maximum, exclusive_maximum) =
                    match (number.maximum, number.exclusive_maximum) {
                        (None, None) => (None, false),
                        (s @ Some(_), None) => (s, false),
                        (None, s @ Some(_)) => (s, true),
                        _ => panic!("invalid"),
                    };

                (
                    multiple_of,
                    minimum,
                    exclusive_minimum,
                    maximum,
                    exclusive_maximum,
                )
            }
        };

    let enumeration = enum_values
        .iter()
        .flat_map(|v| {
            v.iter().map(|vv| match vv {
                serde_json::Value::Null => None,
                serde_json::Value::Number(value) => {
                    Some(value.as_f64().unwrap())
                }
                _ => panic!("unexpected enumeration value {:?}", vv),
            })
        })
        .collect::<Vec<_>>();

    openapiv3::SchemaKind::Type(openapiv3::Type::Number(
        openapiv3::NumberType {
            format,
            multiple_of,
            exclusive_minimum,
            exclusive_maximum,
            minimum,
            maximum,
            enumeration,
        },
    ))
}

fn j2oas_string(
    format: &Option<String>,
    string: &Option<Box<schemars::schema::StringValidation>>,
    enum_values: &Option<Vec<serde_json::value::Value>>,
) -> openapiv3::SchemaKind {
    let format = match format.as_ref().map(|s| s.as_str()) {
        None => openapiv3::VariantOrUnknownOrEmpty::Empty,
        Some("date") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::StringFormat::Date,
        ),
        Some("date-time") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::StringFormat::DateTime,
        ),
        Some("password") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::StringFormat::Password,
        ),
        Some("byte") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::StringFormat::Byte,
        ),
        Some("binary") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::StringFormat::Binary,
        ),
        Some(other) => {
            openapiv3::VariantOrUnknownOrEmpty::Unknown(other.to_string())
        }
    };

    let (max_length, min_length, pattern) = match string.as_ref() {
        None => (None, None, None),
        Some(string) => (
            string.max_length.map(|n| n as usize),
            string.min_length.map(|n| n as usize),
            string.pattern.clone(),
        ),
    };

    let enumeration = enum_values
        .iter()
        .flat_map(|v| {
            v.iter().map(|vv| match vv {
                serde_json::Value::Null => None,
                serde_json::Value::String(s) => Some(s.clone()),
                _ => panic!("unexpected enumeration value {:?}", vv),
            })
        })
        .collect::<Vec<_>>();

    openapiv3::SchemaKind::Type(openapiv3::Type::String(
        openapiv3::StringType {
            format,
            pattern,
            enumeration,
            min_length,
            max_length,
        },
    ))
}

fn j2oas_array(
    array: &Option<Box<schemars::schema::ArrayValidation>>,
) -> openapiv3::SchemaKind {
    let arr = array.as_ref().unwrap();

    openapiv3::SchemaKind::Type(openapiv3::Type::Array(openapiv3::ArrayType {
        items: match &arr.items {
            Some(schemars::schema::SingleOrVec::Single(schema)) => {
                Some(box_reference_or(j2oas_schema(None, &schema)))
            }
            Some(schemars::schema::SingleOrVec::Vec(_)) => {
                panic!("OpenAPI v3.0.x cannot support tuple-like arrays")
            }
            None => None,
        },
        min_items: arr.min_items.map(|n| n as usize),
        max_items: arr.max_items.map(|n| n as usize),
        unique_items: arr.unique_items.unwrap_or(false),
    }))
}

fn box_reference_or<T>(
    r: openapiv3::ReferenceOr<T>,
) -> openapiv3::ReferenceOr<Box<T>> {
    match r {
        openapiv3::ReferenceOr::Item(schema) => {
            openapiv3::ReferenceOr::boxed_item(schema)
        }
        openapiv3::ReferenceOr::Reference { reference } => {
            openapiv3::ReferenceOr::Reference { reference }
        }
    }
}

fn j2oas_object(
    object: &Option<Box<schemars::schema::ObjectValidation>>,
) -> openapiv3::SchemaKind {
    match object {
        None => openapiv3::SchemaKind::Type(openapiv3::Type::Object(
            openapiv3::ObjectType::default(),
        )),
        Some(obj) => openapiv3::SchemaKind::Type(openapiv3::Type::Object(
            openapiv3::ObjectType {
                properties: obj
                    .properties
                    .iter()
                    .map(|(prop, schema)| {
                        (
                            prop.clone(),
                            box_reference_or(j2oas_schema(None, schema)),
                        )
                    })
                    .collect::<_>(),
                required: obj.required.iter().cloned().collect::<_>(),
                additional_properties: obj.additional_properties.as_ref().map(
                    |schema| match schema.as_ref() {
                        schemars::schema::Schema::Bool(b) => {
                            openapiv3::AdditionalProperties::Any(*b)
                        }
                        schemars::schema::Schema::Object(obj) => {
                            openapiv3::AdditionalProperties::Schema(Box::new(
                                j2oas_schema_object(None, obj),
                            ))
                        }
                    },
                ),
                min_properties: obj.min_properties.map(|n| n as usize),
                max_properties: obj.max_properties.map(|n| n as usize),
            },
        )),
    }
}

/**
 * This object is used to specify configuration for building an OpenAPI
 * definition document.  It is constructed using [`ApiDescription::openapi()`].
 * Additional optional properties may be added and then the OpenAPI definition
 * document may be generated via [`write()`](`OpenApiDefinition::write`) or
 * [`json()`](`OpenApiDefinition::json`).
 */
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

    /**
     * Provide a short description of the API.  CommonMark syntax may be
     * used for rich text representation.
     *
     * This routine will set the `description` field of the `Info` object in the
     * OpenAPI definition.
     */
    pub fn description<S: AsRef<str>>(&mut self, description: S) -> &mut Self {
        self.info.description = Some(description.as_ref().to_string());
        self
    }

    /**
     * Include a Terms of Service URL for the API.  Must be in the format of a
     * URL.
     *
     * This routine will set the `termsOfService` field of the `Info` object in
     * the OpenAPI definition.
     */
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

    /**
     * Set the identifying name of the contact person or organisation
     * responsible for the API.
     *
     * This routine will set the `name` property of the `Contact` object within
     * the `Info` object in the OpenAPI definition.
     */
    pub fn contact_name<S: AsRef<str>>(&mut self, name: S) -> &mut Self {
        self.contact_mut().name = Some(name.as_ref().to_string());
        self
    }

    /**
     * Set a contact URL for the API.  Must be in the format of a URL.
     *
     * This routine will set the `url` property of the `Contact` object within
     * the `Info` object in the OpenAPI definition.
     */
    pub fn contact_url<S: AsRef<str>>(&mut self, url: S) -> &mut Self {
        self.contact_mut().url = Some(url.as_ref().to_string());
        self
    }

    /**
     * Set the email address of the contact person or organisation responsible
     * for the API.  Must be in the format of an email address.
     *
     * This routine will set the `email` property of the `Contact` object within
     * the `Info` object in the OpenAPI definition.
     */
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

    /**
     * Provide the name of the licence used for the API, and a URL (must be in
     * URL format) displaying the licence text.
     *
     * This routine will set the `name` and optional `url` properties of the
     * `License` object within the `Info` object in the OpenAPI definition.
     */
    pub fn license<S1, S2>(&mut self, name: S1, url: S2) -> &mut Self
    where
        S1: AsRef<str>,
        S2: AsRef<str>,
    {
        self.license_mut(name.as_ref()).url = Some(url.as_ref().to_string());
        self
    }

    /**
     * Provide the name of the licence used for the API.
     *
     * This routine will set the `name` property of the License object within
     * the `Info` object in the OpenAPI definition.
     */
    pub fn license_name<S: AsRef<str>>(&mut self, name: S) -> &mut Self {
        self.license_mut(name.as_ref());
        self
    }

    /**
     * Build a JSON object containing the OpenAPI definition for this API.
     */
    pub fn json(&self) -> serde_json::Result<serde_json::Value> {
        serde_json::to_value(&self.api.gen_openapi(self.info.clone()))
    }

    /**
     * Build a JSON object containing the OpenAPI definition for this API and
     * write it to the provided stream.
     */
    pub fn write(
        &self,
        out: &mut dyn std::io::Write,
    ) -> serde_json::Result<()> {
        serde_json::to_writer_pretty(
            out,
            &self.api.gen_openapi(self.info.clone()),
        )
    }
}

#[cfg(test)]
mod test {
    use super::super::error::HttpError;
    use super::super::handler::RequestContext;
    use super::super::Path;
    use super::j2oas_schema;
    use super::ApiDescription;
    use super::ApiEndpoint;
    use crate as dropshot; /* for "endpoint" macro */
    use crate::api_description::j2oas_schema_object;
    use crate::endpoint;
    use crate::Query;
    use crate::TypedBody;
    use crate::UntypedBody;
    use http::Method;
    use hyper::Body;
    use hyper::Response;
    use schemars::JsonSchema;
    use serde::Deserialize;
    use std::sync::Arc;

    #[derive(Deserialize, JsonSchema)]
    #[allow(dead_code)]
    struct TestPath {
        a: String,
        b: String,
    }

    async fn test_badpath_handler(
        _: Arc<RequestContext<()>>,
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
            "/",
        ));
        assert_eq!(
            ret,
            Err("specified parameters do not appear in the path (a,b)"
                .to_string())
        )
    }

    #[test]
    fn test_badpath2() {
        let mut api = ApiDescription::new();
        let ret = api.register(ApiEndpoint::new(
            "test_badpath_handler".to_string(),
            test_badpath_handler,
            Method::GET,
            "/{a}/{aa}/{b}/{bb}",
        ));
        assert_eq!(
            ret,
            Err("path parameters are not consumed (aa,bb)".to_string())
        );
    }

    #[test]
    fn test_badpath3() {
        let mut api = ApiDescription::new();
        let ret = api.register(ApiEndpoint::new(
            "test_badpath_handler".to_string(),
            test_badpath_handler,
            Method::GET,
            "/{c}/{d}",
        ));
        assert_eq!(
            ret,
            Err("path parameters are not consumed (c,d) and specified \
                 parameters do not appear in the path (a,b)"
                .to_string())
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
            _: Arc<RequestContext<()>>,
        ) -> Result<Response<Body>, HttpError> {
            unimplemented!();
        }

        let mut api = ApiDescription::new();
        api.register(test_badpath_handler).unwrap();
    }

    #[test]
    fn test_empty_struct() {
        #[derive(JsonSchema)]
        struct Empty {}

        let settings = schemars::gen::SchemaSettings::openapi3();
        let mut generator = schemars::gen::SchemaGenerator::new(settings);

        let schema = Empty::json_schema(&mut generator);
        let _ = j2oas_schema(None, &schema);
    }

    #[test]
    fn test_garbage_barge_structure_conversion() {
        #[allow(dead_code)]
        #[derive(JsonSchema)]
        struct SuperGarbage {
            string: String,
            strings: Vec<String>,
            more_strings: [String; 3],
            substruct: Substruct,
            more: Option<Substruct>,
            union: Union,
            map: std::collections::BTreeMap<String, String>,
        }

        #[allow(dead_code)]
        #[derive(JsonSchema)]
        struct Substruct {
            ii32: i32,
            uu64: u64,
            ff: f32,
            dd: f64,
            b: bool,
        }

        #[allow(dead_code)]
        #[derive(JsonSchema)]
        enum Union {
            A { a: u32 },
            B { b: f32 },
        }

        let settings = schemars::gen::SchemaSettings::openapi3();
        let mut generator = schemars::gen::SchemaGenerator::new(settings);

        let schema = SuperGarbage::json_schema(&mut generator);
        let _ = j2oas_schema(None, &schema);
        for (key, schema) in generator.definitions().iter() {
            let _ = j2oas_schema(Some(key), schema);
        }
    }

    #[test]
    fn test_two_bodies() {
        #[derive(Deserialize, JsonSchema)]
        struct AStruct {}

        #[endpoint {
            method = PUT,
            path = "/testing/two_bodies"
        }]
        async fn test_twobodies_handler(
            _: Arc<RequestContext<()>>,
            _: UntypedBody,
            _: TypedBody<AStruct>,
        ) -> Result<Response<Body>, HttpError> {
            unimplemented!();
        }

        let mut api = ApiDescription::new();
        let error = api.register(test_twobodies_handler).unwrap_err();
        assert_eq!(
            error,
            "only one body extractor can be used in a handler (this function \
             has 2)"
        );
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
            _: Arc<RequestContext<()>>,
            _: Query<TheThing>,
            _: Path<TheThing>,
        ) -> Result<Response<Body>, HttpError> {
            unimplemented!();
        }

        let mut api = ApiDescription::new();
        let error = api.register(test_dup_names_handler).unwrap_err();
        assert_eq!(
            error,
            "the parameter 'thing' is specified for both query and path \
             parameters",
        );
    }

    #[test]
    fn test_additional_properties() {
        #[allow(dead_code)]
        #[derive(JsonSchema)]
        enum Union {
            A { a: u32 },
        }
        let settings = schemars::gen::SchemaSettings::openapi3();
        let mut generator = schemars::gen::SchemaGenerator::new(settings);
        let schema = Union::json_schema(&mut generator);
        let _ = j2oas_schema(None, &schema);
        for (key, schema) in generator.definitions().iter() {
            let _ = j2oas_schema(Some(key), schema);
        }
    }

    #[test]
    fn test_nullable() {
        #[allow(dead_code)]
        #[derive(JsonSchema)]
        struct Foo {
            bar: String,
        }
        let settings = schemars::gen::SchemaSettings::openapi3();
        let generator = schemars::gen::SchemaGenerator::new(settings);
        let root_schema = generator.into_root_schema_for::<Option<Foo>>();
        let schema = root_schema.schema;
        let os = j2oas_schema_object(None, &schema);

        assert_eq!(
            os,
            openapiv3::ReferenceOr::Item(openapiv3::Schema {
                schema_data: openapiv3::SchemaData {
                    title: Some("Nullable_Foo".to_string()),
                    nullable: true,
                    ..Default::default()
                },
                schema_kind: openapiv3::SchemaKind::AllOf {
                    all_of: vec![openapiv3::ReferenceOr::Reference {
                        reference: "#/components/schemas/Foo".to_string()
                    }],
                },
            })
        );
    }
}
