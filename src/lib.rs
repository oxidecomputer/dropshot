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
 * ## Status
 *
 * Dropshot is a work in progress.  It remains inside the parent repo because
 * we're still making changes that span both repos reasonably often.  Once it's
 * mature, we'll separate it into a separate repo and potentially publish it.
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
 * describing the API.  See [`ApiDescription::print_openapi`].
 *
 *
 * ## API Handler Functions
 *
 * HTTP talks about **resources**.  For a REST API, we often talk about
 * **endpoints** or **operations**, which are identified by a combination of the
 * HTTP method and the URI path
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
 * The most convenient way to define an endpoint with a handler function uses
 * the `endpoint!` macro.  Here's an example of a single endpoint that lists
 * three hardcoded projects:
 *
 * ```
 * use dropshot::endpoint;
 * use dropshot::ApiDescription;
 * use dropshot::HttpError;
 * use dropshot::HttpResponseOkObjectList;
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
 * /** Fetch the list of projects. */
 * #[endpoint {
 *     method = GET,
 *     path = "/projects",
 * }]
 * async fn myapi_projects_get(
 *     rqctx: Arc<RequestContext>,
 * ) -> Result<HttpResponseOkObjectList<Project>, HttpError>
 * {
 *    let projects = vec![
 *        Project { name: String::from("project1") },
 *        Project { name: String::from("project2") },
 *        Project { name: String::from("project3") },
 *    ];
 *    Ok(HttpResponseOkObjectList(projects))
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
 *     api.register(myapi_projects_get).unwrap();
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
 *   `HttpResponseOkObjectList<Project>`.  This means that the function will
 *   return an HTTP 200 status code ("OK") with a list of objects, each being an
 *   instance of `Project`.
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
 * ### Function arguments
 *
 * In general, a handler function looks like this:
 *
 * ```ignore
 * async fn f(
 *      rqctx: Arc<RequestContext>,
 *      [query_params: Query<Q>,]
 *      [path_params: Path<P>,]
 *      [body_param: Json<J>,]
 * ) -> Result< SomeResponseType , HttpError>
 * ```
 *
 * Other than the RequestContext, parameters may appear in any order.  The types
 * `Query`, `Path`, and `Json` are called **Extractors** because they cause
 * information to be pulled out of the request and made available to the handler
 * function.
 *
 * * [`Query`]`<Q>` extracts parameters from a query string, deserializing them
 *   into an instance of type `Q`. `Q` must implement `serde::Deserialize` and
 *   `dropshot::ExtractedParameter`.
 * * [`Path`]`<P>` extracts parameters from HTTP path, deserializing them into
 *    an instance of type `P`. `P` must implement `serde::Deserialize` and
 *    `dropshot::ExtractedParameter`.
 * * [`Json`]`<J>` extracts content from the request body by parsing the body as
 *   JSON and deserializing it into an instance of type `J`. `J` must implement
 *   `serde::Deserialize` and `schemars::JsonSchema`.
 *
 * If the handler takes a `Query<Q>`, `Path<P>`, or a `Json<J>` and the
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
 * use dropshot::ExtractedParameter;
 * use dropshot::HttpError;
 * use dropshot::Json;
 * use dropshot::Query;
 * use dropshot::RequestContext;
 * use hyper::Body;
 * use hyper::Response;
 * use std::sync::Arc;
 *
 * #[derive(serde::Deserialize, ExtractedParameter)]
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
 * one of the Dropshot-provided ones or one of your own creation). In
 * situations where the response schema is not fixed, the endpoint should
 * return `Response<Body`>, which also implements `HttpResponse`.
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
 * no way to violate this contract at runtime.  If the function just returned
 * `Response<Body>`, it would be harder to tell what it actually produces (for
 * generating the OpenAPI spec), and no way to validate that it really does
 * that.
 */

mod api_description;
mod config;
mod error;
mod handler;
mod http_util;
mod logging;
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
pub use config::ConfigDropshot;
pub use error::HttpError;
pub use error::HttpErrorResponseBody;
pub use handler::ExtractedParameter;
pub use handler::Extractor;
pub use handler::HttpResponse;
pub use handler::HttpResponseAccepted;
pub use handler::HttpResponseCreated;
pub use handler::HttpResponseDeleted;
pub use handler::HttpResponseOkObject;
pub use handler::HttpResponseOkObjectList;
pub use handler::HttpResponseUpdatedNoContent;
pub use handler::Json;
pub use handler::Path;
pub use handler::Query;
pub use handler::RequestContext;
pub use http_util::CONTENT_TYPE_JSON;
pub use http_util::CONTENT_TYPE_NDJSON;
pub use http_util::HEADER_REQUEST_ID;
pub use logging::ConfigLogging;
pub use logging::ConfigLoggingIfExists;
pub use logging::ConfigLoggingLevel;
pub use server::HttpServer;

/*
 * Users of the `endpoint` macro need `http::Method` available.
 */
pub use http::Method;

extern crate dropshot_endpoint;
pub use dropshot_endpoint::endpoint;
pub use dropshot_endpoint::ExtractedParameter;
