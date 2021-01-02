// Copyright 2020 Oxide Computer Company
/*!
 * Describes the endpoints and handler functions in your API
 */

use crate::handler::HttpHandlerFunc;
use crate::handler::HttpResponse;
use crate::handler::HttpRouteHandler;
use crate::handler::RouteHandler;
use crate::router::path_to_segments;
use crate::router::HttpRouter;
use crate::router::PathSegment;
use crate::Extractor;
use crate::CONTENT_TYPE_JSON;

use http::Method;
use http::StatusCode;
use std::collections::HashSet;

/**
 * ApiEndpoint represents a single API endpoint associated with an
 * ApiDescription. It has a handler, HTTP method (e.g. GET, POST), and a path--
 * provided explicitly--as well as parameters and a description which can be
 * inferred from function parameter types and doc comments (respectively).
 */
#[derive(Debug)]
pub struct ApiEndpoint {
    pub operation_id: String,
    pub handler: Box<dyn RouteHandler>,
    pub method: Method,
    pub path: String,
    pub parameters: Vec<ApiEndpointParameter>,
    pub response: ApiEndpointResponse,
    pub description: Option<String>,
    pub tags: Vec<String>,
}

impl<'a> ApiEndpoint {
    pub fn new<HandlerType, FuncParams, ResponseType>(
        operation_id: String,
        handler: HandlerType,
        method: Method,
        path: &'a str,
    ) -> Self
    where
        HandlerType: HttpHandlerFunc<FuncParams, ResponseType>,
        FuncParams: Extractor + 'static,
        ResponseType: HttpResponse + Send + Sync + 'static,
    {
        ApiEndpoint {
            operation_id: operation_id,
            handler: HttpRouteHandler::new(handler),
            method: method,
            path: path.to_string(),
            parameters: FuncParams::metadata(),
            response: ResponseType::metadata(),
            description: None,
            tags: vec![],
        }
    }

    pub fn description<T: ToString>(mut self, description: T) -> Self {
        self.description.replace(description.to_string());
        self
    }

    pub fn tag<T: ToString>(mut self, tag: T) -> Self {
        self.tags.push(tag.to_string());
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
    pub name: ApiEndpointParameterName,
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
            name: match loc {
                ApiEndpointParameterLocation::Path => {
                    ApiEndpointParameterName::Path(name)
                }
                ApiEndpointParameterLocation::Query => {
                    ApiEndpointParameterName::Query(name)
                }
            },
            description,
            required,
            schema,
            examples,
        }
    }

    pub fn new_body(
        description: Option<String>,
        required: bool,
        schema: ApiSchemaGenerator,
        examples: Vec<String>,
    ) -> Self {
        Self {
            name: ApiEndpointParameterName::Body,
            description,
            required,
            schema,
            examples,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ApiEndpointParameterLocation {
    Path,
    Query,
}

#[derive(Debug, Clone)]
pub enum ApiEndpointParameterName {
    Path(String),
    Query(String),
    Body,
}

/**
 * Metadata for an API endpoint response: type information and status code.
 */
#[derive(Debug)]
pub struct ApiEndpointResponse {
    pub schema: Option<ApiSchemaGenerator>,
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
    Static(schemars::schema::Schema),
}

impl std::fmt::Debug for ApiSchemaGenerator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiSchemaGenerator::Gen {
                ..
            } => f.write_str("[schema generator]"),
            ApiSchemaGenerator::Static(schema) => {
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
pub struct ApiDescription {
    /** In practice, all the information we need is encoded in the router. */
    router: HttpRouter,
}

impl ApiDescription {
    pub fn new() -> Self {
        ApiDescription {
            router: HttpRouter::new(),
        }
    }

    /**
     * Register a new API endpoint.
     */
    pub fn register<T>(&mut self, endpoint: T) -> Result<(), String>
    where
        T: Into<ApiEndpoint>,
    {
        let e = endpoint.into();

        // Gather up the path parameters and the path variable components, and
        // make sure they're identical.
        let path = path_to_segments(&e.path)
            .iter()
            .filter_map(|segment| match PathSegment::from(segment) {
                PathSegment::Varname(v) => Some(v),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let vars = e
            .parameters
            .iter()
            .filter_map(|p| match &p.name {
                ApiEndpointParameterName::Path(name) => Some(name.clone()),
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

            return match (p.is_empty(), v.is_empty()) {
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
            };
        }

        self.router.insert(e);

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
    pub fn openapi<S1, S2>(&self, title: S1, version: S2) -> OpenApiDefinition
    where
        S1: AsRef<str>,
        S2: AsRef<str>,
    {
        OpenApiDefinition::new(self, title.as_ref(), version.as_ref())
    }

    /**
     * Emit the OpenAPI Spec document describing this API in its JSON form.
     *
     * This routine is deprecated in favour of the new openapi() builder
     * routine.
     */
    #[deprecated(note = "switch to openapi()")]
    pub fn print_openapi(
        &self,
        out: &mut dyn std::io::Write,
        title: &dyn ToString,
        description: Option<&dyn ToString>,
        terms_of_service: Option<&dyn ToString>,
        contact_name: Option<&dyn ToString>,
        contact_url: Option<&dyn ToString>,
        contact_email: Option<&dyn ToString>,
        license_name: Option<&dyn ToString>,
        license_url: Option<&dyn ToString>,
        version: &dyn ToString,
    ) -> serde_json::Result<()> {
        let mut oapi = self.openapi(title.to_string(), version.to_string());
        if let Some(s) = description {
            oapi.description(s.to_string());
        }
        if let Some(s) = terms_of_service {
            oapi.terms_of_service(s.to_string());
        }
        if let Some(s) = contact_name {
            oapi.contact_name(s.to_string());
        }
        if let Some(s) = contact_url {
            oapi.contact_url(s.to_string());
        }
        if let Some(s) = contact_email {
            oapi.contact_email(s.to_string());
        }
        if let (Some(name), Some(url)) = (license_name, license_url) {
            oapi.license(name.to_string(), url.to_string());
        } else if let Some(name) = license_name {
            oapi.license_name(name.to_string());
        }

        oapi.write(out)
    }

    /**
     * Internal routine for constructing the OpenAPI definition describing this
     * API in its JSON form.
     */
    // TODO: There's a bunch of error handling we need here such as checking
    // for duplicate parameter names.
    fn gen_openapi(&self, info: openapiv3::Info) -> openapiv3::OpenAPI {
        let mut openapi = openapiv3::OpenAPI::default();

        openapi.openapi = "3.0.3".to_string();
        openapi.info = info;

        let settings = schemars::gen::SchemaSettings::openapi3();
        let mut generator = schemars::gen::SchemaGenerator::new(settings);

        for (path, method, endpoint) in &self.router {
            let path = openapi.paths.entry(path).or_insert(
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
            operation.description = endpoint.description.clone();
            operation.tags = endpoint.tags.clone();

            operation.parameters = endpoint
                .parameters
                .iter()
                .filter_map(|param| {
                    let (name, location) = match &param.name {
                        ApiEndpointParameterName::Body => return None,
                        ApiEndpointParameterName::Path(name) => {
                            (name, ApiEndpointParameterLocation::Path)
                        }
                        ApiEndpointParameterName::Query(name) => {
                            (name, ApiEndpointParameterLocation::Query)
                        }
                    };

                    let schema = match &param.schema {
                        ApiSchemaGenerator::Static(schema) => {
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
                    match &param.name {
                        ApiEndpointParameterName::Body => (),
                        _ => return None,
                    }

                    let (name, js) = match &param.schema {
                        ApiSchemaGenerator::Gen {
                            name,
                            schema,
                        } => (Some(name()), schema(&mut generator)),
                        ApiSchemaGenerator::Static(schema) => {
                            (None, schema.clone())
                        }
                    };
                    let schema = j2oas_schema(name.as_ref(), &js);

                    let mut content = indexmap::IndexMap::new();
                    content.insert(
                        CONTENT_TYPE_JSON.to_string(),
                        openapiv3::MediaType {
                            schema: Some(schema),
                            example: None,
                            examples: indexmap::IndexMap::new(),
                            encoding: indexmap::IndexMap::new(),
                        },
                    );

                    Some(openapiv3::ReferenceOr::Item(openapiv3::RequestBody {
                        description: None,
                        content: content,
                        required: true,
                    }))
                })
                .next();

            if let Some(schema) = &endpoint.response.schema {
                let (name, js) = match schema {
                    ApiSchemaGenerator::Gen {
                        name,
                        schema,
                    } => (Some(name()), schema(&mut generator)),
                    ApiSchemaGenerator::Static(schema) => {
                        (None, schema.clone())
                    }
                };
                let mut content = indexmap::IndexMap::new();
                if !is_null(&js) {
                    content.insert(
                        CONTENT_TYPE_JSON.to_string(),
                        openapiv3::MediaType {
                            schema: Some(j2oas_schema(name.as_ref(), &js)),
                            example: None,
                            examples: indexmap::IndexMap::new(),
                            encoding: indexmap::IndexMap::new(),
                        },
                    );
                }

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
                    headers: indexmap::IndexMap::new(),
                    content: content,
                    links: indexmap::IndexMap::new(),
                };

                match &endpoint.response.success {
                    None => {
                        operation.responses.default =
                            Some(openapiv3::ReferenceOr::Item(response))
                    }
                    Some(code) => {
                        operation.responses.responses.insert(
                            openapiv3::StatusCode::Code(code.as_u16()),
                            openapiv3::ReferenceOr::Item(response),
                        );
                    }
                }
            }

            // Drop in the operation.
            method_ref.replace(operation);
        }

        // Add the schemas for which we generated references.
        let schemas = &mut openapi
            .components
            .get_or_insert_with(openapiv3::Components::default)
            .schemas;
        generator.definitions().iter().for_each(|(key, schema)| {
            schemas.insert(key.clone(), j2oas_schema(None, schema));
        });

        openapi
    }

    /*
     * TODO-cleanup is there a way to make this available only within this
     * crate?  Once we do that, we don't need to consume the ApiDescription to
     * do this.
     */
    pub fn into_router(self) -> HttpRouter {
        self.router
    }
}

/**
 * Returns true iff the schema represents the null type i.e. the rust type `()`.
 */
fn is_null(schema: &schemars::schema::Schema) -> bool {
    if let schemars::schema::Schema::Object(schemars::schema::SchemaObject {
        instance_type: Some(schemars::schema::SingleOrVec::Single(it)),
        ..
    }) = schema
    {
        if let schemars::schema::InstanceType::Null = it.as_ref() {
            return true;
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
        schemars::schema::Schema::Bool(_) => todo!(),
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
            unimplemented!("unsupported by openapiv3")
        }
        None => None,
    };

    let kind = match (ty, &obj.subschemas) {
        (Some(schemars::schema::InstanceType::Null), None) => todo!(),
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
        (None, None) => todo!("missed a valid case {:?}", obj),
        _ => panic!("invalid"),
    };

    let mut data = openapiv3::SchemaData::default();

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
    match (&subschemas.all_of, &subschemas.any_of, &subschemas.one_of) {
        (Some(all_of), None, None) => openapiv3::SchemaKind::AllOf {
            all_of: all_of
                .iter()
                .map(|schema| j2oas_schema(None, schema))
                .collect::<Vec<_>>(),
        },
        (None, Some(any_of), None) => openapiv3::SchemaKind::AnyOf {
            any_of: any_of
                .iter()
                .map(|schema| j2oas_schema(None, schema))
                .collect::<Vec<_>>(),
        },
        (None, None, Some(one_of)) => openapiv3::SchemaKind::OneOf {
            one_of: one_of
                .iter()
                .map(|schema| j2oas_schema(None, schema))
                .collect::<Vec<_>>(),
        },
        (None, None, None) => todo!("missed a valid case"),
        _ => panic!("invalid"),
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
        .flat_map(|v| v.iter().map(|vv| vv.as_u64().unwrap() as i64))
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
        .flat_map(|v| v.iter().map(|vv| vv.as_f64().unwrap() as f64))
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
        .flat_map(|v| v.iter().map(|vv| vv.as_str().unwrap().to_string()))
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
                box_reference_or(j2oas_schema(None, &schema))
            }
            _ => unimplemented!("don't think this is valid"),
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
        openapiv3::ReferenceOr::Reference {
            reference,
        } => openapiv3::ReferenceOr::Reference {
            reference,
        },
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
                    |schema| {
                        openapiv3::AdditionalProperties::Schema(Box::new(
                            j2oas_schema(None, schema),
                        ))
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
pub struct OpenApiDefinition<'a> {
    api: &'a ApiDescription,
    info: openapiv3::Info,
}

impl<'a> OpenApiDefinition<'a> {
    fn new(
        api: &'a ApiDescription,
        title: &str,
        version: &str,
    ) -> OpenApiDefinition<'a> {
        let info = openapiv3::Info {
            title: title.to_string(),
            version: version.to_string(),
            ..Default::default()
        };
        OpenApiDefinition {
            api,
            info,
        }
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
                url: None,
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
        _: Arc<RequestContext>,
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
}
