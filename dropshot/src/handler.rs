// Copyright 2020 Oxide Computer Company
/*!
 * Interface for implementing HTTP endpoint handler functions.
 *
 * For information about supported endpoint function signatures, argument types,
 * extractors, and return types, see the top-level documentation dependencies: () for this crate.
 * As documented there, we support several different sets of function arguments
 * and return types.
 *
 * We allow for variation in the function arguments not so much for programmer
 * convenience (since parsing the query string or JSON body could be implemented
 * in a line or two of code each, with the right helper functions) but rather so
 * that the type signature of the handler function can be programmatically
 * analyzed to generate an OpenAPI snippet for this endpoint.  This approach of
 * treating the server implementation as the source of truth for the API
 * specification ensures that--at least in many important ways--the
 * implementation cannot diverge from the spec.
 *
 * Just like we want API input types to be represented in function arguments, we
 * want API response types to be represented in function return values so that
 * OpenAPI tooling can identify them at build time.  The more specific a type
 * returned by the handler function, the more can be validated at build-time,
 * and the more specific an OpenAPI schema can be generated from the source
 * alone.
 *
 * We go through considerable effort below to make this interface possible.
 * Both the interface (primarily) and the implementation (less so) are inspired
 * by Actix-Web.  The Actix implementation is significantly more general (and
 * commensurately complex).  It would be possible to implement richer facilities
 * here, like extractors for backend server state, headers, and so on; allowing
 * for server and request parameters to be omitted; and so on; but those other
 * facilities don't seem that valuable right now since they largely don't affect
 * OpenAPI document generation.
 */

use super::error::HttpError;
use super::http_util::http_extract_path_params;
use super::http_util::http_read_body;
use super::http_util::CONTENT_TYPE_JSON;
use super::http_util::CONTENT_TYPE_OCTET_STREAM;
use super::server::DropshotState;
use super::server::ServerContext;
use crate::api_description::ApiEndpointHeader;
use crate::api_description::ApiEndpointParameter;
use crate::api_description::ApiEndpointParameterLocation;
use crate::api_description::ApiEndpointResponse;
use crate::api_description::ApiSchemaGenerator;
use crate::api_description::{ApiEndpointBodyContentType, ExtensionMode};
use crate::pagination::PaginationParams;
use crate::pagination::PAGINATION_PARAM_SENTINEL;
use crate::router::VariableSet;
use crate::to_map::to_map;
use crate::websocket::WEBSOCKET_PARAM_SENTINEL;

use async_trait::async_trait;
use bytes::Bytes;
use futures::lock::Mutex;
use http::HeaderMap;
use http::StatusCode;
use hyper::Body;
use hyper::Request;
use hyper::Response;
use schemars::schema::InstanceType;
use schemars::schema::SchemaObject;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;
use slog::Logger;
use std::cmp::min;
use std::convert::TryFrom;
use std::fmt::Debug;
use std::fmt::Formatter;
use std::fmt::Result as FmtResult;
use std::future::Future;
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::sync::Arc;

/**
 * Type alias for the result returned by HTTP handler functions.
 */
pub type HttpHandlerResult = Result<Response<Body>, HttpError>;

/**
 * Handle for various interfaces useful during request processing.
 */
/*
 * TODO-cleanup What's the right way to package up "request"?  The only time we
 * need it to be mutable is when we're reading the body (e.g., as part of the
 * JSON extractor).  In order to support that, we wrap it in something that
 * supports interior mutability.  It also needs to be thread-safe, since we're
 * using async/await.  That brings us to Arc<Mutex<...>>, but it seems like
 * overkill since it will only really be used by one thread at a time (at all,
 * let alone mutably) and there will never be contention on the Mutex.
 */
#[derive(Debug)]
pub struct RequestContext<Context: ServerContext> {
    /** shared server state */
    pub server: Arc<DropshotState<Context>>,
    /** HTTP request details */
    pub request: Arc<Mutex<Request<Body>>>,
    /** HTTP request routing variables */
    pub path_variables: VariableSet,
    /** expected request body mime type */
    pub body_content_type: ApiEndpointBodyContentType,
    /** unique id assigned to this request */
    pub request_id: String,
    /** logger for this specific request */
    pub log: Logger,
}

impl<Context: ServerContext> RequestContext<Context> {
    /**
     * Returns the server context state.
     */
    pub fn context(&self) -> &Context {
        &self.server.private
    }

    /**
     * Returns the appropriate count of items to return for a paginated request
     *
     * This first looks at any client-requested limit and clamps it based on the
     * server-configured maximum page size.  If the client did not request any
     * particular limit, this function returns the server-configured default
     * page size.
     */
    pub fn page_limit<ScanParams, PageSelector>(
        &self,
        pag_params: &PaginationParams<ScanParams, PageSelector>,
    ) -> Result<NonZeroU32, HttpError>
    where
        ScanParams: DeserializeOwned,
        PageSelector: DeserializeOwned + Serialize,
    {
        let server_config = &self.server.config;

        Ok(pag_params
            .limit
            /*
             * Compare the client-provided limit to the configured max for the
             * server and take the smaller one.
             */
            .map(|limit| min(limit, server_config.page_max_nitems))
            /*
             * If no limit was provided by the client, use the configured
             * default.
             */
            .unwrap_or(server_config.page_default_nitems))
    }
}

/**
 * Helper trait for extracting the underlying Context type from the
 * first argument to an endpoint. This trait exists to help the
 * endpoint macro parse this argument.
 *
 * The first argument to an endpoint handler must be of the form:
 * `Arc<RequestContext<T>>` where `T` is a caller-supplied
 * value that implements `ServerContext`.
 */
pub trait RequestContextArgument {
    type Context;
}

impl<T: 'static + ServerContext> RequestContextArgument
    for Arc<RequestContext<T>>
{
    type Context = T;
}

/**
 * `Extractor` defines an interface allowing a type to be constructed from a
 * `RequestContext`.  Unlike most traits, `Extractor` essentially defines only a
 * constructor function, not instance functions.
 *
 * The extractors that we provide (`Query`, `Path`, `TypedBody`, `UntypedBody`, and
 * `WebsocketUpgrade`) implement `Extractor` in order to construct themselves from
 * the request. For example, `Extractor` is implemented for `Query<Q>` with a
 * function that reads the query string from the request, parses it, and
 * constructs a `Query<Q>` with it.
 *
 * We also define implementations of `Extractor` for tuples of types that
 * themselves implement `Extractor`.  See the implementation of
 * `HttpRouteHandler` for more on why this needed.
 */
#[async_trait]
pub trait Extractor: Send + Sync + Sized {
    /**
     * Construct an instance of this type from a `RequestContext`.
     */
    async fn from_request<Context: ServerContext>(
        rqctx: Arc<RequestContext<Context>>,
    ) -> Result<Self, HttpError>;

    fn metadata(
        body_content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata;
}

/**
 * Metadata associated with an extractor including parameters and whether or not
 * the associated endpoint is paginated.
 */
pub struct ExtractorMetadata {
    pub extension_mode: ExtensionMode,
    pub parameters: Vec<ApiEndpointParameter>,
}

/**
 * `impl_derived_for_tuple!` defines implementations of `Extractor` for tuples
 * whose elements themselves implement `Extractor`.
 */
macro_rules! impl_extractor_for_tuple {
    ($( $T:ident),*) => {
    #[async_trait]
    impl< $($T: Extractor + 'static,)* > Extractor for ($($T,)*)
    {
        async fn from_request<Context: ServerContext>(_rqctx: Arc<RequestContext<Context>>)
            -> Result<( $($T,)* ), HttpError>
        {
            futures::try_join!($($T::from_request(Arc::clone(&_rqctx)),)*)
        }

        fn metadata(_body_content_type: ApiEndpointBodyContentType) -> ExtractorMetadata {
            #[allow(unused_mut)]
            let mut extension_mode = ExtensionMode::None;
            #[allow(unused_mut)]
            let mut parameters = vec![];
            $(
                let mut metadata = $T::metadata(_body_content_type.clone());
                extension_mode = match (extension_mode, metadata.extension_mode) {
                    (ExtensionMode::None, x) | (x, ExtensionMode::None) => x,
                    (x, y) if x != y => {
                        panic!("incompatible extension modes in tuple: {:?} != {:?}", x, y);
                    }
                    (_, x) => x,
                };
                parameters.append(&mut metadata.parameters);
            )*
            ExtractorMetadata { extension_mode, parameters }
        }
    }
}}

impl_extractor_for_tuple!();
impl_extractor_for_tuple!(T1);
impl_extractor_for_tuple!(T1, T2);
impl_extractor_for_tuple!(T1, T2, T3);

/**
 * `HttpHandlerFunc` is a trait providing a single function, `handle_request()`,
 * which takes an HTTP request and produces an HTTP response (or
 * `HttpError`).
 *
 * As described above, handler functions can have a number of different
 * signatures.  They all consume a reference to the current request context.
 * They may also consume some number of extractor arguments.  The
 * `HttpHandlerFunc` trait is parametrized by the type `FuncParams`, which is
 * expected to be a tuple describing these extractor arguments.
 *
 * Below, we define implementations of `HttpHandlerFunc` for various function
 * types.  In this way, we can treat functions with different signatures as
 * different kinds of `HttpHandlerFunc`.  However, since the signature shows up
 * in the `FuncParams` type parameter, we'll need additional abstraction to
 * treat different handlers interchangeably.  See `RouteHandler` below.
 */
#[async_trait]
pub trait HttpHandlerFunc<Context, FuncParams, ResponseType>:
    Send + Sync + 'static
where
    Context: ServerContext,
    FuncParams: Extractor,
    ResponseType: HttpResponse + Send + Sync + 'static,
{
    async fn handle_request(
        &self,
        rqctx: Arc<RequestContext<Context>>,
        p: FuncParams,
    ) -> HttpHandlerResult;
}

/**
 * Defines an implementation of the `HttpHandlerFunc` trait for functions
 * matching one of the supported signatures for HTTP endpoint handler functions.
 * We use a macro to do this because we need to provide different
 * implementations for functions that take 0 arguments, 1 argument, 2 arguments,
 * etc., but the implementations are almost identical.
 */
/*
 * For background: as the module-level documentation explains, we want to
 * support API endpoint handler functions that vary in their signature so that
 * the signature can accurately reflect details about their expected input and
 * output instead of a generic `Request -> Response` description.  The
 * `HttpHandlerFunc` trait defines an interface for invoking one of these
 * functions.  This macro defines an implementation of `HttpHandlerFunc` that
 * says how to take any of these HTTP endpoint handler function and provide that
 * uniform interface for callers.  The implementation essentially does three
 * things:
 *
 * 1. Converts the uniform arguments of `handle_request()` into the appropriate
 *    arguments for the underlying function.  This is easier than it sounds at
 *    this point because we require that one of the arguments be a tuple whose
 *    types correspond to the argument types for the function, so we just need
 *    to unpack them from the tuple into function arguments.
 *
 * 2. Converts a call to the `handle_request()` method into a call to the
 *    underlying function.
 *
 * 3. Converts the return type of the underlying function into the uniform
 *    return type expected by callers of `handle_request()`.  This, too, is
 *    easier than it sounds because we require that the return value implement
 *    `HttpResponse`.
 *
 * As mentioned above, we're implementing the trait `HttpHandlerFunc` on _any_
 * type `FuncType` that matches the trait bounds below.  In particular, it must
 * take a request context argument and whatever other type parameters have been
 * passed to this macro.
 *
 * The function's return type deserves further explanation.  (Actually, these
 * functions all return a `Future`, but for convenience when we say "return
 * type" in the comments here we're referring to the output type of the returned
 * future.)  Again, as described above, we'd like to allow HTTP endpoint
 * functions to return a variety of different return types that are ultimately
 * converted into `Result<Response<Body>, HttpError>`.  To do that, the trait
 * bounds below say that the function must produce a `Result<ResponseType,
 * HttpError>` where `ResponseType` is a type that implements `HttpResponse`.
 * We provide a few implementations of the trait `HttpTypedResponse` that
 * includes a HTTP status code and structured output. In addition we allow for
 * functions to hand-craft a `Response<Body>`. For both we implement
 * `HttpResponse` (trivially in the latter case).
 *
 *      1. Handler function
 *            |
 *            | returns:
 *            v
 *      2. Result<ResponseType, HttpError>
 *            |
 *            | This may fail with an HttpError which we return immediately.
 *            | On success, this will be Ok(ResponseType) for some specific
 *            | ResponseType that implements HttpResponse.  We'll end up
 *            | invoking:
 *            v
 *      3. ResponseType::to_result()
 *            |
 *            | This is a type-specific conversion from `ResponseType` into
 *            | `Response<Body>` that's allowed to fail with an `HttpError`.
 *            v
 *      4. Result<Response<Body>, HttpError>
 *
 * Note that the handler function may fail due to an internal error *or* the
 * conversion to JSON may successively fail in the call to
 * `serde_json::to_string()`.
 *
 * The `HttpResponse` trait lets us handle both generic responses via
 * `Response<Body>` as well as more structured responses via structures
 * implementing `HttpResponse<Body = Type>`. The latter gives us a typed
 * structure as well as response code that we use to generate rich OpenAPI
 * content.
 *
 * Note: the macro parameters really ought to be `$i:literal` and `$T:ident`,
 * however that causes us to run afoul of issue dtolnay/async-trait#46. The
 * workaround is to make both parameters `tt` (token tree).
 */
macro_rules! impl_HttpHandlerFunc_for_func_with_params {
    ($(($i:tt, $T:tt)),*) => {

    #[async_trait]
    impl<Context, FuncType, FutureType, ResponseType, $($T,)*>
        HttpHandlerFunc<Context, ($($T,)*), ResponseType> for FuncType
    where
        Context: ServerContext,
        FuncType: Fn(Arc<RequestContext<Context>>, $($T,)*)
            -> FutureType + Send + Sync + 'static,
        FutureType: Future<Output = Result<ResponseType, HttpError>>
            + Send + 'static,
        ResponseType: HttpResponse + Send + Sync + 'static,
        $($T: Extractor + Send + Sync + 'static,)*
    {
        async fn handle_request(
            &self,
            rqctx: Arc<RequestContext<Context>>,
            _param_tuple: ($($T,)*)
        ) -> HttpHandlerResult
        {
            let response: ResponseType =
                (self)(rqctx, $(_param_tuple.$i,)*).await?;
            response.to_result()
        }
    }
}}

impl_HttpHandlerFunc_for_func_with_params!();
impl_HttpHandlerFunc_for_func_with_params!((0, T0));
impl_HttpHandlerFunc_for_func_with_params!((0, T1), (1, T2));
impl_HttpHandlerFunc_for_func_with_params!((0, T1), (1, T2), (2, T3));

/**
 * `RouteHandler` abstracts an `HttpHandlerFunc<FuncParams, ResponseType>` in a
 * way that allows callers to invoke the handler without knowing the handler's
 * function signature.
 *
 * The "Route" in `RouteHandler` refers to the fact that this structure is used
 * to record that a specific handler has been attached to a specific HTTP route.
 */
#[async_trait]
pub trait RouteHandler<Context: ServerContext>: Debug + Send + Sync {
    /**
     * Returns a description of this handler.  This might be a function name,
     * for example.  This is not guaranteed to be unique.
     */
    fn label(&self) -> &str;

    /**
     * Handle an incoming HTTP request.
     */
    async fn handle_request(
        &self,
        rqctx: RequestContext<Context>,
    ) -> HttpHandlerResult;
}

/**
 * `HttpRouteHandler` is the only type that implements `RouteHandler`.  The
 * reason both exist is that we need `HttpRouteHandler::new()` to consume an
 * arbitrary kind of `HttpHandlerFunc<FuncParams>` and return an object that's
 * _not_ parametrized by `FuncParams`.  In fact, the resulting
 * `HttpRouteHandler` _is_ parametrized by `FuncParams`, but we returned it
 * as a `RouteHandler` that does not have those type parameters, allowing the
 * caller to ignore the differences between different handler function type
 * signatures.
 */
pub struct HttpRouteHandler<Context, HandlerType, FuncParams, ResponseType>
where
    Context: ServerContext,
    HandlerType: HttpHandlerFunc<Context, FuncParams, ResponseType>,
    FuncParams: Extractor,
    ResponseType: HttpResponse + Send + Sync + 'static,
{
    /** the actual HttpHandlerFunc used to implement this route */
    handler: HandlerType,

    /** debugging label for the handler */
    label: String,

    /**
     * In order to define `new()` below, we need a type parameter `HandlerType`
     * that implements `HttpHandlerFunc<FuncParams>`, which means we also need a
     * `FuncParams` type parameter.  However, this type parameter would be
     * unconstrained, which makes Rust upset.  Use of PhantomData<FuncParams>
     * here causes the compiler to behave as though this struct referred to a
     * `FuncParams`, which allows us to use the type parameter below.
     */
    phantom: PhantomData<(FuncParams, ResponseType, Context)>,
}

impl<Context, HandlerType, FuncParams, ResponseType> Debug
    for HttpRouteHandler<Context, HandlerType, FuncParams, ResponseType>
where
    Context: ServerContext,
    HandlerType: HttpHandlerFunc<Context, FuncParams, ResponseType>,
    FuncParams: Extractor,
    ResponseType: HttpResponse + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "handler: {}", self.label)
    }
}

#[async_trait]
impl<Context, HandlerType, FuncParams, ResponseType> RouteHandler<Context>
    for HttpRouteHandler<Context, HandlerType, FuncParams, ResponseType>
where
    Context: ServerContext,
    HandlerType: HttpHandlerFunc<Context, FuncParams, ResponseType>,
    FuncParams: Extractor + 'static,
    ResponseType: HttpResponse + Send + Sync + 'static,
{
    fn label(&self) -> &str {
        &self.label
    }

    async fn handle_request(
        &self,
        rqctx_raw: RequestContext<Context>,
    ) -> HttpHandlerResult {
        /*
         * This is where the magic happens: in the code below, `funcparams` has
         * type `FuncParams`, which is a tuple type describing the extractor
         * arguments to the handler function.  This could be `()`, `(Query<Q>)`,
         * `(TypedBody<J>)`, `(Query<Q>, TypedBody<J>)`, or any other
         * combination of extractors we decide to support in the future.
         * Whatever it is must implement `Extractor`, which means we can invoke
         * `Extractor::from_request()` to construct the argument tuple,
         * generally from information available in the `request` object.  We
         * pass this down to the `HttpHandlerFunc`, for which there's a
         * different implementation for each value of `FuncParams`.  The
         * `HttpHandlerFunc` for each `FuncParams` just pulls the arguments out
         * of the `funcparams` tuple and makes them actual function arguments
         * for the actual handler function.  From this point down, all of this
         * is resolved statically.makes them actual function arguments for the
         * actual handler function.  From this point down, all of this is
         * resolved statically.
         */
        let rqctx = Arc::new(rqctx_raw);
        let funcparams = Extractor::from_request(Arc::clone(&rqctx)).await?;
        let future = self.handler.handle_request(rqctx, funcparams);
        future.await
    }
}

/*
 * Public interfaces
 */

impl<Context, HandlerType, FuncParams, ResponseType>
    HttpRouteHandler<Context, HandlerType, FuncParams, ResponseType>
where
    Context: ServerContext,
    HandlerType: HttpHandlerFunc<Context, FuncParams, ResponseType>,
    FuncParams: Extractor + 'static,
    ResponseType: HttpResponse + Send + Sync + 'static,
{
    /**
     * Given a function matching one of the supported API handler function
     * signatures, return a RouteHandler that can be used to respond to HTTP
     * requests using this function.
     */
    pub fn new(handler: HandlerType) -> Box<dyn RouteHandler<Context>> {
        HttpRouteHandler::new_with_name(handler, "<unlabeled handler>")
    }

    /**
     * Given a function matching one of the supported API handler function
     * signatures, return a RouteHandler that can be used to respond to HTTP
     * requests using this function.
     */
    pub fn new_with_name(
        handler: HandlerType,
        label: &str,
    ) -> Box<dyn RouteHandler<Context>> {
        Box::new(HttpRouteHandler {
            label: label.to_string(),
            handler,
            phantom: PhantomData,
        })
    }
}

/*
 * Extractors
 */

/*
 * Query: query string extractor
 */

/**
 * `Query<QueryType>` is an extractor used to deserialize an instance of
 * `QueryType` from an HTTP request's query string.  `QueryType` is any
 * structure of yours that implements `serde::Deserialize`.  See this module's
 * documentation for more information.
 */
#[derive(Debug)]
pub struct Query<QueryType: DeserializeOwned + JsonSchema + Send + Sync> {
    inner: QueryType,
}

impl<QueryType: DeserializeOwned + JsonSchema + Send + Sync> Query<QueryType> {
    /*
     * TODO drop this in favor of Deref?  + Display and Debug for convenience?
     */
    pub fn into_inner(self) -> QueryType {
        self.inner
    }
}

/**
 * Given an HTTP request, pull out the query string and attempt to deserialize
 * it as an instance of `QueryType`.
 */
fn http_request_load_query<QueryType>(
    request: &Request<Body>,
) -> Result<Query<QueryType>, HttpError>
where
    QueryType: DeserializeOwned + JsonSchema + Send + Sync,
{
    let raw_query_string = request.uri().query().unwrap_or("");
    /*
     * TODO-correctness: are query strings defined to be urlencoded in this way?
     */
    match serde_urlencoded::from_str(raw_query_string) {
        Ok(q) => Ok(Query { inner: q }),
        Err(e) => Err(HttpError::for_bad_request(
            None,
            format!("unable to parse query string: {}", e),
        )),
    }
}

/*
 * The `Extractor` implementation for Query<QueryType> describes how to construct
 * an instance of `Query<QueryType>` from an HTTP request: namely, by parsing
 * the query string to an instance of `QueryType`.
 * TODO-cleanup We shouldn't have to use the "'static" bound on `QueryType`
 * here.  It seems like we ought to be able to use 'async_trait, but that
 * doesn't seem to be defined.
 */
#[async_trait]
impl<QueryType> Extractor for Query<QueryType>
where
    QueryType: JsonSchema + DeserializeOwned + Send + Sync + 'static,
{
    async fn from_request<Context: ServerContext>(
        rqctx: Arc<RequestContext<Context>>,
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

/*
 * Path: path parameter string extractor
 */

/**
 * `Path<PathType>` is an extractor used to deserialize an instance of
 * `PathType` from an HTTP request's path parameters.  `PathType` is any
 * structure of yours that implements `serde::Deserialize`.  See this module's
 * documentation for more information.
 */
#[derive(Debug)]
pub struct Path<PathType: JsonSchema + Send + Sync> {
    inner: PathType,
}

impl<PathType: JsonSchema + Send + Sync> Path<PathType> {
    /*
     * TODO drop this in favor of Deref?  + Display and Debug for convenience?
     */
    pub fn into_inner(self) -> PathType {
        self.inner
    }
}

/*
 * The `Extractor` implementation for Path<PathType> describes how to construct
 * an instance of `Path<QueryType>` from an HTTP request: namely, by extracting
 * parameters from the query string.
 */
#[async_trait]
impl<PathType> Extractor for Path<PathType>
where
    PathType: DeserializeOwned + JsonSchema + Send + Sync + 'static,
{
    async fn from_request<Context: ServerContext>(
        rqctx: Arc<RequestContext<Context>>,
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

/**
 * Convenience function to generate parameter metadata from types implementing
 * `JsonSchema` for use with `Query` and `Path` `Extractors`.
 */
fn get_metadata<ParamType>(
    loc: &ApiEndpointParameterLocation,
) -> ExtractorMetadata
where
    ParamType: JsonSchema,
{
    /*
     * Generate the type for `ParamType` then pluck out each member of
     * the structure to encode as an individual parameter.
     */
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

    /*
     * Convert our collection of struct members list of parameters.
     */
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

fn schema_extensions(
    schema: &schemars::schema::Schema,
) -> Option<&schemars::Map<String, serde_json::Value>> {
    match schema {
        schemars::schema::Schema::Bool(_) => None,
        schemars::schema::Schema::Object(object) => Some(&object.extensions),
    }
}

/**
 * Used to visit all schemas and collect all dependencies.
 */
struct ReferenceVisitor<'a> {
    generator: &'a schemars::gen::SchemaGenerator,
    dependencies: indexmap::IndexMap<String, schemars::schema::Schema>,
}

impl<'a> ReferenceVisitor<'a> {
    fn new(generator: &'a schemars::gen::SchemaGenerator) -> Self {
        Self { generator, dependencies: indexmap::IndexMap::new() }
    }

    fn dependencies(
        self,
    ) -> indexmap::IndexMap<String, schemars::schema::Schema> {
        self.dependencies
    }
}

impl<'a> schemars::visit::Visitor for ReferenceVisitor<'a> {
    fn visit_schema_object(&mut self, schema: &mut SchemaObject) {
        if let Some(refstr) = &schema.reference {
            let definitions_path = &self.generator.settings().definitions_path;
            let name = &refstr[definitions_path.len()..];

            if !self.dependencies.contains_key(name) {
                let mut refschema = self
                    .generator
                    .definitions()
                    .get(name)
                    .expect("invalid reference")
                    .clone();
                self.dependencies.insert(
                    name.to_string(),
                    schemars::schema::Schema::Bool(false),
                );
                schemars::visit::visit_schema(self, &mut refschema);
                self.dependencies.insert(name.to_string(), refschema);
            }
        }

        schemars::visit::visit_schema_object(self, schema);
    }
}

#[derive(Debug)]
pub(crate) struct StructMember {
    pub name: String,
    pub description: Option<String>,
    pub schema: schemars::schema::Schema,
    pub required: bool,
}

/**
 * This helper function produces a list of the structure members for the
 * given schema. For each it returns:
 *   (name: &String, schema: &Schema, required: bool)
 *
 * If the input schema is not a flat structure the result will be a runtime
 * failure reflective of a programming error (likely an invalid type specified
 * in a handler function).
 *
 * This function is invoked recursively on subschemas.
 */
pub(crate) fn schema2struct(
    schema: &schemars::schema::Schema,
    generator: &schemars::gen::SchemaGenerator,
    required: bool,
) -> Vec<StructMember> {
    /*
     * We ignore schema.metadata, which includes things like doc comments, and
     * schema.extensions. We call these out explicitly rather than eliding them
     * as .. since we match all other fields in the structure.
     */
    match schema {
        /* We expect references to be on their own. */
        schemars::schema::Schema::Object(schemars::schema::SchemaObject {
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
            reference: Some(_),
            extensions: _,
        }) => schema2struct(
            generator.dereference(schema).expect("invalid reference"),
            generator,
            required,
        ),

        /* Match objects and subschemas. */
        schemars::schema::Schema::Object(schemars::schema::SchemaObject {
            metadata: _,
            instance_type: Some(schemars::schema::SingleOrVec::Single(_)),
            format: None,
            enum_values: None,
            const_value: None,
            subschemas,
            number: None,
            string: None,
            array: None,
            object,
            reference: None,
            extensions: _,
        }) => {
            let mut results = Vec::new();

            /*
             * If there's a top-level object, add its members to the list of
             * parameters.
             */
            if let Some(object) = object {
                results.extend(object.properties.iter().map(
                    |(name, schema)| {
                        let (description, schema) =
                            schema_extract_description(schema);
                        StructMember {
                            name: name.clone(),
                            description,
                            schema,
                            required: required
                                && object.required.contains(name),
                        }
                    },
                ));
            }

            /*
             * We might see subschemas here in the case of flattened enums
             * or flattened structures that have associated doc comments.
             */
            if let Some(subschemas) = subschemas {
                match subschemas.as_ref() {
                    /* We expect any_of in the case of an enum. */
                    schemars::schema::SubschemaValidation {
                        all_of: None,
                        any_of: Some(schemas),
                        one_of: None,
                        not: None,
                        if_schema: None,
                        then_schema: None,
                        else_schema: None,
                    } => results.extend(schemas.iter().flat_map(|subschema| {
                        /* Note that these will be tagged as optional. */
                        schema2struct(subschema, generator, false)
                    })),

                    /*
                     * With an all_of, there should be a single element. We
                     * typically see this in the case where there is a doc
                     * comment on a structure as OpenAPI 3.0.x doesn't have
                     * a description field directly on schemas.
                     */
                    schemars::schema::SubschemaValidation {
                        all_of: Some(subschemas),
                        any_of: None,
                        one_of: None,
                        not: None,
                        if_schema: None,
                        then_schema: None,
                        else_schema: None,
                    } if subschemas.len() == 1 => results.extend(
                        subschemas.iter().flat_map(|subschema| {
                            schema2struct(subschema, generator, required)
                        }),
                    ),

                    /* We don't expect any other types of subschemas. */
                    invalid => panic!("invalid subschema {:#?}", invalid),
                }
            }

            results
        }

        /* The generated schema should be an object. */
        invalid => panic!("invalid type {:#?}", invalid),
    }
}

/*
 * TypedBody: body extractor for formats that can be deserialized to a specific
 * type.  Only JSON is currently supported.
 */

/**
 * `TypedBody<BodyType>` is an extractor used to deserialize an instance of
 * `BodyType` from an HTTP request body.  `BodyType` is any structure of yours
 * that implements `serde::Deserialize`.  See this module's documentation for
 * more information.
 */
#[derive(Debug)]
pub struct TypedBody<BodyType: JsonSchema + DeserializeOwned + Send + Sync> {
    inner: BodyType,
}

impl<BodyType: JsonSchema + DeserializeOwned + Send + Sync>
    TypedBody<BodyType>
{
    /*
     * TODO drop this in favor of Deref?  + Display and Debug for convenience?
     */
    pub fn into_inner(self) -> BodyType {
        self.inner
    }
}

/**
 * Given an HTTP request, attempt to read the body, parse it according
 * to the content type, and deserialize it to an instance of `BodyType`.
 */
async fn http_request_load_body<Context: ServerContext, BodyType>(
    rqctx: Arc<RequestContext<Context>>,
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

/*
 * The `Extractor` implementation for TypedBody<BodyType> describes how to
 * construct an instance of `TypedBody<BodyType>` from an HTTP request: namely,
 * by reading the request body and parsing it as JSON into type `BodyType`.
 * TODO-cleanup We shouldn't have to use the "'static" bound on `BodyType` here.
 * It seems like we ought to be able to use 'async_trait, but that doesn't seem
 * to be defined.
 */
#[async_trait]
impl<BodyType> Extractor for TypedBody<BodyType>
where
    BodyType: JsonSchema + DeserializeOwned + Send + Sync + 'static,
{
    async fn from_request<Context: ServerContext>(
        rqctx: Arc<RequestContext<Context>>,
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

/*
 * UntypedBody: body extractor for a plain array of bytes of a body.
 */

/**
 * `UntypedBody` is an extractor for reading in the contents of the HTTP request
 * body and making the raw bytes directly available to the consumer.
 */
#[derive(Debug)]
pub struct UntypedBody {
    content: Bytes,
}

impl UntypedBody {
    /**
     * Returns a byte slice of the underlying body content.
     */
    /*
     * TODO drop this in favor of Deref?  + Display and Debug for convenience?
     */
    pub fn as_bytes(&self) -> &[u8] {
        &self.content
    }

    /**
     * Convenience wrapper to convert the body to a UTF-8 string slice,
     * returning a 400-level error if the body is not valid UTF-8.
     */
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
impl Extractor for UntypedBody {
    async fn from_request<Context: ServerContext>(
        rqctx: Arc<RequestContext<Context>>,
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

/*
 * Response Type Conversion
 *
 * See the discussion on macro `impl_HttpHandlerFunc_for_func_with_params` for a
 * great deal of context on this.
 */

/**
 * HttpResponse must produce a `Result<Response<Body>, HttpError>` and generate
 * the response metadata.  Typically one should use `Response<Body>` or an
 * implementation of `HttpTypedResponse`.
 */
pub trait HttpResponse {
    /**
     * Generate the response to the HTTP call.
     */
    fn to_result(self) -> HttpHandlerResult;

    /**
     * Extract status code and structure metadata for the non-error response.
     * Type information for errors is handled generically across all endpoints.
     */
    fn response_metadata() -> ApiEndpointResponse;
}

/**
 * `Response<Body>` is used for free-form responses. The implementation of
 * `to_result()` is trivial, and we don't have any typed metadata to return.
 */
impl HttpResponse for Response<Body> {
    fn to_result(self) -> HttpHandlerResult {
        Ok(self)
    }
    fn response_metadata() -> ApiEndpointResponse {
        ApiEndpointResponse::default()
    }
}

/// Wraps a [hyper::Body] so that it can be used with coded response types such
/// as [HttpResponseOk].
pub struct FreeformBody(pub Body);

impl From<Body> for FreeformBody {
    fn from(body: Body) -> Self {
        Self(body)
    }
}

/**
 * An "empty" type used to represent responses that have no associated data
 * payload. This isn't intended for general use, but must be pub since it's
 * used as the Body type for certain responses.
 */
#[doc(hidden)]
pub struct Empty;

/*
 * Specific Response Types
 *
 * The `HttpTypedResponse` trait and the concrete types below are provided so
 * that handler functions can return types that indicate at compile time the
 * kind of HTTP response body they produce.
 */

/// Adapter trait that allows both concrete types that implement [JsonSchema]
/// and the [FreeformBody] type to add their content to a response builder
/// object.
pub trait HttpResponseContent {
    fn to_response(self, builder: http::response::Builder)
        -> HttpHandlerResult;

    // TODO the return type here could be something more elegant that is able
    // to produce the map of mime type -> openapiv3::MediaType that's needed in
    // in api_description. One could imagine, for example, that this could
    // allow dropshot consumers in the future to have endpoints that respond
    // with multiple, explicitly enumerated mime types.
    // TODO the ApiSchemaGenerator type is particularly inelegant.
    fn content_metadata() -> Option<ApiSchemaGenerator>;
}

impl HttpResponseContent for FreeformBody {
    fn to_response(
        self,
        builder: http::response::Builder,
    ) -> HttpHandlerResult {
        Ok(builder
            .header(http::header::CONTENT_TYPE, CONTENT_TYPE_OCTET_STREAM)
            .body(self.0)?)
    }

    fn content_metadata() -> Option<ApiSchemaGenerator> {
        None
    }
}

impl HttpResponseContent for Empty {
    fn to_response(
        self,
        builder: http::response::Builder,
    ) -> HttpHandlerResult {
        Ok(builder.body(Body::empty())?)
    }

    fn content_metadata() -> Option<ApiSchemaGenerator> {
        Some(ApiSchemaGenerator::Static {
            schema: Box::new(schemars::schema::Schema::Bool(false)),
            dependencies: indexmap::IndexMap::default(),
        })
    }
}

impl<T> HttpResponseContent for T
where
    T: JsonSchema + Serialize + Send + Sync + 'static,
{
    fn to_response(
        self,
        builder: http::response::Builder,
    ) -> HttpHandlerResult {
        let serialized = serde_json::to_string(&self)
            .map_err(|e| HttpError::for_internal_error(e.to_string()))?;
        Ok(builder
            .header(http::header::CONTENT_TYPE, CONTENT_TYPE_JSON)
            .body(serialized.into())?)
    }

    fn content_metadata() -> Option<ApiSchemaGenerator> {
        Some(ApiSchemaGenerator::Gen {
            name: Self::schema_name,
            schema: make_subschema_for::<Self>,
        })
    }
}

/**
 * The `HttpCodedResponse` trait is used for all of the specific response types
 * that we provide. We use it in particular to encode the success status code
 * and the type information of the return value.
 */
pub trait HttpCodedResponse:
    Into<HttpHandlerResult> + Send + Sync + 'static
{
    type Body: HttpResponseContent;
    const STATUS_CODE: StatusCode;
    const DESCRIPTION: &'static str;

    /**
     * Convenience method to produce a response based on the input
     * `body_object` (whose specific type is defined by the implementing type)
     * and the STATUS_CODE specified by the implementing type. This is a default
     * trait method to allow callers to avoid redundant type specification.
     */
    fn for_object(body: Self::Body) -> HttpHandlerResult {
        body.to_response(Response::builder().status(Self::STATUS_CODE))
    }
}

/**
 * Provide results and metadata generation for all implementing types.
 */
impl<T> HttpResponse for T
where
    T: HttpCodedResponse,
{
    fn to_result(self) -> HttpHandlerResult {
        self.into()
    }
    fn response_metadata() -> ApiEndpointResponse {
        ApiEndpointResponse {
            schema: T::Body::content_metadata(),
            success: Some(T::STATUS_CODE),
            description: Some(T::DESCRIPTION.to_string()),
            ..Default::default()
        }
    }
}

fn make_subschema_for<T: JsonSchema>(
    gen: &mut schemars::gen::SchemaGenerator,
) -> schemars::schema::Schema {
    gen.subschema_for::<T>()
}

/**
 * `HttpResponseCreated<T: Serialize>` wraps an object of any serializable type.
 * It denotes an HTTP 201 "Created" response whose body is generated by
 * serializing the object.
 */
/*
 * TODO-cleanup should ApiObject move into this submodule?  It'd be nice if we
 * could restrict this to an ApiObject::View (by having T: ApiObject and the
 * field having type T::View).
 */
pub struct HttpResponseCreated<T: HttpResponseContent + Send + Sync + 'static>(
    pub T,
);
impl<T: HttpResponseContent + Send + Sync + 'static> HttpCodedResponse
    for HttpResponseCreated<T>
{
    type Body = T;
    const STATUS_CODE: StatusCode = StatusCode::CREATED;
    const DESCRIPTION: &'static str = "successful creation";
}
impl<T: HttpResponseContent + Send + Sync + 'static>
    From<HttpResponseCreated<T>> for HttpHandlerResult
{
    fn from(response: HttpResponseCreated<T>) -> HttpHandlerResult {
        /* TODO-correctness (or polish?): add Location header */
        HttpResponseCreated::for_object(response.0)
    }
}

/**
 * `HttpResponseAccepted<T: Serialize>` wraps an object of any
 * serializable type.  It denotes an HTTP 202 "Accepted" response whose body is
 * generated by serializing the object.
 */
pub struct HttpResponseAccepted<T: HttpResponseContent + Send + Sync + 'static>(
    pub T,
);
impl<T: HttpResponseContent + Send + Sync + 'static> HttpCodedResponse
    for HttpResponseAccepted<T>
{
    type Body = T;
    const STATUS_CODE: StatusCode = StatusCode::ACCEPTED;
    const DESCRIPTION: &'static str = "successfully enqueued operation";
}
impl<T: HttpResponseContent + Send + Sync + 'static>
    From<HttpResponseAccepted<T>> for HttpHandlerResult
{
    fn from(response: HttpResponseAccepted<T>) -> HttpHandlerResult {
        HttpResponseAccepted::for_object(response.0)
    }
}

/**
 * `HttpResponseOk<T: Serialize>` wraps an object of any serializable type.  It
 * denotes an HTTP 200 "OK" response whose body is generated by serializing the
 * object.
 */
pub struct HttpResponseOk<T: HttpResponseContent + Send + Sync + 'static>(
    pub T,
);
impl<T: HttpResponseContent + Send + Sync + 'static> HttpCodedResponse
    for HttpResponseOk<T>
{
    type Body = T;
    const STATUS_CODE: StatusCode = StatusCode::OK;
    const DESCRIPTION: &'static str = "successful operation";
}
impl<T: HttpResponseContent + Send + Sync + 'static> From<HttpResponseOk<T>>
    for HttpHandlerResult
{
    fn from(response: HttpResponseOk<T>) -> HttpHandlerResult {
        HttpResponseOk::for_object(response.0)
    }
}

/**
 * `HttpResponseDeleted` represents an HTTP 204 "No Content" response, intended
 * for use when an API operation has successfully deleted an object.
 */
pub struct HttpResponseDeleted();

impl HttpCodedResponse for HttpResponseDeleted {
    type Body = Empty;
    const STATUS_CODE: StatusCode = StatusCode::NO_CONTENT;
    const DESCRIPTION: &'static str = "successful deletion";
}
impl From<HttpResponseDeleted> for HttpHandlerResult {
    fn from(_: HttpResponseDeleted) -> HttpHandlerResult {
        HttpResponseDeleted::for_object(Empty)
    }
}

/**
 * `HttpResponseUpdatedNoContent` represents an HTTP 204 "No Content" response,
 * intended for use when an API operation has successfully updated an object and
 * has nothing to return.
 */
pub struct HttpResponseUpdatedNoContent();

impl HttpCodedResponse for HttpResponseUpdatedNoContent {
    type Body = Empty;
    const STATUS_CODE: StatusCode = StatusCode::NO_CONTENT;
    const DESCRIPTION: &'static str = "resource updated";
}
impl From<HttpResponseUpdatedNoContent> for HttpHandlerResult {
    fn from(_: HttpResponseUpdatedNoContent) -> HttpHandlerResult {
        HttpResponseUpdatedNoContent::for_object(Empty)
    }
}

#[derive(Serialize, JsonSchema)]
pub struct NoHeaders {}

/**
 * `HttpResponseHeaders` is a wrapper for responses that include both
 * structured and unstructured headers. The first type parameter is a
 * `HttpTypedResponse` that provides the structure of the response body.
 * The second type parameter is an optional struct that enumerates named
 * headers that are included in the response. In addition to those (optional)
 * named headers, consumers may add additional headers via the `headers_mut`
 * interface. Unnamed headers override named headers in the case of naming
 * conflicts.
 */
pub struct HttpResponseHeaders<
    T: HttpCodedResponse,
    H: JsonSchema + Serialize + Send + Sync + 'static = NoHeaders,
> {
    body: T,
    structured_headers: H,
    other_headers: HeaderMap,
}
impl<T: HttpCodedResponse> HttpResponseHeaders<T, NoHeaders> {
    pub fn new_unnamed(body: T) -> Self {
        Self {
            body,
            structured_headers: NoHeaders {},
            other_headers: HeaderMap::default(),
        }
    }
}
impl<
        T: HttpCodedResponse,
        H: JsonSchema + Serialize + Send + Sync + 'static,
    > HttpResponseHeaders<T, H>
{
    pub fn new(body: T, headers: H) -> Self {
        Self {
            body,
            structured_headers: headers,
            other_headers: HeaderMap::default(),
        }
    }

    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.other_headers
    }
}
impl<
        T: HttpCodedResponse,
        H: JsonSchema + Serialize + Send + Sync + 'static,
    > HttpResponse for HttpResponseHeaders<T, H>
{
    fn to_result(self) -> HttpHandlerResult {
        let HttpResponseHeaders { body, structured_headers, other_headers } =
            self;
        /* Compute the body. */
        let mut result = body.into()?;
        /* Add in both the structured and other headers. */
        let headers = result.headers_mut();
        let header_map = to_map(&structured_headers).map_err(|e| {
            HttpError::for_internal_error(format!(
                "error processing headers: {}",
                e.0
            ))
        })?;

        for (key, value) in header_map {
            let key = http::header::HeaderName::try_from(key)
                .map_err(|e| HttpError::for_internal_error(e.to_string()))?;
            let value = http::header::HeaderValue::try_from(value)
                .map_err(|e| HttpError::for_internal_error(e.to_string()))?;
            headers.insert(key, value);
        }

        headers.extend(other_headers);

        Ok(result)
    }

    fn response_metadata() -> ApiEndpointResponse {
        let mut metadata = T::response_metadata();

        let mut generator = schemars::gen::SchemaGenerator::new(
            schemars::gen::SchemaSettings::openapi3(),
        );
        let schema = generator.root_schema_for::<H>().schema.into();

        let headers = schema2struct(&schema, &generator, true)
            .into_iter()
            .map(|struct_member| {
                let mut s = struct_member.schema;
                let mut visitor = ReferenceVisitor::new(&generator);
                schemars::visit::visit_schema(&mut visitor, &mut s);
                ApiEndpointHeader {
                    name: struct_member.name,
                    description: struct_member.description,
                    schema: ApiSchemaGenerator::Static {
                        schema: Box::new(s),
                        dependencies: visitor.dependencies(),
                    },
                    required: struct_member.required,
                }
            })
            .collect::<Vec<_>>();

        metadata.headers = headers;
        metadata
    }
}

fn schema_extract_description(
    schema: &schemars::schema::Schema,
) -> (Option<String>, schemars::schema::Schema) {
    /*
     * Because the OpenAPI v3.0.x Schema cannot include a description with
     * a reference, we may see a schema with a description and an `all_of`
     * with a single subschema. In this case, we flatten the trivial subschema.
     */
    if let schemars::schema::Schema::Object(schemars::schema::SchemaObject {
        metadata,
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
            all_of: Some(subschemas),
            any_of: None,
            one_of: None,
            not: None,
            if_schema: None,
            then_schema: None,
            else_schema: None,
        } = subschemas.as_ref()
        {
            match (subschemas.first(), subschemas.len()) {
                (Some(subschema), 1) => {
                    let description = metadata
                        .as_ref()
                        .and_then(|m| m.as_ref().description.clone());
                    return (description, subschema.clone());
                }
                _ => (),
            }
        }
    }

    match schema {
        schemars::schema::Schema::Bool(_) => (None, schema.clone()),

        schemars::schema::Schema::Object(object) => {
            let description = object
                .metadata
                .as_ref()
                .and_then(|m| m.as_ref().description.clone());
            (
                description,
                schemars::schema::SchemaObject {
                    metadata: None,
                    ..object.clone()
                }
                .into(),
            )
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

        /*
         * This is order-dependent. We might not really care if the order
         * changes, but it will be interesting to understand why if it does.
         */
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
