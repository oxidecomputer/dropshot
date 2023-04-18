// Copyright 2023 Oxide Computer Company
//! Interface for implementing HTTP endpoint handler functions.
//!
//! For information about supported endpoint function signatures, argument types,
//! extractors, and return types, see the top-level documentation for this crate.
//! As documented there, we support several different sets of function arguments
//! and return types.
//!
//! We allow for variation in the function arguments not so much for programmer
//! convenience (since parsing the query string or JSON body could be implemented
//! in a line or two of code each, with the right helper functions) but rather so
//! that the type signature of the handler function can be programmatically
//! analyzed to generate an OpenAPI snippet for this endpoint.  This approach of
//! treating the server implementation as the source of truth for the API
//! specification ensures that--at least in many important ways--the
//! implementation cannot diverge from the spec.
//!
//! Just like we want API input types to be represented in function arguments, we
//! want API response types to be represented in function return values so that
//! OpenAPI tooling can identify them at build time.  The more specific a type
//! returned by the handler function, the more can be validated at build-time,
//! and the more specific an OpenAPI schema can be generated from the source
//! alone.
//!
//! We go through considerable effort below to make this interface possible.
//! Both the interface (primarily) and the implementation (less so) are inspired
//! by Actix-Web.  The Actix implementation is significantly more general (and
//! commensurately complex).  It would be possible to implement richer facilities
//! here, like extractors for backend server state, headers, and so on; allowing
//! for server and request parameters to be omitted; and so on; but those other
//! facilities don't seem that valuable right now since they largely don't affect
//! OpenAPI document generation.

use super::error::HttpError;
use super::extractor::RequestExtractor;
use super::http_util::CONTENT_TYPE_JSON;
use super::http_util::CONTENT_TYPE_OCTET_STREAM;
use super::server::DropshotState;
use super::server::ServerContext;
use crate::api_description::ApiEndpointBodyContentType;
use crate::api_description::ApiEndpointHeader;
use crate::api_description::ApiEndpointResponse;
use crate::api_description::ApiSchemaGenerator;
use crate::pagination::PaginationParams;
use crate::router::VariableSet;
use crate::schema_util::make_subschema_for;
use crate::schema_util::schema2struct;
use crate::schema_util::ReferenceVisitor;
use crate::to_map::to_map;

use async_trait::async_trait;
use http::HeaderMap;
use http::StatusCode;
use hyper::Body;
use hyper::Response;
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

/// Type alias for the result returned by HTTP handler functions.
pub type HttpHandlerResult = Result<Response<Body>, HttpError>;

/// Handle for various interfaces useful during request processing.
#[derive(Debug)]
pub struct RequestContext<Context: ServerContext> {
    /// shared server state
    pub server: Arc<DropshotState<Context>>,
    /// HTTP request routing variables
    pub path_variables: VariableSet,
    /// expected request body mime type
    pub body_content_type: ApiEndpointBodyContentType,
    /// unique id assigned to this request
    pub request_id: String,
    /// logger for this specific request
    pub log: Logger,

    /// basic request information (method, URI, etc.)
    pub request: RequestInfo,
}

// This is deliberately as close to compatible with `hyper::Request` as
// reasonable.
#[derive(Debug)]
pub struct RequestInfo {
    method: http::Method,
    uri: http::Uri,
    version: http::Version,
    headers: http::HeaderMap<http::HeaderValue>,
    remote_addr: std::net::SocketAddr,
}

impl RequestInfo {
    pub(crate) fn new<B>(
        request: &hyper::Request<B>,
        remote_addr: std::net::SocketAddr,
    ) -> Self {
        RequestInfo {
            method: request.method().clone(),
            uri: request.uri().clone(),
            version: request.version(),
            headers: request.headers().clone(),
            remote_addr,
        }
    }
}

impl RequestInfo {
    pub fn method(&self) -> &http::Method {
        &self.method
    }

    pub fn uri(&self) -> &http::Uri {
        &self.uri
    }

    pub fn version(&self) -> http::Version {
        self.version
    }

    pub fn headers(&self) -> &http::HeaderMap<http::HeaderValue> {
        &self.headers
    }

    pub fn remote_addr(&self) -> std::net::SocketAddr {
        self.remote_addr
    }

    /// Returns a reference to the `RequestInfo` itself
    ///
    /// This is provided for source compatibility.  In previous versions of
    /// Dropshot, `RequestContext.request` was an
    /// `Arc<Mutex<hyper::Request<hyper::Body>>>`.  Now, it's just
    /// `RequestInfo`, which provides many of the same functions as
    /// `hyper::Request` does.  Consumers _should_ just use `rqctx.request`
    /// instead of this function.
    ///
    /// For example, in previous versions of Dropshot, you might have:
    ///
    /// ```ignore
    /// let request = rqctx.request.lock().await;
    /// let headers = request.headers();
    /// ```
    ///
    /// Now, you would do this:
    ///
    /// ```ignore
    /// let headers = rqctx.request.headers();
    /// ```
    ///
    /// This function allows the older code to continue to work.
    #[deprecated(
        since = "0.9.0",
        note = "use `rqctx.request` directly instead of \
            `rqctx.request.lock().await`"
    )]
    pub async fn lock(&self) -> &Self {
        self
    }
}

impl<Context: ServerContext> RequestContext<Context> {
    /// Returns the server context state.
    pub fn context(&self) -> &Context {
        &self.server.private
    }

    /// Returns the appropriate count of items to return for a paginated request
    ///
    /// This first looks at any client-requested limit and clamps it based on the
    /// server-configured maximum page size.  If the client did not request any
    /// particular limit, this function returns the server-configured default
    /// page size.
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
            // Compare the client-provided limit to the configured max for the
            // server and take the smaller one.
            .map(|limit| min(limit, server_config.page_max_nitems))
            // If no limit was provided by the client, use the configured
            // default.
            .unwrap_or(server_config.page_default_nitems))
    }
}

/// Helper trait for extracting the underlying Context type from the
/// first argument to an endpoint. This trait exists to help the
/// endpoint macro parse this argument.
///
/// The first argument to an endpoint handler must be of the form:
/// `RequestContext<T>` where `T` is a caller-supplied
/// value that implements `ServerContext`.
pub trait RequestContextArgument {
    type Context;
}

impl<T: 'static + ServerContext> RequestContextArgument for RequestContext<T> {
    type Context = T;
}

/// `HttpHandlerFunc` is a trait providing a single function, `handle_request()`,
/// which takes an HTTP request and produces an HTTP response (or
/// `HttpError`).
///
/// As described above, handler functions can have a number of different
/// signatures.  They all consume a reference to the current request context.
/// They may also consume some number of extractor arguments.  The
/// `HttpHandlerFunc` trait is parametrized by the type `FuncParams`, which is
/// expected to be a tuple describing these extractor arguments.
///
/// Below, we define implementations of `HttpHandlerFunc` for various function
/// types.  In this way, we can treat functions with different signatures as
/// different kinds of `HttpHandlerFunc`.  However, since the signature shows up
/// in the `FuncParams` type parameter, we'll need additional abstraction to
/// treat different handlers interchangeably.  See `RouteHandler` below.
#[async_trait]
pub trait HttpHandlerFunc<Context, FuncParams, ResponseType>:
    Send + Sync + 'static
where
    Context: ServerContext,
    FuncParams: RequestExtractor,
    ResponseType: HttpResponse + Send + Sync + 'static,
{
    async fn handle_request(
        &self,
        rqctx: RequestContext<Context>,
        p: FuncParams,
    ) -> HttpHandlerResult;
}

/// Defines an implementation of the `HttpHandlerFunc` trait for functions
/// matching one of the supported signatures for HTTP endpoint handler functions.
/// We use a macro to do this because we need to provide different
/// implementations for functions that take 0 arguments, 1 argument, 2 arguments,
/// etc., but the implementations are almost identical.
// For background: as the module-level documentation explains, we want to
// support API endpoint handler functions that vary in their signature so that
// the signature can accurately reflect details about their expected input and
// output instead of a generic `Request -> Response` description.  The
// `HttpHandlerFunc` trait defines an interface for invoking one of these
// functions.  This macro defines an implementation of `HttpHandlerFunc` that
// says how to take any of these HTTP endpoint handler function and provide that
// uniform interface for callers.  The implementation essentially does three
// things:
//
// 1. Converts the uniform arguments of `handle_request()` into the appropriate
//    arguments for the underlying function.  This is easier than it sounds at
//    this point because we require that one of the arguments be a tuple whose
//    types correspond to the argument types for the function, so we just need
//    to unpack them from the tuple into function arguments.
//
// 2. Converts a call to the `handle_request()` method into a call to the
//    underlying function.
//
// 3. Converts the return type of the underlying function into the uniform
//    return type expected by callers of `handle_request()`.  This, too, is
//    easier than it sounds because we require that the return value implement
//    `HttpResponse`.
//
// As mentioned above, we're implementing the trait `HttpHandlerFunc` on _any_
// type `FuncType` that matches the trait bounds below.  In particular, it must
// take a request context argument and whatever other type parameters have been
// passed to this macro.
//
// The function's return type deserves further explanation.  (Actually, these
// functions all return a `Future`, but for convenience when we say "return
// type" in the comments here we're referring to the output type of the returned
// future.)  Again, as described above, we'd like to allow HTTP endpoint
// functions to return a variety of different return types that are ultimately
// converted into `Result<Response<Body>, HttpError>`.  To do that, the trait
// bounds below say that the function must produce a `Result<ResponseType,
// HttpError>` where `ResponseType` is a type that implements `HttpResponse`.
// We provide a few implementations of the trait `HttpTypedResponse` that
// includes a HTTP status code and structured output. In addition we allow for
// functions to hand-craft a `Response<Body>`. For both we implement
// `HttpResponse` (trivially in the latter case).
//
//      1. Handler function
//            |
//            | returns:
//            v
//      2. Result<ResponseType, HttpError>
//            |
//            | This may fail with an HttpError which we return immediately.
//            | On success, this will be Ok(ResponseType) for some specific
//            | ResponseType that implements HttpResponse.  We'll end up
//            | invoking:
//            v
//      3. ResponseType::to_result()
//            |
//            | This is a type-specific conversion from `ResponseType` into
//            | `Response<Body>` that's allowed to fail with an `HttpError`.
//            v
//      4. Result<Response<Body>, HttpError>
//
// Note that the handler function may fail due to an internal error *or* the
// conversion to JSON may successively fail in the call to
// `serde_json::to_string()`.
//
// The `HttpResponse` trait lets us handle both generic responses via
// `Response<Body>` as well as more structured responses via structures
// implementing `HttpResponse<Body = Type>`. The latter gives us a typed
// structure as well as response code that we use to generate rich OpenAPI
// content.
//
// Note: the macro parameters really ought to be `$i:literal` and `$T:ident`,
// however that causes us to run afoul of issue dtolnay/async-trait#46. The
// workaround is to make both parameters `tt` (token tree).
macro_rules! impl_HttpHandlerFunc_for_func_with_params {
    ($(($i:tt, $T:tt)),*) => {

    #[async_trait]
    impl<Context, FuncType, FutureType, ResponseType, $($T,)*>
        HttpHandlerFunc<Context, ($($T,)*), ResponseType> for FuncType
    where
        Context: ServerContext,
        FuncType: Fn(RequestContext<Context>, $($T,)*)
            -> FutureType + Send + Sync + 'static,
        FutureType: Future<Output = Result<ResponseType, HttpError>>
            + Send + 'static,
        ResponseType: HttpResponse + Send + Sync + 'static,
        ($($T,)*): RequestExtractor,
        $($T: Send + Sync + 'static,)*
    {
        async fn handle_request(
            &self,
            rqctx: RequestContext<Context>,
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

/// `RouteHandler` abstracts an `HttpHandlerFunc<FuncParams, ResponseType>` in a
/// way that allows callers to invoke the handler without knowing the handler's
/// function signature.
///
/// The "Route" in `RouteHandler` refers to the fact that this structure is used
/// to record that a specific handler has been attached to a specific HTTP route.
#[async_trait]
pub trait RouteHandler<Context: ServerContext>: Debug + Send + Sync {
    /// Returns a description of this handler.  This might be a function name,
    /// for example.  This is not guaranteed to be unique.
    fn label(&self) -> &str;

    /// Handle an incoming HTTP request.
    async fn handle_request(
        &self,
        rqctx: RequestContext<Context>,
        request: hyper::Request<hyper::Body>,
    ) -> HttpHandlerResult;
}

/// `HttpRouteHandler` is the only type that implements `RouteHandler`.  The
/// reason both exist is that we need `HttpRouteHandler::new()` to consume an
/// arbitrary kind of `HttpHandlerFunc<FuncParams>` and return an object that's
/// _not_ parametrized by `FuncParams`.  In fact, the resulting
/// `HttpRouteHandler` _is_ parametrized by `FuncParams`, but we returned it
/// as a `RouteHandler` that does not have those type parameters, allowing the
/// caller to ignore the differences between different handler function type
/// signatures.
pub struct HttpRouteHandler<Context, HandlerType, FuncParams, ResponseType>
where
    Context: ServerContext,
    HandlerType: HttpHandlerFunc<Context, FuncParams, ResponseType>,
    FuncParams: RequestExtractor,
    ResponseType: HttpResponse + Send + Sync + 'static,
{
    /// the actual HttpHandlerFunc used to implement this route
    handler: HandlerType,

    /// debugging label for the handler
    label: String,

    /// In order to define `new()` below, we need a type parameter `HandlerType`
    /// that implements `HttpHandlerFunc<FuncParams>`, which means we also need a
    /// `FuncParams` type parameter.  However, this type parameter would be
    /// unconstrained, which makes Rust upset.  Use of PhantomData<FuncParams>
    /// here causes the compiler to behave as though this struct referred to a
    /// `FuncParams`, which allows us to use the type parameter below.
    phantom: PhantomData<(FuncParams, ResponseType, Context)>,
}

impl<Context, HandlerType, FuncParams, ResponseType> Debug
    for HttpRouteHandler<Context, HandlerType, FuncParams, ResponseType>
where
    Context: ServerContext,
    HandlerType: HttpHandlerFunc<Context, FuncParams, ResponseType>,
    FuncParams: RequestExtractor,
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
    FuncParams: RequestExtractor + 'static,
    ResponseType: HttpResponse + Send + Sync + 'static,
{
    fn label(&self) -> &str {
        &self.label
    }

    async fn handle_request(
        &self,
        rqctx: RequestContext<Context>,
        request: hyper::Request<hyper::Body>,
    ) -> HttpHandlerResult {
        // This is where the magic happens: in the code below, `funcparams` has
        // type `FuncParams`, which is a tuple type describing the extractor
        // arguments to the handler function.  This could be `()`, `(Query<Q>)`,
        // `(TypedBody<J>)`, `(Query<Q>, TypedBody<J>)`, or any other
        // combination of extractors we decide to support in the future.
        // Whatever it is must implement `RequestExtractor`, which means we can
        // invoke `RequestExtractor::from_request()` to construct the argument
        // tuple, generally from information available in the `request` object.
        // We pass this down to the `HttpHandlerFunc`, for which there's a
        // different implementation for each value of `FuncParams`.  The
        // `HttpHandlerFunc` for each `FuncParams` just pulls the arguments out
        // of the `funcparams` tuple and makes them actual function arguments
        // for the actual handler function.  From this point down, all of this
        // is resolved statically.makes them actual function arguments for the
        // actual handler function.  From this point down, all of this is
        // resolved statically.
        let funcparams =
            RequestExtractor::from_request(&rqctx, request).await?;
        let future = self.handler.handle_request(rqctx, funcparams);
        future.await
    }
}

// Public interfaces

impl<Context, HandlerType, FuncParams, ResponseType>
    HttpRouteHandler<Context, HandlerType, FuncParams, ResponseType>
where
    Context: ServerContext,
    HandlerType: HttpHandlerFunc<Context, FuncParams, ResponseType>,
    FuncParams: RequestExtractor + 'static,
    ResponseType: HttpResponse + Send + Sync + 'static,
{
    /// Given a function matching one of the supported API handler function
    /// signatures, return a RouteHandler that can be used to respond to HTTP
    /// requests using this function.
    pub fn new(handler: HandlerType) -> Box<dyn RouteHandler<Context>> {
        HttpRouteHandler::new_with_name(handler, "<unlabeled handler>")
    }

    /// Given a function matching one of the supported API handler function
    /// signatures, return a RouteHandler that can be used to respond to HTTP
    /// requests using this function.
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

// Response Type Conversion
//
// See the discussion on macro `impl_HttpHandlerFunc_for_func_with_params` for a
// great deal of context on this.

/// HttpResponse must produce a `Result<Response<Body>, HttpError>` and generate
/// the response metadata.  Typically one should use `Response<Body>` or an
/// implementation of `HttpTypedResponse`.
pub trait HttpResponse {
    /// Generate the response to the HTTP call.
    fn to_result(self) -> HttpHandlerResult;

    /// Extract status code and structure metadata for the non-error response.
    /// Type information for errors is handled generically across all endpoints.
    fn response_metadata() -> ApiEndpointResponse;
}

/// `Response<Body>` is used for free-form responses. The implementation of
/// `to_result()` is trivial, and we don't have any typed metadata to return.
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

/// An "empty" type used to represent responses that have no associated data
/// payload. This isn't intended for general use, but must be pub since it's
/// used as the Body type for certain responses.
#[doc(hidden)]
pub struct Empty;

// Specific Response Types
//
// The `HttpTypedResponse` trait and the concrete types below are provided so
// that handler functions can return types that indicate at compile time the
// kind of HTTP response body they produce.

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

/// The `HttpCodedResponse` trait is used for all of the specific response types
/// that we provide. We use it in particular to encode the success status code
/// and the type information of the return value.
pub trait HttpCodedResponse:
    Into<HttpHandlerResult> + Send + Sync + 'static
{
    type Body: HttpResponseContent;
    const STATUS_CODE: StatusCode;
    const DESCRIPTION: &'static str;

    /// Convenience method to produce a response based on the input
    /// `body_object` (whose specific type is defined by the implementing type)
    /// and the STATUS_CODE specified by the implementing type. This is a default
    /// trait method to allow callers to avoid redundant type specification.
    fn for_object(body: Self::Body) -> HttpHandlerResult {
        body.to_response(Response::builder().status(Self::STATUS_CODE))
    }
}

/// Provide results and metadata generation for all implementing types.
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

/// `HttpResponseCreated<T: Serialize>` wraps an object of any serializable type.
/// It denotes an HTTP 201 "Created" response whose body is generated by
/// serializing the object.
// TODO-cleanup should ApiObject move into this submodule?  It'd be nice if we
// could restrict this to an ApiObject::View (by having T: ApiObject and the
// field having type T::View).
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
        // TODO-correctness (or polish?): add Location header
        HttpResponseCreated::for_object(response.0)
    }
}

/// `HttpResponseAccepted<T: Serialize>` wraps an object of any
/// serializable type.  It denotes an HTTP 202 "Accepted" response whose body is
/// generated by serializing the object.
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

/// `HttpResponseOk<T: Serialize>` wraps an object of any serializable type.  It
/// denotes an HTTP 200 "OK" response whose body is generated by serializing the
/// object.
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

/// `HttpResponseDeleted` represents an HTTP 204 "No Content" response, intended
/// for use when an API operation has successfully deleted an object.
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

/// `HttpResponseUpdatedNoContent` represents an HTTP 204 "No Content" response,
/// intended for use when an API operation has successfully updated an object and
/// has nothing to return.
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

/// Describes headers associated with a 300-level response.
#[derive(JsonSchema, Serialize)]
#[doc(hidden)]
pub struct RedirectHeaders {
    /// HTTP "Location" header
    // What type should we use to represent header values?
    //
    // It's tempting to use `http::HeaderValue` here.  But in HTTP, header
    // values can contain bytes that aren't valid Rust strings.  See
    // `http::header::HeaderValue`.  We could propagate this nonsense all the
    // way to the OpenAPI spec, encoding the Location header as, say,
    // base64-encoded bytes.  This sounds really annoying to consumers.  It's
    // also a fair bit more work to implement.  We'd need to create a separate
    // type for this field so that we can impl `Serialize` and `JsonSchema` on
    // it, and we'd need to also impl serialization of byte sequences in
    // `MapSerializer`.  Ugh.
    //
    // We just use `String`.  This might contain values that aren't valid in
    // HTTP response headers.  But we can at least validate that at runtime, and
    // it sure is easier to implement!
    location: String,
}

/// See `http_response_found()`
pub type HttpResponseFound =
    HttpResponseHeaders<HttpResponseFoundStatus, RedirectHeaders>;

/// `http_response_found` returns an HTTP 302 "Found" response with no response
/// body.
///
/// The sole argument will become the value of the `Location` header.  This is
/// where you want to redirect the client to.
///
/// Per MDN and RFC 9110 S15.4.3, you might want to use 307 ("Temporary
/// Redirect") or 303 ("See Other") instead.
pub fn http_response_found(
    location: String,
) -> Result<HttpResponseFound, HttpError> {
    let _ = http::HeaderValue::from_str(&location)
        .map_err(|e| http_redirect_error(e, &location))?;
    Ok(HttpResponseHeaders::new(
        HttpResponseFoundStatus,
        RedirectHeaders { location },
    ))
}

fn http_redirect_error(
    error: http::header::InvalidHeaderValue,
    location: &str,
) -> HttpError {
    HttpError::for_internal_error(format!(
        "error encoding redirect URL {:?}: {:#}",
        location, error
    ))
}

/// This internal type impls HttpCodedResponse.  Consumers should use
/// `HttpResponseFound` instead, which includes metadata about the `Location`
/// header.
#[doc(hidden)]
pub struct HttpResponseFoundStatus;
impl HttpCodedResponse for HttpResponseFoundStatus {
    type Body = Empty;
    const STATUS_CODE: StatusCode = StatusCode::FOUND;
    const DESCRIPTION: &'static str = "redirect (found)";
}
impl From<HttpResponseFoundStatus> for HttpHandlerResult {
    fn from(_: HttpResponseFoundStatus) -> HttpHandlerResult {
        HttpResponseFoundStatus::for_object(Empty)
    }
}

/// See `http_response_see_other()`
pub type HttpResponseSeeOther =
    HttpResponseHeaders<HttpResponseSeeOtherStatus, RedirectHeaders>;

/// `http_response_see_other` returns an HTTP 303 "See Other" response with no
/// response body.
///
/// The sole argument will become the value of the `Location` header.  This is
/// where you want to redirect the client to.
///
/// Use this (as opposed to 307 "Temporary Redirect") when you want the client to
/// follow up with a GET, rather than whatever method they used to make the
/// current request.  This is intended to be used after a PUT or POST to show a
/// confirmation page or the like.
pub fn http_response_see_other(
    location: String,
) -> Result<HttpResponseSeeOther, HttpError> {
    let _ = http::HeaderValue::from_str(&location)
        .map_err(|e| http_redirect_error(e, &location))?;
    Ok(HttpResponseHeaders::new(
        HttpResponseSeeOtherStatus,
        RedirectHeaders { location },
    ))
}

/// This internal type impls HttpCodedResponse.  Consumers should use
/// `HttpResponseSeeOther` instead, which includes metadata about the `Location`
/// header.
#[doc(hidden)]
pub struct HttpResponseSeeOtherStatus;
impl HttpCodedResponse for HttpResponseSeeOtherStatus {
    type Body = Empty;
    const STATUS_CODE: StatusCode = StatusCode::SEE_OTHER;
    const DESCRIPTION: &'static str = "redirect (see other)";
}
impl From<HttpResponseSeeOtherStatus> for HttpHandlerResult {
    fn from(_: HttpResponseSeeOtherStatus) -> HttpHandlerResult {
        HttpResponseSeeOtherStatus::for_object(Empty)
    }
}

/// See `http_response_temporary_redirect()`
pub type HttpResponseTemporaryRedirect =
    HttpResponseHeaders<HttpResponseTemporaryRedirectStatus, RedirectHeaders>;

/// `http_response_temporary_redirect` represents an HTTP 307 "Temporary
/// Redirect" response with no response body.
///
/// The sole argument will become the value of the `Location` header.  This is
/// where you want to redirect the client to.
///
/// Use this (as opposed to 303 "See Other") when you want the client to use the
/// same request method and body when it makes the follow-up request.
pub fn http_response_temporary_redirect(
    location: String,
) -> Result<HttpResponseTemporaryRedirect, HttpError> {
    let _ = http::HeaderValue::from_str(&location)
        .map_err(|e| http_redirect_error(e, &location))?;
    Ok(HttpResponseHeaders::new(
        HttpResponseTemporaryRedirectStatus,
        RedirectHeaders { location },
    ))
}

/// This internal type impls HttpCodedResponse.  Consumers should use
/// `HttpResponseTemporaryRedirect` instead, which includes metadata about the
/// `Location` header.
#[doc(hidden)]
pub struct HttpResponseTemporaryRedirectStatus;
impl HttpCodedResponse for HttpResponseTemporaryRedirectStatus {
    type Body = Empty;
    const STATUS_CODE: StatusCode = StatusCode::TEMPORARY_REDIRECT;
    const DESCRIPTION: &'static str = "redirect (temporary redirect)";
}
impl From<HttpResponseTemporaryRedirectStatus> for HttpHandlerResult {
    fn from(_: HttpResponseTemporaryRedirectStatus) -> HttpHandlerResult {
        HttpResponseTemporaryRedirectStatus::for_object(Empty)
    }
}

#[derive(Serialize, JsonSchema)]
pub struct NoHeaders {}

/// `HttpResponseHeaders` is a wrapper for responses that include both
/// structured and unstructured headers. The first type parameter is a
/// `HttpTypedResponse` that provides the structure of the response body.
/// The second type parameter is an optional struct that enumerates named
/// headers that are included in the response. In addition to those (optional)
/// named headers, consumers may add additional headers via the `headers_mut`
/// interface. Unnamed headers override named headers in the case of naming
/// conflicts.
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
        // Compute the body.
        let mut result = body.into()?;
        // Add in both the structured and other headers.
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
