// Copyright 2025 Oxide Computer Company

//! Dropshot is a general-purpose crate for exposing REST APIs from a Rust
//! program.  Planned highlights include:
//!
//! * Suitability for production use on a largely untrusted network.
//!   Dropshot-based systems should be high-performing, reliable, debuggable, and
//!   secure against basic denial of service attacks (intentional or otherwise).
//!
//! * First-class OpenAPI support, in the form of precise OpenAPI specs generated
//!   directly from code.  This works because the functions that serve HTTP
//!   resources consume arguments and return values of specific types from which
//!   a schema can be statically generated.
//!
//! * Ease of integrating into a diverse team.  An important use case for
//!   Dropshot consumers is to have a team of engineers where individuals might
//!   add a few endpoints at a time to a complex server, and it should be
//!   relatively easy to do this.  Part of this means an emphasis on the
//!   principle of least surprise: like Rust itself, we may choose abstractions
//!   that require more time to learn up front in order to make it harder to
//!   accidentally build systems that will not perform, will crash in corner
//!   cases, etc.
//!
//! By "REST API", we primarily mean an API built atop existing HTTP primitives,
//! organized into hierarchical resources, and providing consistent, idempotent
//! mechanisms to create, update, list, and delete those resources.  "REST" can
//! mean a range of things depending on who you talk to, and some people are
//! dogmatic about what is or isn't RESTy.  We find such dogma not only
//! unhelpful, but poorly defined.  (Consider such a simple case as trying to
//! update a resource in a REST API.  Popular APIs sometimes use `PUT`, `PATCH`,
//! or `POST` for the verb; and JSON Merge Patch or JSON Patch as the format.
//! (sometimes without even knowing it!).  There's hardly a clear standard, yet
//! this is a really basic operation for any REST API.)
//!
//! For a discussion of alternative crates considered, see [Oxide RFD 10].
//!
//! We hope Dropshot will be fairly general-purpose, but it's primarily intended
//! to address the needs of the Oxide control plane.
//!
//! [Oxide RFD 10]: https://github.com/oxidecomputer/dropshot/issues/56#issuecomment-705710515
//!
//! ## Usage
//!
//! The bare minimum might look like this:
//!
//! ```no_run
//! use dropshot::ApiDescription;
//! use dropshot::ConfigDropshot;
//! use dropshot::ConfigLogging;
//! use dropshot::ConfigLoggingLevel;
//! use dropshot::HandlerTaskMode;
//! use dropshot::ServerBuilder;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), String> {
//!     // Set up a logger.
//!     let log =
//!         ConfigLogging::StderrTerminal {
//!             level: ConfigLoggingLevel::Info,
//!         }
//!         .to_logger("minimal-example")
//!         .map_err(|e| e.to_string())?;
//!
//!     // Describe the API.
//!     let api = ApiDescription::new();
//!     // Register API functions -- see detailed example or ApiDescription docs.
//!
//!     // Start the server.
//!     let server = ServerBuilder::new(api, (), log)
//!         .start()
//!         .map_err(|error| format!("failed to start server: {}", error))?;
//!
//!     server.await
//! }
//! ```
//!
//! This server returns a 404 for all resources because no API functions were
//! registered.  See `examples/basic.rs` for a simple, documented example that
//! provides a few resources using shared state.
//!
//! ## API Handler Functions
//!
//! HTTP talks about **resources**.  For a REST API, we often talk about
//! **endpoints** or **operations**, which are identified by a combination of the
//! HTTP method and the URI path.
//!
//! Example endpoints for a resource called a "project" might include:
//!
//! * `GET /projects` (list projects)
//! * `POST /projects` (one way to create a project)
//! * `GET /projects/my_project` (fetch one project)
//! * `PUT /projects/my_project` (update (or possibly create) a project)
//! * `DELETE /projects/my_project` (delete a project)
//!
//! With Dropshot, an incoming request for a given API endpoint is handled by a
//! particular Rust function.  That function is called an **entrypoint**, an
//! **endpoint handler**, or a **handler function**.  When you set up a Dropshot
//! server, you configure the set of available API endpoints and which functions
//! will handle each one by setting up an [`ApiDescription`].
//!
//! There are two ways to define a set of endpoints:
//!
//! ### As a free function
//!
//! The simplest Dropshot server defines an endpoint as a function, annotated
//! with the [`endpoint`] macro. Here's an example of a single endpoint that
//! lists a hardcoded project:
//!
//! ```
//! use dropshot::endpoint;
//! use dropshot::ApiDescription;
//! use dropshot::HttpError;
//! use dropshot::HttpResponseOk;
//! use dropshot::RequestContext;
//! use http::Method;
//! use schemars::JsonSchema;
//! use serde::Serialize;
//! use std::sync::Arc;
//!
//! /// Represents a project in our API.
//! #[derive(Serialize, JsonSchema)]
//! struct Project {
//!     /// Name of the project.
//!     name: String,
//! }
//!
//! /// Fetch a project.
//! #[endpoint {
//!     method = GET,
//!     path = "/projects/project1",
//! }]
//! async fn myapi_projects_get_project(
//!     rqctx: RequestContext<()>,
//! ) -> Result<HttpResponseOk<Project>, HttpError>
//! {
//!    let project = Project { name: String::from("project1") };
//!    Ok(HttpResponseOk(project))
//! }
//!
//! fn main() {
//!     let mut api = ApiDescription::new();
//!
//!     // Register our endpoint and its handler function.  The "endpoint" macro
//!     // specifies the HTTP method and URI path that identify the endpoint,
//!     // allowing this metadata to live right alongside the handler function.
//!     api.register(myapi_projects_get_project).unwrap();
//!
//!     // ... (use `api` to set up an `HttpServer` )
//! }
//! ```
//!
//! There's quite a lot going on here:
//!
//! * The `endpoint` macro specifies the HTTP method and URI path.  When we
//!   invoke `ApiDescription::register()`, this information is used to register
//!   the endpoint that will be handled by our function.
//! * The signature of our function indicates that on success, it returns a
//!   `HttpResponseOk<Project>`.  This means that the function will
//!   return an HTTP 200 status code ("OK") with an object of type `Project`.
//! * The function itself has a Rustdoc comment that will be used to document
//!   this _endpoint_ in the OpenAPI schema.
//!
//! From this information, Dropshot can generate an OpenAPI specification for
//! this API that describes the endpoint (which OpenAPI calls an "operation"),
//! its documentation, the possible responses that it can return, and the schema
//! for each type of response (which can also include documentation).  This is
//! largely known statically, though generated at runtime.
//!
//! ### As an API trait
//!
//! An **API trait** is a Rust trait that represents a collection of API
//! endpoints. Each endpoint is defined as a static method on the trait, and the
//! trait as a whole is annotated with
//! [`#[dropshot::api_description]`](macro@api_description). (Rust 1.75 or later
//! is required.)
//!
//! While slightly more complex than the function-based server, API traits
//! separate the interface definition from the implementation. Keeping the
//! definition and implementation in different crates can allow for faster
//! iteration of the interface, and simplifies multi-service repos with clients
//! generated from the OpenAPI output of interfaces. In addition, API traits
//! allow for multiple implementations, such as a mock implementation for
//! testing.
//!
//! Here's an example of an API trait that's equivalent to the function-based
//! server above:
//!
//! ```
//! use dropshot::ApiDescription;
//! use dropshot::HttpError;
//! use dropshot::HttpResponseOk;
//! use dropshot::RequestContext;
//! use http::Method;
//! use schemars::JsonSchema;
//! use serde::Serialize;
//! use std::sync::Arc;
//!
//! /// Represents a project in our API.
//! #[derive(Serialize, JsonSchema)]
//! struct Project {
//!     /// Name of the project.
//!     name: String,
//! }
//!
//! /// Defines the trait that captures all the methods.
//! #[dropshot::api_description]
//! trait ProjectApi {
//!     /// The context type used within endpoints.
//!     type Context;
//!
//!     /// Fetch a project.
//!     #[endpoint {
//!         method = GET,
//!         path = "/projects/project1",
//!     }]
//!     async fn myapi_projects_get_project(
//!         rqctx: RequestContext<Self::Context>,
//!     ) -> Result<HttpResponseOk<Project>, HttpError>;
//! }
//!
//! // The `dropshot::api_description` macro generates a module called
//! // `project_api_mod`. This module has a method called `api_description`
//! // that, given an implementation of the trait, returns an `ApiDescription`.
//! // The `ApiDescription` can then be used to set up an `HttpServer`.
//!
//! // --- The following code may be in another crate ---
//!
//! /// An empty type to hold the project server context.
//! ///
//! /// This type is never constructed, and is purely a way to name
//! /// the specific server impl.
//! enum ServerImpl {}
//!
//! impl ProjectApi for ServerImpl {
//!     type Context = ();
//!
//!     async fn myapi_projects_get_project(
//!         rqctx: RequestContext<Self::Context>,
//!     ) -> Result<HttpResponseOk<Project>, HttpError> {
//!         let project = Project { name: String::from("project1") };
//!         Ok(HttpResponseOk(project))
//!     }
//! }
//!
//! fn main() {
//!     // The type of `api` is provided for clarity -- it is generally inferred.
//!     // "api" will automatically register all endpoints defined in the trait.
//!     let mut api: ApiDescription<()> =
//!         project_api_mod::api_description::<ServerImpl>().unwrap();
//!
//!     // ... (use `api` to set up an `HttpServer` )
//! }
//! ```
//!
//! See [`api-trait.rs`] and [`api-trait-alternate.rs`] for working
//! examples.
//!
//! [`api-trait.rs`]:
//!     https://github.com/oxidecomputer/dropshot/blob/main/dropshot/examples/api-trait.rs
//! [`api-trait-alternate.rs`]:
//!     https://github.com/oxidecomputer/dropshot/blob/main/dropshot/examples/api-trait-alternate.rs
//!
//! #### Limitations
//!
//! Currently, the `#[dropshot::api_description]` macro is only supported in module
//! contexts, not within function bodies. This is a Rust limitation -- see [Rust
//! issue #79260](https://github.com/rust-lang/rust/issues/79260) for more
//! details.
//!
//! ### Choosing between functions and traits
//!
//! *Prototyping:* If you're prototyping with a small number of endpoints,
//! functions provide an easier way to get started. The downside to traits is
//! that endpoints signatures are defined at least twice, once in the trait and
//! once in the implementation.
//!
//! *Small services:* For a service that is relatively isolated and quick to
//! compile, traits and functions are both good options.
//!
//! *APIs with multiple implementations:* For services that are large enough to
//! have a second, simpler implementation (of potentially parts of them), a
//! trait is best.
//!
#![cfg_attr(
    feature = "internal-docs",
    doc = "Here's an archetypal way to organize code for a large service with a \
         real and an in-memory test implementation. Each rounded node \
         represents a binary and each rectangular node represents a library \
         crate (or more than one for \"logic\").\n"
)]
#![cfg_attr(feature = "internal-docs", doc = simple_mermaid::mermaid!("../large-service-dep-graph.mmd"))]
//!
//! ### `#[endpoint { ... }]` attribute parameters
//!
//! The `endpoint` attribute accepts parameters the affect the operation of
//! the endpoint as well as metadata that appears in the OpenAPI description
//! of it. For more, see the documentation on [`endpoint`].
//!
//! ### Function parameters
//!
//! In general, a handler function looks like this:
//!
//! ```ignore
//! async fn f(
//!      rqctx: RequestContext<Context>,
//!      [query_params: Query<Q>,]
//!      [path_params: Path<P>,]
//!      [header_params: Header<H>,]
//!      [body_param: TypedBody<J>,]
//!      [body_param: UntypedBody,]
//!      [body_param: StreamingBody,]
//!      [raw_request: RawRequest,]
//! ) -> Result<HttpResponse*, HttpError>
//! ```
//!
//! The `RequestContext` must appear first.  The `Context` type is
//! caller-provided context which is provided when the server is created.
//!
//! The types `Query`, `Path`, `Header`, `TypedBody`, `UntypedBody`, and
//! `RawRequest` are called **Extractors** because they cause information to be
//! pulled out of the request and made available to the handler function.
//!
//! * [`Query`]`<Q>` extracts parameters from a query string, deserializing them
//!   into an instance of type `Q`. `Q` must implement `serde::Deserialize` and
//!   `schemars::JsonSchema`.
//! * [`Path`]`<P>` extracts parameters from HTTP path, deserializing them into
//!   an instance of type `P`. `P` must implement `serde::Deserialize` and
//!   `schemars::JsonSchema`.
//! * [`Header`]`<H>` extracts parameters from HTTP headers, deserializing
//!   them into an instance of type `H`. `H` must implement
//!   `serde::Deserialize` and `schemars::JsonSchema`.
//! * [`TypedBody`]`<J>` extracts content from the request body by parsing the
//!   body as JSON (or form/url-encoded) and deserializing it into an instance
//!   of type `J`. `J` must implement `serde::Deserialize` and `schemars::JsonSchema`.
//! * [`UntypedBody`] extracts the raw bytes of the request body.
//! * [`StreamingBody`] provides the raw bytes of the request body as a
//!   [`Stream`](futures::Stream) of [`Bytes`](bytes::Bytes) chunks.
//! * [`RawRequest`] provides access to the underlying [`hyper::Request`].  The
//!   hope is that this would generally not be needed.  It can be useful to
//!   implement functionality not provided by Dropshot.
//!
//! `Query` and `Path` impl `SharedExtractor`.  `TypedBody`, `UntypedBody`,
//! `StreamingBody`, and `RawRequest` impl `ExclusiveExtractor`.  Your function
//! may accept 0-3 extractors, but only one can be `ExclusiveExtractor`, and it
//! must be the last one.  Otherwise, the order of extractor arguments does not
//! matter.
//!
//! If the handler accepts any extractors and the corresponding extraction
//! cannot be completed, the request fails with status code 400 and an error
//! message reflecting the error (usually a validation error).
//!
//! As with any serde-deserializable type, you can make fields optional by having
//! the corresponding property of the type be an `Option`.  Here's an example of
//! an endpoint that takes two arguments via query parameters: "limit", a
//! required u32, and "marker", an optional string:
//!
//! ```
//! use http::StatusCode;
//! use dropshot::HttpError;
//! use dropshot::TypedBody;
//! use dropshot::Query;
//! use dropshot::RequestContext;
//! use dropshot::Body;
//! use hyper::Response;
//! use schemars::JsonSchema;
//! use serde::Deserialize;
//! use std::sync::Arc;
//!
//! #[derive(Deserialize, JsonSchema)]
//! struct MyQueryArgs {
//!     limit: u32,
//!     marker: Option<String>
//! }
//!
//! struct MyContext {}
//!
//! async fn myapi_projects_get(
//!     rqctx: RequestContext<MyContext>,
//!     query: Query<MyQueryArgs>)
//!     -> Result<Response<Body>, HttpError>
//! {
//!     let query_args = query.into_inner();
//!     let context: &MyContext = rqctx.context();
//!     let limit: u32 = query_args.limit;
//!     let marker: Option<String> = query_args.marker;
//!     Ok(Response::builder()
//!         .status(StatusCode::OK)
//!         .body(format!("limit = {}, marker = {:?}\n", limit, marker).into())?)
//! }
//! ```
//!
//! ### Endpoint function return types
//!
//! Endpoint handler functions are async, so they always return a `Future`.  When
//! we say "return type" below, we use that as shorthand for the output of the
//! future.
//!
//! An endpoint function must return a [`Result`]`<T, E>` where the `Ok` type
//! implements [`HttpResponse`].  Typically this should be a type that
//! implements `HttpTypedResponse` (either one of the Dropshot-provided ones or
//! one of your own creation).
//!
//! Endpoint functions may return the [`HttpError`] type in the error case, or a
//! user-defined error type that implements the [`HttpResponseError`] trait.
//! [`HttpError`] may be simpler, while a custom error type permits greater
//! expressivity and control over the representation of errors in the API. See
//! the documentation for the [`HttpResponseError`] trait for details on how to
//! implement it for your own error types.
//!
//! The more specific a type returned by the handler function, the more can be
//! validated at build-time, and the more specific an OpenAPI schema can be
//! generated from the source code.  For example, a POST to an endpoint
//! "/projects" might return `Result<HttpResponseCreated<Project>, HttpError>`.
//! As you might expect, on success, this turns into an HTTP 201 "Created"
//! response whose body is constructed by serializing the `Project`.  In this
//! example, OpenAPI tooling can identify at build time that this function
//! produces a 201 "Created" response on success with a body whose schema matches
//! `Project` (which we already said implements `Serialize`), and there would be
//! no way to violate this contract at runtime.
//!
//! These are the implementations of `HttpTypedResponse` with their associated
//! HTTP response code
//! on the HTTP method:
//!
//! | Return Type | HTTP status code |
//! | ----------- | ---------------- |
//! | [`HttpResponseOk`] | 200 |
//! | [`HttpResponseCreated`] | 201 |
//! | [`HttpResponseAccepted`] | 202 |
//! | [`HttpResponseDeleted`] | 204 |
//! | [`HttpResponseUpdatedNoContent`] | 204 |
//!
//! In situations where the response schema is not fixed, the endpoint should
//! return `Response<Body>`, which also implements `HttpResponse`. Note that
//! the OpenAPI spec will not include any status code or type information in
//! this case.
//!
//! ## What about generic handlers that run on all requests?
//!
//! There's no mechanism in Dropshot for this.  Instead, it's recommended that
//! users commonize code using regular Rust functions and calling them.  See the
//! design notes in the README for more on this.
//!
//! ### Generating OpenAPI documents
//!
//! For a given `ApiDescription`, you can also print out an [OpenAPI
//! document](https://spec.openapis.org/oas/v3.1.0#openapi-document) describing
//! the API.  See [`ApiDescription::openapi`].
//!
//! With API traits, the `#[dropshot::api_description]` macro generates a helper
//! function called `stub_api_description`, which returns an `ApiDescription`
//! not backed by an implementation. This _stub description_ can be used to
//! generate an OpenAPI document for the trait without requiring an
//! implementation of the trait. For example:
//!
//! ```
//! # use dropshot::ApiDescription;
//! # use dropshot::HttpError;
//! # use dropshot::HttpResponseOk;
//! # use dropshot::RequestContext;
//! # use http::Method;
//! # use schemars::JsonSchema;
//! # use serde::Serialize;
//! # use std::sync::Arc;
//! #
//! # #[derive(Serialize, JsonSchema)]
//! # struct Project {
//! #     name: String,
//! # }
//! /// This is the API trait defined above.
//! #[dropshot::api_description]
//! trait ProjectApi {
//!     type Context;
//!     #[endpoint {
//!         method = GET,
//!         path = "/projects/project1",
//!     }]
//!     async fn myapi_projects_get_project(
//!         rqctx: RequestContext<Self::Context>,
//!     ) -> Result<HttpResponseOk<Project>, HttpError>;
//! }
//!
//! # // defining fn main puts the doctest in a module context
//! # fn main() {
//! let description = project_api_mod::stub_api_description().unwrap();
//! let mut openapi = description
//!     .openapi("Project Server", semver::Version::new(1, 0, 0));
//! openapi.write(&mut std::io::stdout().lock()).unwrap();
//! # }
//! ```
//!
//! A stub description must not be used for an actual server: all request
//! handlers will immediately panic.
//!
//! ## Support for paginated resources
//!
//! "Pagination" here refers to the interface pattern where HTTP resources (or
//! API endpoints) that provide a list of the items in a collection return a
//! relatively small maximum number of items per request, often called a "page"
//! of results.  Each page includes some metadata that the client can use to make
//! another request for the next page of results.  The client can repeat this
//! until they've gotten all the results.  Limiting the number of results
//! returned per request helps bound the resource utilization and time required
//! for any request, which in turn facilities horizontal scalability, high
//! availability, and protection against some denial of service attacks
//! (intentional or otherwise).  For more background, see the comments in
//! dropshot/src/pagination.rs.
//!
//! Pagination support in Dropshot implements this common pattern:
//!
//! * This server exposes an **API endpoint** that returns the **items**
//!   contained within a **collection**.
//! * The client is not allowed to list the entire collection in one request.
//!   Instead, they list the collection using a sequence of requests to the one
//!   endpoint.  We call this sequence of requests a **scan** of the collection,
//!   and we sometimes say that the client **pages through** the collection.
//! * The initial request in the scan may specify the **scan parameters**, which
//!   typically specify how the results are to be sorted (i.e., by which
//!   field(s) and whether the sort is ascending or descending), any filters to
//!   apply, etc.
//! * Each request returns a **page** of results at a time, along with a **page
//!   token** that's provided with the next request as a query parameter.
//! * The scan parameters cannot change between requests that are part of the
//!   same scan.
//! * With all requests: there's a default limit (e.g., 100 items returned at a
//!   time).  Clients can request a higher limit using a query parameter (e.g.,
//!   `limit=1000`).  This limit is capped by a hard limit on the server.  If the
//!   client asks for more than the hard limit, the server can use the hard limit
//!   or reject the request.
//!
//! As an example, imagine that we have an API endpoint called `"/animals"`.  Each
//! item returned is an `Animal` object that might look like this:
//!
//! ```json
//! {
//!     "name": "aardvark",
//!     "class": "mammal",
//!     "max_weight": "80", /* kilograms, typical */
//! }
//! ```
//!
//! There are at least 1.5 million known species of animal -- too many to return
//! in one API call!  Our API supports paginating them by `"name"`, which we'll
//! say is a unique field in our data set.
//!
//! The first request to the API fetches `"/animals"` (with no querystring
//! parameters) and returns:
//!
//! ```json
//! {
//!     "next_page": "abc123...",
//!     "items": [
//!         {
//!             "name": "aardvark",
//!             "class": "mammal",
//!             "max_weight": "80",
//!         },
//!         ...
//!         {
//!             "name": "badger",
//!             "class": "mammal",
//!             "max_weight": "12",
//!         }
//!     ]
//! }
//! ```
//!
//! The subsequent request to the API fetches `"/animals?page_token=abc123..."`.
//! The page token `"abc123..."` is an opaque token to the client, but typically
//! encodes the scan parameters and the value of the last item seen
//! (`"name=badger"`).  The client knows it has completed the scan when it
//! receives a response with no `next_page` in it.
//!
//! Our API endpoint can also support scanning in reverse order.  In this case,
//! when the client makes the first request, it should fetch
//! `"/animals?sort=name-descending"`.  Now the first result might be `"zebra"`.
//! Again, the page token must include the scan parameters so that in subsequent
//! requests, the API endpoint knows that we're scanning backwards, not forwards,
//! from the value we were given.  It's not allowed to change directions or sort
//! order in the middle of a scan.  (You can always start a new scan, but you
//! can't pick up from where you were in the previous scan.)
//!
//! It's also possible to support sorting by multiple fields.  For example, we
//! could support `sort=class-name`, which we could define to mean that we'll
//! sort the results first by the animal's class, then by name.  Thus we'd get
//! all the amphibians in sorted order, then all the mammals, then all the
//! reptiles.  The main requirement is that the combination of fields used for
//! pagination must be unique.  We cannot paginate by the animal's class alone.
//! (To see why: there are over 6,000 mammals.  If the page size is, say, 1000,
//! then the page_token would say `"mammal"`, but there's not enough information
//! there to see where we are within the list of mammals.  It doesn't matter
//! whether there are 2 mammals or 6,000 because clients can limit the page size
//! to just one item if they want and that ought to work.)
//!
//!
//! ### Dropshot interfaces for pagination
//!
//! The interfaces for pagination include:
//!
//! * input: your paginated API endpoint's handler function should accept an
//!   argument of type [`Query`]`<`[`PaginationParams`]`<ScanParams,
//!   PageSelector>>`, where you define `ScanParams` and `PageSelector` (see
//!   `PaginationParams` for more on this.)
//!
//! * output: your paginated API endpoint's handler function can return
//!   `Result<`[`HttpResponseOk`]<[`ResultsPage`]`<T>, HttpError>` where `T:
//!   Serialize` is the item listed by the endpoint.  You can also use your own
//!   structure that contains a [`ResultsPage`] (possibly using
//!   `#[serde(flatten)]`), if that's the behavior you want.
//!
//! See the complete, documented pagination examples in the "examples"
//! directory for more on how to use these.
//!
//! ### Advanced usage notes
//!
//! It's possible to accept additional query parameters besides the pagination
//! parameters by having your API endpoint handler function take two different
//! arguments using `Query`, like this:
//!
//! ```
//! use dropshot::HttpError;
//! use dropshot::HttpResponseOk;
//! use dropshot::PaginationParams;
//! use dropshot::Query;
//! use dropshot::RequestContext;
//! use dropshot::ResultsPage;
//! use dropshot::endpoint;
//! use schemars::JsonSchema;
//! use serde::Deserialize;
//! use std::sync::Arc;
//! # use serde::Serialize;
//! # #[derive(Debug, Deserialize, JsonSchema)]
//! # enum MyScanParams { A };
//! # #[derive(Debug, Deserialize, JsonSchema, Serialize)]
//! # enum MyPageSelector { A(String) };
//! #[derive(Deserialize, JsonSchema)]
//! struct MyExtraQueryParams {
//!     do_extra_stuff: bool,
//! }
//!
//! #[endpoint {
//!     method = GET,
//!     path = "/list_stuff"
//! }]
//! async fn my_list_api(
//!     rqctx: RequestContext<()>,
//!     pag_params: Query<PaginationParams<MyScanParams, MyPageSelector>>,
//!     extra_params: Query<MyExtraQueryParams>,
//! ) -> Result<HttpResponseOk<ResultsPage<String>>, HttpError>
//! {
//!  # unimplemented!();
//!  /* ... */
//! }
//! ```
//!
//! You might expect that instead of doing this, you could define your own
//! structure that includes a `PaginationParams` using `#[serde(flatten)]`, and
//! this ought to work, but it currently doesn't due to serde_urlencoded#33,
//! which is really serde#1183.
//!
//! Note that any parameters defined by `MyScanParams` are effectively encoded
//! into the page token and need not be supplied with invocations when `page_token`
//! is specified. That is not the case for required parameters defined by
//! `MyExtraQueryParams`--those must be supplied on each invocation.
//!
//! ### OpenAPI extension
//!
//! In generated OpenAPI documents, Dropshot adds the `x-dropshot-pagination`
//! extension to paginated operations. The value is currently a structure
//! with this format:
//! ```json
//! {
//!     "required": [ .. ]
//! }
//! ```
//!
//! The string values in the `required` array are the names of those query
//! parameters that are mandatory if `page_token` is not specified (when
//! fetching the first page of data).
//!
//! ## API Versioning
//!
//! Dropshot servers can host multiple versions of an API.  See
//! dropshot/examples/versioning.rs for a complete, working, commented example
//! that uses a client-provided header to determine which API version to use for
//! each incoming request.
//!
//! API versioning basically works like this:
//!
//! 1. When using the `endpoint` macro to define an endpoint, you specify a
//!    `versions` field as a range of [semver](https://semver.org/) version
//!    strings.  This identifies what versions of the API this endpoint
//!    implementation appears in.  Examples:
//!
//! ```text
//! // introduced in 1.0.0, present in all subsequent versions
//! versions = "1.0.0"..
//!
//! // removed in 2.0.0, present in all previous versions
//! // (not present in 2.0.0 itself)
//! versions = .."2.0.0"
//!
//! // introduced in 1.0.0, removed in 2.0.0
//! // (present only in all 1.x versions, NOT 2.0.0 or later)
//! versions = "1.0.0".."2.0.0"
//!
//! // present in all versions (the default)
//! versions = ..
//! ```
//!
//! You can also use identifiers, as in:
//!
//! ```text
//! const V_MY_FEATURE_ADDED: semver::Version = semver::Version::new(1, 2, 3);
//! // They can even be inside a module
//! mod my_mod {
//!     const V_MY_FEATURE_REMOVED: semver::Version =
//!         semver::Version::new(4, 5, 6);
//! }
//!
//! // introduced in 1.2.3 and removed in 4.5.6.
//! versions = V_MY_FEATURE_ADDED..my_mod::V_MY_FEATURE_REMOVED
//! ```
//!
//! 2. When constructing the server, you provide [`VersionPolicy::Dynamic`] with
//!    your own impl of [`DynamicVersionPolicy`] that tells Dropshot how to
//!    determine which API version to use for each request.
//!
//! 3. When a request arrives for a server using `VersionPolicy::Dynamic`,
//!    Dropshot uses the provided impl to determine the appropriate API version.
//!    Then it routes requests by HTTP method and path (like usual) but only
//!    considers endpoints whose version range matches the requested API
//!    version.
//!
//! 4. When generating an OpenAPI document for your `ApiDescription`, you must
//!    provide a specific version to generate it _for_.  It will only include
//!    endpoints present in that version and types referenced by those
//!    endpoints.
//!
//! It is illegal to register multiple endpoints for the same HTTP method and
//! path with overlapping version ranges.
//!
//! All versioning-related configuration is optional.  You can ignore it
//! altogether by simply not specifying `versions` for each endpoint and not
//! providing a `VersionPolicy` for the server (or, equivalently, providing
//! `VersionPolicy::Unversioned`).  In this case, the server does not try to
//! determine a version for incoming requests.  It routes requests to handlers
//! without considering API versions.
//!
//! It's maybe surprising that this mechanism only talks about versioning
//! endpoints, but usually when we think about API versioning we think about
//! types, especially the input and output types.  This works because the
//! endpoint implementation itself specifies the input and output types.  Let's
//! look at an example.
//!
//! Suppose you have version 1.0.0 of an API with an endpoint `my_endpoint` with
//! a body parameter `TypedBody<MyArg>`.  You want to make a breaking change to
//! the API, creating version 2.0.0 where `MyArg` has a new required field.  You
//! still want to support API version 1.0.0.  Here's one clean way to do this:
//!
//! 1. Mark the existing `my_endpoint` as removed after 1.0.0:
//!     1. Move the `my_endpoint` function _and_ its input type `MyArg` to a
//!        new module called `v1`.  (You'd also move its output type here if
//!        that's changing.)
//!     2. Change the `endpoint` macro invocation on `my_endpoint` to say
//!        `versions = ..1.0.0`.  This says that it was removed after 1.0.0.
//! 2. Create a new endpoint that appears in 2.0.0.
//!     1. Create a new module called `v2`.
//!     2. In `v2`, create a new type `MyArg` that looks the way you want it to
//!        appear in 2.0.0.  (You'd also create new versions of the output
//!        types, if those are changing, too).
//!     3. Also in `v2`, create a new `my_endpoint` function that accepts and
//!        returns the `v2` new versions of the types.  Its `endpoint` macro
//!        will say `versions = 2.0.0`.
//!
//! As mentioned above, you will also need to create your server with
//! `VersionPolicy::Dynamic` and specify how Dropshot should determine which
//! version to use for each request.  But that's it!  Having done this:
//!
//! * If you generate an OpenAPI doc for version 1.0.0, Dropshot will include
//!   `v1::my_endpoint` and its types.
//! * If you generate an OpenAPI doc for version 2.0.0, Dropshot will include
//!   `v2::my_endpoint` and its types.
//! * If a request comes in for version 1.0.0, Dropshot will route it to
//!   `v1::my_endpoint` and so parse the body as `v1::MyArg`.
//! * If a request comes in for version 2.0.0, Dropshot will route it to
//!   `v2::my_endpoint` and so parse the body as `v2::MyArg`.
//!
//! To see a completed example of this, see dropshot/examples/versioning.rs.
//!
//! ## DTrace probes
//!
//! Dropshot optionally exposes two DTrace probes, `request_start` and
//! `request_finish`. These provide detailed information about each request,
//! such as their ID, the local and remote IPs, and the response information.
//! See the `dropshot::dtrace::RequestInfo` and `dropshot::dtrace::ResponseInfo`
//! types for a complete listing of what's available.
//!
//! These probes are implemented via the `usdt` crate. They may require a
//! nightly toolchain if built on macOS prior to Rust version 1.66. Otherwise a
//! stable compiler >= v1.59 is required in order to present the necessary
//! features. Given these constraints, USDT functionality is behind the feature
//! flag `"usdt-probes"`, which may become a default feature of this crate in
//! future releases.
//!
//! > *Important:* The probes are internally registered with the DTrace kernel
//! module, making them visible via `dtrace(1M)`. This is done when an `HttpServer`
//! object is created, but it's possible that registration fails. The result of
//! registration is stored in the server after creation, and can be accessed with
//! the [`HttpServer::probe_registration()`] method. This allows callers to decide
//! how to handle failures, but ensures that probes are always enabled if possible.
//!
//! Once in place, the probes can be seen via DTrace. For example, running:
//!
//! ```text
//! $ cargo +nightly run --example basic --features usdt-probes
//! ```
//!
//! And making several requests to it with `curl`, we can see the DTrace
//! probes with an invocation like:
//!
//! ```text
//! ## dtrace -Zq -n 'dropshot*:::request-* { printf("%s\n", copyinstr(arg0)); }'
//! {"ok":{"id":"b793c62e-60e4-45c5-9274-198a04d9abb1","local_addr":"127.0.0.1:61028","remote_addr":"127.0.0.1:34286","method":"GET","path":"/counter","query":null}}
//! {"ok":{"id":"b793c62e-60e4-45c5-9274-198a04d9abb1","local_addr":"127.0.0.1:61028","remote_addr":"127.0.0.1:34286","status_code":200,"message":""}}
//! {"ok":{"id":"9050e30a-1ce3-4d6f-be1c-69a11c618800","local_addr":"127.0.0.1:61028","remote_addr":"127.0.0.1:41101","method":"PUT","path":"/counter","query":null}}
//! {"ok":{"id":"9050e30a-1ce3-4d6f-be1c-69a11c618800","local_addr":"127.0.0.1:61028","remote_addr":"127.0.0.1:41101","status_code":400,"message":"do not like the number 10"}}
//! {"ok":{"id":"a53696af-543d-452f-81b6-5a045dd9921d","local_addr":"127.0.0.1:61028","remote_addr":"127.0.0.1:57376","method":"PUT","path":"/counter","query":null}}
//! {"ok":{"id":"a53696af-543d-452f-81b6-5a045dd9921d","local_addr":"127.0.0.1:61028","remote_addr":"127.0.0.1:57376","status_code":204,"message":""}}
//! ```

// The `usdt` crate may require nightly, enabled if our consumer is enabling
// DTrace probes.
#![cfg_attr(all(feature = "usdt-probes", usdt_need_asm), feature(asm))]
#![cfg_attr(
    all(feature = "usdt-probes", target_os = "macos", usdt_need_asm_sym),
    feature(asm_sym)
)]

// The macro used to define DTrace probes needs to be defined before anything
// that might use it.
mod dtrace;

mod api_description;
mod body;
mod config;
mod error;
mod error_status_code;
mod extractor;
mod from_map;
mod handler;
mod http_util;
mod logging;
#[cfg(feature = "otel-tracing")]
pub mod otel;
mod pagination;
mod router;
mod schema_util;
mod server;
mod to_map;
#[cfg(any(feature = "tracing", feature = "otel-tracing"))]
pub mod tracing_support;
mod type_util;
mod versioning;
mod websocket;

pub mod test_util;

#[macro_use]
extern crate slog;

pub use api_description::ApiDescription;
pub use api_description::ApiDescriptionBuildErrors;
pub use api_description::ApiDescriptionRegisterError;
pub use api_description::ApiEndpoint;
pub use api_description::ApiEndpointBodyContentType;
pub use api_description::ApiEndpointParameter;
pub use api_description::ApiEndpointParameterLocation;
pub use api_description::ApiEndpointResponse;
pub use api_description::ApiEndpointVersions;
pub use api_description::EndpointTagPolicy;
pub use api_description::ExtensionMode;
pub use api_description::OpenApiDefinition;
pub use api_description::StubContext;
pub use api_description::TagConfig;
pub use api_description::TagDetails;
pub use api_description::TagExternalDocs;
pub use body::Body;
pub use config::ConfigDropshot;
pub use config::ConfigTls;
pub use config::HandlerTaskMode;
pub use config::RawTlsConfig;
pub use dtrace::ProbeRegistration;
pub use error::HttpError;
pub use error::HttpErrorResponseBody;
pub use error_status_code::ClientErrorStatusCode;
pub use error_status_code::ErrorStatusCode;
pub use error_status_code::InvalidClientErrorStatusCode;
pub use error_status_code::InvalidErrorStatusCode;
pub use error_status_code::NotAClientError;
pub use error_status_code::NotAnError;
pub use extractor::ExclusiveExtractor;
pub use extractor::ExtractorMetadata;
pub use extractor::Header;
pub use extractor::MultipartBody;
pub use extractor::Path;
pub use extractor::Query;
pub use extractor::RawRequest;
pub use extractor::SharedExtractor;
pub use extractor::StreamingBody;
pub use extractor::TypedBody;
pub use extractor::UntypedBody;
pub use handler::http_response_found;
pub use handler::http_response_see_other;
pub use handler::http_response_temporary_redirect;
pub use handler::FreeformBody;
pub use handler::HttpCodedResponse;
pub use handler::HttpResponse;
pub use handler::HttpResponseAccepted;
pub use handler::HttpResponseCreated;
pub use handler::HttpResponseDeleted;
pub use handler::HttpResponseError;
pub use handler::HttpResponseFound;
pub use handler::HttpResponseHeaders;
pub use handler::HttpResponseOk;
pub use handler::HttpResponseSeeOther;
pub use handler::HttpResponseTemporaryRedirect;
pub use handler::HttpResponseUpdatedNoContent;
pub use handler::NoHeaders;
pub use handler::RequestContext;
pub use handler::RequestEndpointMetadata;
pub use handler::RequestInfo;
pub use http_util::CONTENT_TYPE_JSON;
pub use http_util::CONTENT_TYPE_MULTIPART_FORM_DATA;
pub use http_util::CONTENT_TYPE_NDJSON;
pub use http_util::CONTENT_TYPE_OCTET_STREAM;
pub use http_util::CONTENT_TYPE_URL_ENCODED;
pub use http_util::HEADER_REQUEST_ID;
pub use logging::ConfigLogging;
pub use logging::ConfigLoggingIfExists;
pub use logging::ConfigLoggingLevel;
pub use pagination::EmptyScanParams;
pub use pagination::PaginationOrder;
pub use pagination::PaginationParams;
pub use pagination::ResultsPage;
pub use pagination::WhichPage;
pub use server::BuildError;
pub use server::ServerBuilder;
pub use server::ServerContext;
pub use server::ShutdownWaitFuture;
pub use server::{HttpServer, HttpServerStarter};
pub use versioning::ClientSpecifiesVersionInHeader;
pub use versioning::DynamicVersionPolicy;
pub use versioning::VersionPolicy;
pub use websocket::WebsocketChannelResult;
pub use websocket::WebsocketConnection;
pub use websocket::WebsocketConnectionRaw;
pub use websocket::WebsocketEndpointResult;
pub use websocket::WebsocketUpgrade;

// Users of the `endpoint` macro need the following macros:
pub use handler::RequestContextArgument;
pub use http::Method;

extern crate dropshot_endpoint;

/// Generates a Dropshot API description from a trait.
///
/// An API trait consists of:
///
/// 1. A context type, typically `Self::Context`, values of which are shared
///    across all the endpoints.
/// 2. A set of endpoint methods, each of which is an `async fn` defined with
///    the same constraints, and via the same syntax, as [`macro@endpoint`] or
///    [`macro@channel`].
///
/// API traits can also have arbitrary non-endpoint items, such as helper
/// functions.
///
/// The macro performs a number of checks on endpoint methods, and produces the
/// following items:
///
/// * The trait itself, with the following modifications to enable use as a
///   Dropshot API:
///
///     1. The trait itself has a `'static` bound added to it.
///     2. The context type has a `dropshot::ServerContext + 'static` bound
///        added to it, making it `Send + Sync + 'static`.
///     3. Each endpoint `async fn` is modified to become a function that
///        returns a `Send + 'static` future. (Implementations can continue to
///        define endpoints via the `async fn` syntax.)
///
///   Non-endpoint items are left unchanged.
///
/// * A support module, typically with the same name as the trait but in
///   `snake_case`, with two functions:
///
///     1. `api_description()`, which accepts an implementation of the trait as
///        a type argument and generates an `ApiDescription`.
///     2. `stub_api_description()`, which generates a _stub_ `ApiDescription`
///        that can be used to generate an OpenAPI spec without having an
///        implementation of the trait available.
///
/// For more information about API traits, see the Dropshot crate-level
/// documentation.
///
/// ## Arguments
///
/// The `#[dropshot::api_description]` macro accepts these arguments:
///
/// * `context`: The type of the context on the trait. Optional, defaults to
///   `Self::Context`.
/// * `module`: The name of the support module. Optional, defaults to the
///   `{T}_mod`, where `T` is the snake_case version of the trait name.
///
///    For example, for a trait called `MyApi` the corresponding module name
///    would be `my_api_mod`.
///
///    (The suffix `_mod` is added to module names so that a crate called
///    `my-api` can define a trait `MyApi`, avoiding name conflicts.)
/// * `tag_config`: Trait-wide tag configuration. _Optional._ For more
///   information, see [_Tag configuration_](#tag-configuration) below.
///
/// ### Example: specify a custom context type
///
/// ```
/// use dropshot::{RequestContext, HttpResponseUpdatedNoContent, HttpError};
///
/// #[dropshot::api_description { context = MyContext }]
/// trait MyTrait {
///     type MyContext;
///
///     #[endpoint {
///         method = PUT,
///         path = "/test",
///     }]
///     async fn put_test(
///         rqctx: RequestContext<Self::MyContext>,
///     ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
/// }
/// # // defining fn main puts the doctest in a module context
/// # fn main() {}
/// ```
///
/// ### Tag configuration
///
/// Endpoints can have tags associated with them that appear in the OpenAPI
/// document. These provide a means of organizing an API. Use the `tag_config`
/// argument to specify tag information for the trait.
///
/// `tag_config` is optional. If not specified, the default is to allow any tag
/// value and any number of tags associated with the endpoint.
///
/// If `tag_config` is specified, compliance with it is evaluated at runtime
/// while registering endpoints. A failure to comply--for example, if `policy =
/// at_least_one` is specified and some endpoint has no associated tags--results
/// in the `api_description` and `stub_api_description` functions returning an
/// error.
///
/// The shape of `tag_config` is broadly similar to that of [`TagConfig`]. It
/// has the following fields:
///
/// * `tags`: A map of tag names with information about them. _Required, but can
///   be empty._
///
///   The keys are tag names, which are strings. The values are objects that
///   consist of:
///
///   * `description`: A string description of the tag. _Optional._
///   * `external_docs`: External documentation for the tag. _Optional._ This
///     has the following fields:
///     * `description`: A string description of the external documentation.
///       _Optional._
///     * `url`: The URL for the external documentation. _Required._
///
/// * `allow_other_tags`: Whether to allow tags not explicitly defined in
///   `tags`. _Optional, defaults to false. But if `tag_config` as a whole is
///   not specified, all tags are allowed._
///
/// * `policy`: Must be an expression of type `EndpointTagPolicy`; typically just
///   the enum variant.
///
///   _Optional, defaults to `EndpointTagPolicy::Any`._
///
/// ### Example: tag configuration
///
/// ```
/// use dropshot::{
///     EndpointTagPolicy, RequestContext, HttpResponseUpdatedNoContent,
///     HttpError,
/// };
///
/// #[dropshot::api_description {
///     tag_config = {
///         // If tag_config is specified, tags is required (but can be empty).
///         tags = {
///             "tag1" = {
///                 // The description is optional.
///                 description = "Tag 1",
///                 // external_docs is optional.
///                 external_docs = {
///                     // The description is optional.
///                     description = "External docs for tag1",
///                     // If external_docs is present, url is required.
///                     url = "https://example.com/tag1",
///                 },
///             },
///         },
///         policy = EndpointTagPolicy::ExactlyOne,
///         allow_other_tags = false,
///     },
/// }]
/// trait MyTrait {
///     type Context;
///
///     #[endpoint {
///         method = PUT,
///         path = "/test",
///         tags = ["tag1"],
///     }]
///     async fn put_test(
///         rqctx: RequestContext<Self::Context>,
///     ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
/// }
/// # // defining fn main puts the doctest in a module context
/// # fn main() {}
/// ```
///
/// ## Limitations
///
/// Currently, the `#[dropshot::api_description]` macro is only supported in
/// module contexts, not function definitions. This is a Rust limitation -- see
/// [Rust issue #79260](https://github.com/rust-lang/rust/issues/79260) for more
/// details.
///
/// ## More information
///
/// For more information about the design decisions behind API traits, see
/// [Oxide RFD 479](https://rfd.shared.oxide.computer/rfd/0479).
pub use dropshot_endpoint::api_description;

/// Transforms a WebSocket handler function into a Dropshot endpoint.
///
/// The transformed function is suitable to be used as a parameter to
/// [`ApiDescription::register()`].
///
/// As with [`macro@endpoint`], this attribute turns a handler function into a
/// Dropshot endpoint, but first wraps the handler function in such a way
/// that is spawned asynchronously and given the upgraded connection of
/// the given `protocol` (i.e. `WEBSOCKETS`).
///
/// The first argument still must be a `RequestContext<_>`.
///
/// The last argument passed to the handler function must be a
/// [`WebsocketConnection`].
///
/// The function must return a [`WebsocketChannelResult`] (which is a
/// general-purpose `Result<(), Box<dyn Error + Send + Sync + 'static>>`).
/// Returned error values will be written to the RequestContext's log.
///
/// ```ignore
/// #[dropshot::channel { protocol = WEBSOCKETS, path = "/my/ws/channel/{id}" }]
/// ```
pub use dropshot_endpoint::channel;

/// Transforms an HTTP handler function into a Dropshot endpoint.
///
/// The transformed function is suitable to be used as a parameter to
/// [`ApiDescription::register()`].
///
/// The arguments to this macro encode information relevant to the operation of
/// an API endpoint beyond what is expressed by the parameter and return types
/// of a handler function.
///
/// ## Arguments
///
/// The `#[dropshot::endpoint]` macro accepts the following arguments:
///
/// * `method`: The [HTTP request method] (HTTP verb) for the endpoint. Can be
///   one of `DELETE`, `HEAD`, `GET`, `OPTIONS`, `PATCH`, `POST`, or `PUT`.
///   Required.
/// * `path`: The path to the endpoint, along with path variables. Path
///   variables are enclosed in curly braces. For example, `path =
///   "/widget/{id}"`. Required.
/// * `tags`: An array of [OpenAPI tags] for the operation. Optional, defaults
///   to an empty list.
/// * `versions`: API versions for which the endpoint is valid. Optional. For more, see
///   [API Versioning].
/// * `content_type`: The media type used to encode the request body. Can be one
///   of `application/json`, `application/x-www-form-urlencoded`, or
///   `multipart/form-data`. Optional, defaults to `application/json`.
/// * `deprecated`: A boolean indicating whether the operation is marked
///   deprecated in the OpenAPI document. Optional, defaults to false.
/// * `unpublished`: A boolean indicating whether the operation is omitted from
///   the OpenAPI document. Optional, defaults to false.
/// * `request_body_max_bytes`: The maximum size of the request body in bytes.
///   Accepts literals as well as constants of type `usize`. Optional, defaults
///   to the server configuration's `default_request_body_max_bytes`.
///
/// [HTTP request method]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Methods
/// [OpenAPI tags]: https://swagger.io/docs/specification/v3_0/grouping-operations-with-tags/
/// [API Versioning]: crate#api-versioning
///
/// ### Example: configuring an endpoint
///
/// ```ignore
/// const LARGE_REQUEST_BODY_MAX_BYTES: usize = 1 * 1024 * 1024;
///
/// #[endpoint {
///     // --- Required fields ---
///     // The HTTP method for the endpoint
///     method = { DELETE | HEAD | GET | OPTIONS | PATCH | POST | PUT },
///     // The path to the endpoint, along with path variables
///     path = "/path/name/with/{named}/{variables}",
///
///     // --- Optional fields ---
///     // Tags for the operation's description
///     tags = [ "all", "your", "OpenAPI", "tags" ],
///     // API versions for which the endpoint is valid
///     versions = "1.0.0".."2.0.0",
///     // The media type used to encode the request body
///     content_type = { "application/json" | "application/x-www-form-urlencoded" | "multipart/form-data" }
///     // True if the operation is deprecated
///     deprecated = { true | false },
///     // True causes the operation to be omitted from the API description
///     unpublished = { true | false },
///     // Maximum request body size in bytes
///     request_body_max_bytes = LARGE_REQUEST_BODY_MAX_BYTES,
/// }]
/// async fn my_endpoint(/* ... */) -> Result<HttpResponseOk, HttpError> {
///     // ...
/// }
/// ```
pub use dropshot_endpoint::endpoint;
