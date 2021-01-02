// Copyright 2020 Oxide Computer Company
/*!
 * Dropshot is a general-purpose crate for exposing REST APIs from a Rust
 * program.  Planned highlights include:
 *
 * * Suitability for production use on a largely untrusted network.
 *   Dropshot-based systems should be high-performing, reliable, debuggable, and
 *   secure against basic denial of service attacks (intentional or otherwise).
 *
 * * First-class OpenAPI support, in the form of precise OpenAPI specs generated
 *   directly from code.  This works because the functions that serve HTTP
 *   resources consume arguments and return values of specific types from which
 *   a schema can be statically generated.
 *
 * * Ease of integrating into a diverse team.  An important use case for
 *   Dropshot consumers is to have a team of engineers where individuals might
 *   add a few endpoints at a time to a complex server, and it should be
 *   relatively easy to do this.  Part of this means an emphasis on the
 *   principle of least surprise: like Rust itself, we may choose abstractions
 *   that require more time to learn up front in order to make it harder to
 *   accidentally build systems that will not perform, will crash in corner
 *   cases, etc.
 *
 * By "REST API", we primarily mean an API built atop existing HTTP primitives,
 * organized into hierarchical resources, and providing consistent, idempotent
 * mechanisms to create, update, list, and delete those resources.  "REST" can
 * mean a range of things depending on who you talk to, and some people are
 * dogmatic about what is or isn't RESTy.  We find such dogma not only
 * unhelpful, but poorly defined.  (Consider such a simple case as trying to
 * update a resource in a REST API.  Popular APIs sometimes use `PUT`, `PATCH`,
 * or `POST` for the verb; and JSON Merge Patch or JSON Patch as the format.
 * (sometimes without even knowing it!).  There's hardly a clear standard, yet
 * this is a really basic operation for any REST API.)
 *
 * For a discussion of alternative crates considered, see Oxide RFD 10.
 *
 * We hope Dropshot will be fairly general-purpose, but it's primarily intended
 * to address the needs of the Oxide control plane.
 *
 *
 * ## Usage
 *
 * The bare minimum might look like this:
 *
 * ```no_run
 * use dropshot::ApiDescription;
 * use dropshot::ConfigDropshot;
 * use dropshot::ConfigLogging;
 * use dropshot::ConfigLoggingLevel;
 * use dropshot::HttpServer;
 * use std::sync::Arc;
 *
 * #[tokio::main]
 * async fn main() -> Result<(), String> {
 *     // Set up a logger.
 *     let log =
 *         ConfigLogging::StderrTerminal {
 *             level: ConfigLoggingLevel::Info,
 *         }
 *         .to_logger("minimal-example")
 *         .map_err(|e| e.to_string())?;
 *
 *     // Describe the API.
 *     let mut api = ApiDescription::new();
 *     // Register API functions -- see detailed example or ApiDescription docs.
 *
 *     // Start the server.
 *     let mut server =
 *         HttpServer::new(
 *             &ConfigDropshot {
 *                 bind_address: "127.0.0.1:0".parse().unwrap(),
 *                 request_body_max_bytes: 1024,
 *             },
 *             api,
 *             Arc::new(()),
 *             &log,
 *         )
 *         .map_err(|error| format!("failed to start server: {}", error))?;
 *
 *     let server_task = server.run();
 *     server.wait_for_shutdown(server_task).await
 * }
 * ```
 *
 * This server returns a 404 for all resources because no API functions were
 * registered.  See `examples/basic.rs` for a simple, documented example that
 * provides a few resources using shared state.
 *
 * For a given `ApiDescription`, you can also print out an OpenAPI spec
 * describing the API.  See [`ApiDescription::openapi`].
 *
 *
 * ## API Handler Functions
 *
 * HTTP talks about **resources**.  For a REST API, we often talk about
 * **endpoints** or **operations**, which are identified by a combination of the
 * HTTP method and the URI path.
 *
 * Example endpoints for a resource called a "project" might include:
 *
 * * `GET /projects` (list projects)
 * * `POST /projects` (one way to create a project)
 * * `GET /projects/my_project` (fetch one project)
 * * `PUT /projects/my_project` (update (or possibly create) a project)
 * * `DELETE /projects/my_project` (delete a project)
 *
 * With Dropshot, an incoming request for a given API endpoint is handled by a
 * particular Rust function.  That function is called an **entrypoint**, an
 * **endpoint handler**, or a **handler function**.  When you set up a Dropshot
 * server, you configure the set of available API endpoints and which functions
 * will handle each one by setting up an [`ApiDescription`].
 *
 * Typically, you define an endpoint with a handler function by using the
 * [`endpoint`] macro. Here's an example of a single endpoint that lists
 * a hardcoded project:
 *
 * ```
 * use dropshot::endpoint;
 * use dropshot::ApiDescription;
 * use dropshot::HttpError;
 * use dropshot::HttpResponseOk;
 * use dropshot::RequestContext;
 * use http::Method;
 * use schemars::JsonSchema;
 * use serde::Serialize;
 * use std::sync::Arc;
 *
 * /** Represents a project in our API */
 * #[derive(Serialize, JsonSchema)]
 * struct Project {
 *     /** name of the project */
 *     name: String,
 * }
 *
 * /** Fetch a project. */
 * #[endpoint {
 *     method = GET,
 *     path = "/projects/project1",
 * }]
 * async fn myapi_projects_get_project(
 *     rqctx: Arc<RequestContext>,
 * ) -> Result<HttpResponseOk<Project>, HttpError>
 * {
 *    let project = Project { name: String::from("project1") };
 *    Ok(HttpResponseOk(project))
 * }
 *
 * fn main() {
 *     let mut api = ApiDescription::new();
 *
 *     /*
 *      * Register our endpoint and its handler function.  The "endpoint" macro
 *      * specifies the HTTP method and URI path that identify the endpoint,
 *      * allowing this metadata to live right alongside the handler function.
 *      */
 *     api.register(myapi_projects_get_project).unwrap();
 *
 *     /* ... (use `api` to set up an `HttpServer` ) */
 * }
 *
 * ```
 *
 * There's quite a lot going on here:
 *
 * * The `endpoint` macro specifies the HTTP method and URI path.  When we
 *   invoke `ApiDescription::register()`, this information is used to register
 *   the endpoint that will be handled by our function.
 * * The signature of our function indicates that on success, it returns a
 *   `HttpResponseOk<Project>`.  This means that the function will
 *   return an HTTP 200 status code ("OK") with an object of type `Project`.
 * * The function itself has a Rustdoc comment that will be used to document
 *   this _endpoint_ in the OpenAPI schema.
 *
 * From this information, Dropshot can generate an OpenAPI specification for
 * this API that describes the endpoint (which OpenAPI calls an "operation"),
 * its documentation, the possible responses that it can return, and the schema
 * for each type of response (which can also include documentation).  This is
 * largely known statically, though generated at runtime.
 *
 *
 * ### `#[endpoint { ... }]` attribute parameters
 *
 * The `endpoint` attribute accepts parameters the affect the operation of
 * the endpoint as well as metadata that appears in the OpenAPI description
 * of it.
 *
 * ```ignore
 * #[endpoint {
 *     // Required fields
 *     method = { DELETE | GET | PATCH | POST | PUT },
 *     path = "/path/name/with/{named}/{variables}",
 *
 *     // Optional fields
 *     tags = [ "all", "your", "OpenAPI", "tags" ],
 * }]
 * ```
 *
 * This is where you specify the HTTP method and path (including path variables)
 * for the API endpoint. These are used as part of endpoint registration and
 * appear in the OpenAPI spec output.
 *
 * The tags field is used to categorize API endpoints and only impacts the
 * OpenAPI spec output.
 *
 *
 * ### Function parameters
 *
 * In general, a handler function looks like this:
 *
 * ```ignore
 * async fn f(
 *      rqctx: Arc<RequestContext>,
 *      [query_params: Query<Q>,]
 *      [path_params: Path<P>,]
 *      [body_param: TypedBody<J>,]
 * ) -> Result<HttpResponse*, HttpError>
 * ```
 *
 * Other than the RequestContext, parameters may appear in any order.  The types
 * `Query`, `Path`, and `TypedBody` are called **Extractors** because they cause
 * information to be pulled out of the request and made available to the handler
 * function.
 *
 * * [`Query`]`<Q>` extracts parameters from a query string, deserializing them
 *   into an instance of type `Q`. `Q` must implement `serde::Deserialize` and
 *   `schemars::JsonSchema`.
 * * [`Path`]`<P>` extracts parameters from HTTP path, deserializing them into
 *   an instance of type `P`. `P` must implement `serde::Deserialize` and
 *   `schemars::JsonSchema`.
 * * [`TypedBody`]`<J>` extracts content from the request body by parsing the
 *   body as JSON and deserializing it into an instance of type `J`. `J` must
 *   implement `serde::Deserialize` and `schemars::JsonSchema`.
 *
 * If the handler takes a `Query<Q>`, `Path<P>`, or a `TypedBody<J>` and the
 * corresponding extraction cannot be completed, the request fails with status
 * code 400 and an error message reflecting a validation error.
 *
 * As with any serde-deserializable type, you can make fields optional by having
 * the corresponding property of the type be an `Option`.  Here's an example of
 * an endpoint that takes two arguments via query parameters: "limit", a
 * required u32, and "marker", an optional string:
 *
 * ```
 * use http::StatusCode;
 * use dropshot::HttpError;
 * use dropshot::TypedBody;
 * use dropshot::Query;
 * use dropshot::RequestContext;
 * use hyper::Body;
 * use hyper::Response;
 * use schemars::JsonSchema;
 * use serde::Deserialize;
 * use std::sync::Arc;
 *
 * #[derive(Deserialize, JsonSchema)]
 * struct MyQueryArgs {
 *     limit: u32,
 *     marker: Option<String>
 * }
 *
 * async fn myapi_projects_get(
 *     _: Arc<RequestContext>,
 *     query: Query<MyQueryArgs>)
 *     -> Result<Response<Body>, HttpError>
 * {
 *     let query_args = query.into_inner();
 *     let limit: u32 = query_args.limit;
 *     let marker: Option<String> = query_args.marker;
 *     Ok(Response::builder()
 *         .status(StatusCode::OK)
 *         .body(format!("limit = {}, marker = {:?}\n", limit, marker).into())?)
 * }
 * ```
 *
 * ### Endpoint function return types
 *
 * Endpoint handler functions are async, so they always return a `Future`.  When
 * we say "return type" below, we use that as shorthand for the output of the
 * future.
 *
 * An endpoint function must return a type that implements `HttpResponse`.
 * Typically this should be a type that implements `HttpTypedResponse` (either
 * one of the Dropshot-provided ones or one of your own creation).
 *
 * The more specific a type returned by the handler function, the more can be
 * validated at build-time, and the more specific an OpenAPI schema can be
 * generated from the source code.  For example, a POST to an endpoint
 * "/projects" might return `Result<HttpResponseCreated<Project>, HttpError>`.
 * As you might expect, on success, this turns into an HTTP 201 "Created"
 * response whose body is constructed by serializing the `Project`.  In this
 * example, OpenAPI tooling can identify at build time that this function
 * produces a 201 "Created" response on success with a body whose schema matches
 * `Project` (which we already said implements `Serialize`), and there would be
 * no way to violate this contract at runtime.
 *
 * These are the implementations of `HttpTypedResponse` with their associated
 * HTTP response code
 * on the HTTP method:
 *
 * | Return Type | HTTP status code |
 * | ----------- | ---------------- |
 * | [`HttpResponseOk`] | 200 |
 * | [`HttpResponseCreated`] | 201 |
 * | [`HttpResponseAccepted`] | 202 |
 * | [`HttpResponseDeleted`] | 204 |
 * | [`HttpResponseUpdatedNoContent`] | 204 |
 *
 * In situations where the response schema is not fixed, the endpoint should
 * return `Response<Body>`, which also implements `HttpResponse`. Note that
 * the OpenAPI spec will not include any status code or type information in
 * this case.
 *
 *
 * ## Support for paginated resources
 *
 * "Pagination" here refers to the interface pattern where HTTP resources (or
 * API endpoints) that provide a list of the items in a collection return a
 * relatively small maximum number of items per request, often called a "page"
 * of results.  Each page includes some metadata that the client can use to make
 * another request for the next page of results.  The client can repeat this
 * until they've gotten all the results.  Limiting the number of results
 * returned per request helps bound the resource utilization and time required
 * for any request, which in turn facilities horizontal scalability, high
 * availability, and protection against some denial of service attacks
 * (intentional or otherwise).  For more background, see the comments in
 * dropshot/src/pagination.rs.
 *
 * Pagination support in Dropshot implements this common pattern:
 *
 * * This server exposes an **API endpoint** that returns the **items**
 *   contained within a **collection**.
 * * The client is not allowed to list the entire collection in one request.
 *   Instead, they list the collection using a sequence of requests to the one
 *   endpoint.  We call this sequence of requests a **scan** of the collection,
 *   and we sometimes say that the client **pages through** the collection.
 * * The initial request in the scan may specify the **scan parameters**, which
 *   typically specify how the results are to be sorted (i.e., by which
 *   field(s) and whether the sort is ascending or descending), any filters to
 *   apply, etc.
 * * Each request returns a **page** of results at a time, along with a **page
 *   token** that's provided with the next request as a query parameter.
 * * The scan parameters cannot change between requests that are part of the
 *   same scan.
 * * With all requests: there's a default limit (e.g., 100 items returned at a
 *   time).  Clients can request a higher limit using a query parameter (e.g.,
 *   `limit=1000`).  This limit is capped by a hard limit on the server.  If the
 *   client asks for more than the hard limit, the server can use the hard limit
 *   or reject the request.
 *
 * As an example, imagine that we have an API endpoint called `"/animals"`.  Each
 * item returned is an `Animal` object that might look like this:
 *
 * ```json
 * {
 *     "name": "aardvark",
 *     "class": "mammal",
 *     "max_weight": "80", /* kilograms, typical */
 * }
 * ```
 *
 * There are at least 1.5 million known species of animal -- too many to return
 * in one API call!  Our API supports paginating them by `"name"`, which we'll
 * say is a unique field in our data set.
 *
 * The first request to the API fetches `"/animals"` (with no querystring
 * parameters) and returns:
 *
 * ```json
 * {
 *     "page_token": "abc123...",
 *     "items": [
 *         {
 *             "name": "aardvark",
 *             "class": "mammal",
 *             "max_weight": "80",
 *         },
 *         ...
 *         {
 *             "name": "badger",
 *             "class": "mammal",
 *             "max_weight": "12",
 *         }
 *     ]
 * }
 * ```
 *
 * The subsequent request to the API fetches `"/animals?page_token=abc123..."`.
 * The page token `"abc123..."` is an opaque token to the client, but typically
 * encodes the scan parameters and the value of the last item seen
 * (`"name=badger"`).  The client knows it has completed the scan when it
 * receives a response with no `page_token` in it.
 *
 * Our API endpoint can also support scanning in reverse order.  In this case,
 * when the client makes the first request, it should fetch
 * `"/animals?sort=name-descending"`.  Now the first result might be `"zebra"`.
 * Again, the page token must include the scan parameters so that in subsequent
 * requests, the API endpoint knows that we're scanning backwards, not forwards,
 * from the value we were given.  It's not allowed to change directions or sort
 * order in the middle of a scan.  (You can always start a new scan, but you
 * can't pick up from where you were in the previous scan.)
 *
 * It's also possible to support sorting by multiple fields.  For example, we
 * could support `sort=class-name`, which we could define to mean that we'll
 * sort the results first by the animal's class, then by name.  Thus we'd get
 * all the amphibians in sorted order, then all the mammals, then all the
 * reptiles.  The main requirement is that the combination of fields used for
 * pagination must be unique.  We cannot paginate by the animal's class alone.
 * (To see why: there are over 6,000 mammals.  If the page size is, say, 1000,
 * then the page_token would say `"mammal"`, but there's not enough information
 * there to see where we are within the list of mammals.  It doesn't matter
 * whether there are 2 mammals or 6,000 because clients can limit the page size
 * to just one item if they want and that ought to work.)
 *
 *
 * ### Dropshot interfaces for pagination
 *
 * We can think of pagination in two parts: the input (handling the pagination
 * query parameters) and the output (emitting a page of results, including the
 * page token).
 *
 * For input, a paginated API endpoint's handler function should accept a
 * [`Query`]`<`[`PaginationParams`]`<ScanParams, PageSelector>>`, where
 * `ScanParams` is a consumer-defined type specifying the parameters of the scan
 * (typically including the sort fields, sort order, and filter options) and
 * `PageSelector` is a consumer-defined type describing the page token.  The
 * PageSelector will be serialized to JSON and base64-encoded to construct the
 * page token.  This will be automatically parsed on the way back in.
 *
 * For output, a paginated API endpoint's handler function can return
 * `Result<`[`HttpResponseOk`]<[`ResultsPage`]`<T>, HttpError>` where `T:
 * Serialize` is the item listed by the endpoint.  You can also use your own
 * structure that contains a [`ResultsPage`] (possibly using
 * `#[serde(flatten)]`), if that's the behavior you want.
 *
 * There are several complete, documented examples in the "examples" directory.
 *
 *
 * ### Advanced usage notes
 *
 * It's possible to accept additional query parameters besides the pagination
 * parameters by having your API endpoint handler function take two different
 * arguments using `Query`, like this:
 *
 * ```
 * use dropshot::HttpError;
 * use dropshot::HttpResponseOk;
 * use dropshot::PaginationParams;
 * use dropshot::Query;
 * use dropshot::RequestContext;
 * use dropshot::ResultsPage;
 * use dropshot::endpoint;
 * use schemars::JsonSchema;
 * use serde::Deserialize;
 * use std::sync::Arc;
 * # use serde::Serialize;
 * # #[derive(Debug, Deserialize, JsonSchema)]
 * # enum MyScanParams { A };
 * # #[derive(Debug, Deserialize, JsonSchema, Serialize)]
 * # enum MyPageSelector { A(String) };
 * #[derive(Deserialize, JsonSchema)]
 * struct MyExtraQueryParams {
 *     do_extra_stuff: bool,
 * }
 *
 * #[endpoint {
 *     method = GET,
 *     path = "/list_stuff"
 * }]
 * async fn my_list_api(
 *     rqctx: Arc<RequestContext>,
 *     pag_params: Query<PaginationParams<MyScanParams, MyPageSelector>>,
 *     extra_params: Query<MyExtraQueryParams>,
 * ) -> Result<HttpResponseOk<ResultsPage<String>>, HttpError>
 * {
 *  # unimplemented!();
 *  /* ... */
 * }
 * ```
 *
 * You might expect that instead of doing this, you could define your own
 * structure that includes a `PaginationParams` using `#[serde(flatten)]`, and
 * this ought to work, but it currently doesn't due to serde_urlencoded#33,
 * which is really serde#1183.
 */

mod api_description;
mod config;
mod error;
mod from_map;
mod handler;
mod http_util;
mod logging;
mod pagination;
mod router;
mod server;

pub mod test_util;

#[macro_use]
extern crate slog;

pub use api_description::ApiDescription;
pub use api_description::ApiEndpoint;
pub use api_description::ApiEndpointParameter;
pub use api_description::ApiEndpointParameterLocation;
pub use api_description::ApiEndpointResponse;
pub use api_description::OpenApiDefinition;
pub use config::ConfigDropshot;
pub use error::HttpError;
pub use error::HttpErrorResponseBody;
pub use handler::Extractor;
pub use handler::HttpResponse;
pub use handler::HttpResponseAccepted;
pub use handler::HttpResponseCreated;
pub use handler::HttpResponseDeleted;
pub use handler::HttpResponseOk;
pub use handler::HttpResponseUpdatedNoContent;
pub use handler::Path;
pub use handler::Query;
pub use handler::RequestContext;
pub use handler::TypedBody;
pub use http_util::CONTENT_TYPE_JSON;
pub use http_util::CONTENT_TYPE_NDJSON;
pub use http_util::HEADER_REQUEST_ID;
pub use logging::ConfigLogging;
pub use logging::ConfigLoggingIfExists;
pub use logging::ConfigLoggingLevel;
pub use pagination::EmptyScanParams;
pub use pagination::PaginationOrder;
pub use pagination::PaginationParams;
pub use pagination::ResultsPage;
pub use pagination::WhichPage;
pub use server::HttpServer;

/*
 * Users of the `endpoint` macro need `http::Method` available.
 */
pub use http::Method;

extern crate dropshot_endpoint;
pub use dropshot_endpoint::endpoint;
