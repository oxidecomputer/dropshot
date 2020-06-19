/*!
 * Interface for implementing HTTP endpoint handler functions.
 *
 * For information about supported endpoint function signatures, argument types,
 * extractors, and return types, see the top-level documentation for this crate.
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
use super::http_util::CONTENT_TYPE_NDJSON;
use super::server::DropshotState;
use crate::api_description::ApiEndpointParameter;
use crate::api_description::ApiEndpointParameterLocation;
use crate::api_description::ApiEndpointParameterName;
use crate::api_description::ApiEndpointResponse;
use crate::api_description::ApiSchemaGenerator;

use async_trait::async_trait;
use bytes::BufMut;
use bytes::Bytes;
use bytes::BytesMut;
use futures::lock::Mutex;
use http::StatusCode;
use hyper::Body;
use hyper::Request;
use hyper::Response;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;
use slog::Logger;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fmt::Formatter;
use std::fmt::Result as FmtResult;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

/**
 * Type alias for the result returned by HTTP handler functions.
 */
pub type HttpHandlerResult = Result<Response<Body>, HttpError>;

/**
 * Handle for various interfaces useful during request processing.
 * TODO-cleanup What's the right way to package up "request"?  The only time we
 * need it to be mutable is when we're reading the body (e.g., as part of the
 * JSON extractor).  In order to support that, we wrap it in something that
 * supports interior mutability.  It also needs to be thread-safe, since we're
 * using async/await.  That brings us to Arc<Mutex<...>>, but it seems like
 * overkill since it will only really be used by one thread at a time (at all,
 * let alone mutably) and there will never be contention on the Mutex.
 */
pub struct RequestContext {
    /** shared server state */
    pub server: Arc<DropshotState>,
    /** HTTP request details */
    pub request: Arc<Mutex<Request<Body>>>,
    /** HTTP request routing variables */
    pub path_variables: BTreeMap<String, String>,
    /** unique id assigned to this request */
    pub request_id: String,
    /** logger for this specific request */
    pub log: Logger,
}

/**
 * `Extractor` defines an interface allowing a type to be constructed from a
 * `RequestContext`.  Unlike most traits, `Extractor` essentially defines only a
 * constructor function, not instance functions.
 *
 * The extractors that we provide (`Query`, `Path`, `Json`) implement
 * `Extractor` in order to construct themselves from the request. For example,
 * `Extractor` is implemented for `Query<Q>` with a function that reads the
 * query string from the request, parses it, and constructs a `Query<Q>` with
 * it.
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
    async fn from_request(
        rqctx: Arc<RequestContext>,
    ) -> Result<Self, HttpError>;

    fn metadata() -> Vec<ApiEndpointParameter>;
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
        async fn from_request(_rqctx: Arc<RequestContext>)
            -> Result<( $($T,)* ), HttpError>
        {
            Ok( ($($T::from_request(Arc::clone(&_rqctx)).await?,)* ) )
        }

        fn metadata() -> Vec<ApiEndpointParameter> {
            #[allow(unused_mut)]
            let mut v = vec![];
            $( v.append(&mut $T::metadata()); )*
            v
        }
    }
}}

impl_extractor_for_tuple!();
impl_extractor_for_tuple!(T1);
impl_extractor_for_tuple!(T1, T2);
impl_extractor_for_tuple!(T1, T2, T3);

pub trait ExtractedParameter: DeserializeOwned {
    fn metadata(inn: ApiEndpointParameterLocation)
        -> Vec<ApiEndpointParameter>;
}

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
pub trait HttpHandlerFunc<FuncParams, ResponseType>:
    Send + Sync + 'static
where
    FuncParams: Extractor,
    ResponseType: HttpResponse + Send + Sync + 'static,
{
    async fn handle_request(
        &self,
        rqctx: Arc<RequestContext>,
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
    impl<FuncType, FutureType, ResponseType, $($T,)*>
        HttpHandlerFunc<($($T,)*), ResponseType> for FuncType
    where
        FuncType: Fn(Arc<RequestContext>, $($T,)*)
            -> FutureType + Send + Sync + 'static,
        FutureType: Future<Output = Result<ResponseType, HttpError>>
            + Send + 'static,
        ResponseType: HttpResponse + Send + Sync + 'static,
        $($T: Extractor + Send + Sync + 'static,)*
    {
        async fn handle_request(
            &self,
            rqctx: Arc<RequestContext>,
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
pub trait RouteHandler: Debug + Send + Sync {
    /**
     * Returns a description of this handler.  This might be a function name,
     * for example.  This is not guaranteed to be unique.
     */
    fn label(&self) -> &str;

    /**
     * Handle an incoming HTTP request.
     */
    async fn handle_request(&self, rqctx: RequestContext) -> HttpHandlerResult;
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
pub struct HttpRouteHandler<HandlerType, FuncParams, ResponseType>
where
    HandlerType: HttpHandlerFunc<FuncParams, ResponseType>,
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
    phantom: PhantomData<(FuncParams, ResponseType)>,
}

impl<HandlerType, FuncParams, ResponseType> Debug
    for HttpRouteHandler<HandlerType, FuncParams, ResponseType>
where
    HandlerType: HttpHandlerFunc<FuncParams, ResponseType>,
    FuncParams: Extractor,
    ResponseType: HttpResponse + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "handler: {}", self.label)
    }
}

#[async_trait]
impl<HandlerType, FuncParams, ResponseType> RouteHandler
    for HttpRouteHandler<HandlerType, FuncParams, ResponseType>
where
    HandlerType: HttpHandlerFunc<FuncParams, ResponseType>,
    FuncParams: Extractor + 'static,
    ResponseType: HttpResponse + Send + Sync + 'static,
{
    fn label(&self) -> &str {
        &self.label
    }

    async fn handle_request(
        &self,
        rqctx_raw: RequestContext,
    ) -> HttpHandlerResult {
        /*
         * This is where the magic happens: in the code below, `funcparams` has
         * type `FuncParams`, which is a tuple type describing the extractor
         * arguments to the handler function.  This could be `()`, `(Query<Q>)`,
         * `(Json<J>)`, `(Query<Q>, Json<J>)`, or any other combination of
         * extractors we decide to support in the future.  Whatever it is must
         * implement `Extractor`, which means we can invoke
         * `Extractor::from_request()` to construct the argument tuple, generally
         * from information available in the `request` object.  We pass this
         * down to the `HttpHandlerFunc`, for which there's a different
         * implementation for each value of `FuncParams`.  The `HttpHandlerFunc`
         * for each `FuncParams` just pulls the arguments out of the
         * `funcparams` tuple and makes them actual function arguments for the
         * actual handler function.  From this point down, all of this is
         * resolved statically.makes them actual function arguments for the
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

impl<HandlerType, FuncParams, ResponseType>
    HttpRouteHandler<HandlerType, FuncParams, ResponseType>
where
    HandlerType: HttpHandlerFunc<FuncParams, ResponseType>,
    FuncParams: Extractor + 'static,
    ResponseType: HttpResponse + Send + Sync + 'static,
{
    /**
     * Given a function matching one of the supported API handler function
     * signatures, return a RouteHandler that can be used to respond to HTTP
     * requests using this function.
     */
    pub fn new(handler: HandlerType) -> Box<dyn RouteHandler> {
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
    ) -> Box<dyn RouteHandler> {
        Box::new(HttpRouteHandler {
            label: label.to_string(),
            handler: handler,
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
pub struct Query<QueryType: ExtractedParameter + Send + Sync> {
    inner: QueryType,
}

impl<QueryType: ExtractedParameter + Send + Sync> Query<QueryType> {
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
fn http_request_load_query<QueryType: Send + Sync>(
    request: &Request<Body>,
) -> Result<Query<QueryType>, HttpError>
where
    QueryType: ExtractedParameter,
{
    let raw_query_string = request.uri().query().unwrap_or("");
    /*
     * TODO-correctness: are query strings defined to be urlencoded in this way?
     */
    match serde_urlencoded::from_str(raw_query_string) {
        Ok(q) => Ok(Query {
            inner: q,
        }),
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
    QueryType: ExtractedParameter + Send + Sync + 'static,
{
    async fn from_request(
        rqctx: Arc<RequestContext>,
    ) -> Result<Query<QueryType>, HttpError> {
        let request = rqctx.request.lock().await;
        http_request_load_query(&request)
    }

    fn metadata() -> Vec<ApiEndpointParameter> {
        QueryType::metadata(ApiEndpointParameterLocation::Query)
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
pub struct Path<PathType: ExtractedParameter + Send + Sync> {
    inner: PathType,
}

impl<PathType: ExtractedParameter + Send + Sync> Path<PathType> {
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
    PathType: ExtractedParameter + Send + Sync + 'static,
{
    async fn from_request(
        rqctx: Arc<RequestContext>,
    ) -> Result<Path<PathType>, HttpError> {
        let params: PathType = http_extract_path_params(&rqctx.path_variables)?;
        Ok(Path {
            inner: params,
        })
    }

    fn metadata() -> Vec<ApiEndpointParameter> {
        PathType::metadata(ApiEndpointParameterLocation::Path)
    }
}

/*
 * JSON: json body extractor
 */

/**
 * `Json<JsonType>` is an extractor used to deserialize an instance of
 * `JsonType` from an HTTP request body.  `JsonType` is any structure of yours
 * that implements `serde::Deserialize`.  See this module's documentation for
 * more information.
 */
pub struct Json<JsonType: JsonSchema + DeserializeOwned + Send + Sync> {
    inner: JsonType,
}

impl<JsonType: JsonSchema + DeserializeOwned + Send + Sync> Json<JsonType> {
    /*
     * TODO drop this in favor of Deref?  + Display and Debug for convenience?
     */
    pub fn into_inner(self) -> JsonType {
        self.inner
    }
}

/**
 * Given an HTTP request, attempt to read the body, parse it as JSON, and
 * deserialize an instance of `JsonType` from it.
 */
async fn http_request_load_json_body<JsonType>(
    rqctx: Arc<RequestContext>,
) -> Result<Json<JsonType>, HttpError>
where
    JsonType: JsonSchema + DeserializeOwned + Send + Sync,
{
    let server = &rqctx.server;
    let mut request = rqctx.request.lock().await;
    let body_bytes = http_read_body(
        request.body_mut(),
        server.config.request_body_max_bytes,
    )
    .await?;
    let value: Result<JsonType, serde_json::Error> =
        serde_json::from_slice(&body_bytes);
    match value {
        Ok(j) => Ok(Json {
            inner: j,
        }),
        Err(e) => Err(HttpError::for_bad_request(
            None,
            format!("unable to parse body: {}", e),
        )),
    }
}

/*
 * The `Extractor` implementation for Json<JsonType> describes how to construct
 * an instance of `Json<JsonType>` from an HTTP request: namely, by reading the
 * request body and parsing it as JSON into type `JsonType`.
 * TODO-cleanup We shouldn't have to use the "'static" bound on `JsonType` here.
 * It seems like we ought to be able to use 'async_trait, but that doesn't seem
 * to be defined.
 */
#[async_trait]
impl<JsonType> Extractor for Json<JsonType>
where
    JsonType: JsonSchema + DeserializeOwned + Send + Sync + 'static,
{
    async fn from_request(
        rqctx: Arc<RequestContext>,
    ) -> Result<Json<JsonType>, HttpError> {
        http_request_load_json_body(rqctx).await
    }

    fn metadata() -> Vec<ApiEndpointParameter> {
        vec![ApiEndpointParameter {
            name: ApiEndpointParameterName::Body,
            description: None,
            required: true,
            schema: Some(ApiSchemaGenerator(JsonType::json_schema)),
            examples: vec![],
        }]
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
    fn metadata() -> ApiEndpointResponse;
}

/**
 * `Response<Body>` is used for free-form responses. The implementation of
 * `to_result()` is trivial, and we don't have any typed metadata to return.
 */
impl HttpResponse for Response<Body> {
    fn to_result(self) -> HttpHandlerResult {
        Ok(self)
    }
    fn metadata() -> ApiEndpointResponse {
        ApiEndpointResponse {
            schema: None,
            success: None,
        }
    }
}

/*
 * Specific Response Types
 *
 * The `HttpTypedResponse` trait and the concrete types below are provided so
 * that handler functions can return types that indicate at compile time the
 * kind of HTTP response body they produce.
 */

/**
 * The `HttpTypedResponse` trait is used for all of the specific response types
 * that we provide. We use it in particular to encode the success status code
 * and the type information of the return value.
 */
pub trait HttpTypedResponse:
    Into<HttpHandlerResult> + Send + Sync + 'static
{
    type Body: JsonSchema + Serialize;
    const STATUS_CODE: StatusCode;

    /**
     * Convenience method to produce a response based on the input
     * `body_object` (whose specific type is defined by the implementing type)
     * and the STATUS_CODE specified by the implementing type. This is a default
     * trait method to allow callers to avoid redundant type specification.
     */
    fn for_object(&self, body_object: &Self::Body) -> HttpHandlerResult {
        let serialized = serde_json::to_string(&body_object)?;
        Ok(Response::builder()
            .status(Self::STATUS_CODE)
            .header(http::header::CONTENT_TYPE, CONTENT_TYPE_JSON)
            .body(serialized.into())?)
    }
}

/**
 * Provide results and metadata generation for all implementing types.
 */
impl<T> HttpResponse for T
where
    T: HttpTypedResponse,
{
    fn to_result(self) -> HttpHandlerResult {
        self.into()
    }
    fn metadata() -> ApiEndpointResponse {
        ApiEndpointResponse {
            schema: Some(ApiSchemaGenerator(T::Body::json_schema)),
            success: Some(T::STATUS_CODE),
        }
    }
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
pub struct HttpResponseCreated<T: Serialize + Send + Sync + 'static>(pub T);
impl<T: JsonSchema + Serialize + Send + Sync + 'static> HttpTypedResponse
    for HttpResponseCreated<T>
{
    type Body = T;
    const STATUS_CODE: StatusCode = StatusCode::CREATED;
}
impl<T: JsonSchema + Serialize + Send + Sync + 'static>
    From<HttpResponseCreated<T>> for HttpHandlerResult
{
    fn from(response: HttpResponseCreated<T>) -> HttpHandlerResult {
        /* TODO-correctness (or polish?): add Location header */
        response.for_object(&response.0)
    }
}

/**
 * `HttpResponseAccepted<T: Serialize>` wraps an object of any
 * serializable type.  It denotes an HTTP 202 "Accepted" response whose body is
 * generated by serializing the object.
 */
pub struct HttpResponseAccepted<T: Serialize + Send + Sync + 'static>(pub T);
impl<T: JsonSchema + Serialize + Send + Sync + 'static> HttpTypedResponse
    for HttpResponseAccepted<T>
{
    type Body = T;
    const STATUS_CODE: StatusCode = StatusCode::ACCEPTED;
}
impl<T: JsonSchema + Serialize + Send + Sync + 'static>
    From<HttpResponseAccepted<T>> for HttpHandlerResult
{
    fn from(response: HttpResponseAccepted<T>) -> HttpHandlerResult {
        response.for_object(&response.0)
    }
}

/**
 * `HttpResponseOkObject<T: Serialize>` wraps an object of any serializable
 * type.  It denotes an HTTP 200 "OK" response whose body is generated by
 * serializing the object.
 */
pub struct HttpResponseOkObject<T: Serialize + Send + Sync + 'static>(pub T);
impl<T: JsonSchema + Serialize + Send + Sync + 'static> HttpTypedResponse
    for HttpResponseOkObject<T>
{
    type Body = T;
    const STATUS_CODE: StatusCode = StatusCode::OK;
}
impl<T: JsonSchema + Serialize + Send + Sync + 'static>
    From<HttpResponseOkObject<T>> for HttpHandlerResult
{
    fn from(response: HttpResponseOkObject<T>) -> HttpHandlerResult {
        response.for_object(&response.0)
    }
}

/**
 * `HttpResponseOkObjectList<T: Serialize>` wraps a collection of serializable
 * types.  It denotes an HTTP 200 "OK" response whose body is generated by
 * serializing the sequence of objects.
 * TODO-polish We will probably want to add headers for the total result set
 * size and the number of results that we're returning here, plus the marker.
 * TODO-cleanup move/copy the type aliases from src/api_model.rs?
 */
pub struct HttpResponseOkObjectList<T: Serialize + Send + Sync + 'static>(
    pub Vec<T>,
);
impl<T: JsonSchema + Serialize + Send + Sync + 'static> HttpTypedResponse
    for HttpResponseOkObjectList<T>
{
    type Body = T;
    const STATUS_CODE: StatusCode = StatusCode::OK;
}
impl<T: JsonSchema + Serialize + Send + Sync + 'static>
    From<HttpResponseOkObjectList<T>> for HttpHandlerResult
{
    fn from(list_wrap: HttpResponseOkObjectList<T>) -> HttpHandlerResult {
        let list = list_wrap.0;
        let buffer_list = list.iter().map(serialize_json_stream_element);
        let mut bytebuf = BytesMut::new();
        for maybe_buffer in buffer_list {
            let buffer = maybe_buffer?;
            bytebuf.put(buffer);
        }

        Ok(Response::builder()
            .status(HttpResponseOkObjectList::<T>::STATUS_CODE)
            .header(http::header::CONTENT_TYPE, CONTENT_TYPE_NDJSON)
            .body(bytebuf.freeze().into())?)
    }
}

/**
 * `HttpResponseDeleted` represents an HTTP 204 "No Content" response, intended
 * for use when an API operation has successfully deleted an object.
 */
pub struct HttpResponseDeleted();

impl HttpTypedResponse for HttpResponseDeleted {
    type Body = ();
    const STATUS_CODE: StatusCode = StatusCode::NO_CONTENT;
}
impl From<HttpResponseDeleted> for HttpHandlerResult {
    fn from(_: HttpResponseDeleted) -> HttpHandlerResult {
        Ok(Response::builder()
            .status(HttpResponseDeleted::STATUS_CODE)
            .body(Body::empty())?)
    }
}

/**
 * `HttpResponseUpdatedNoContent` represents an HTTP 204 "No Content" response,
 * intended for use when an API operation has successfully updated an object and
 * has nothing to return.
 */
pub struct HttpResponseUpdatedNoContent();
impl HttpTypedResponse for HttpResponseUpdatedNoContent {
    type Body = ();
    const STATUS_CODE: StatusCode = StatusCode::NO_CONTENT;
}
impl From<HttpResponseUpdatedNoContent> for HttpHandlerResult {
    fn from(_: HttpResponseUpdatedNoContent) -> HttpHandlerResult {
        Ok(Response::builder()
            .status(HttpResponseUpdatedNoContent::STATUS_CODE)
            .body(Body::empty())?)
    }
}

/**
 * Given a Rust object representing an object in the API, serialize the object
 * to JSON bytes for inclusion in a newline-delimited-JSON ("ndjson") payload.
 * TODO-hardening: consider proceeding with the response even if one or more of
 * the objects fails to be serialized.  Otherwise, one bad database record (for
 * example) could cause us to be unable to list a whole class of items.
 */
fn serialize_json_stream_element<T: Serialize>(
    object: &T,
) -> Result<Bytes, HttpError> {
    let mut object_json_bytes = serde_json::to_vec(object)?;
    object_json_bytes.push(b'\n');
    Ok(object_json_bytes.into())
}
