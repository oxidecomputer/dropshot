// Copyright 2026 Oxide Computer Company

use dropshot::{
    api_to_typespec, endpoint, http_response_see_other, ApiDescription,
    Body, FreeformBody, Header, HttpError, HttpResponseAccepted,
    HttpResponseCreated, HttpResponseDeleted, HttpResponseHeaders,
    HttpResponseOk, HttpResponseSeeOther, HttpResponseUpdatedNoContent,
    PaginationParams, Path, Query, RequestContext, ResultsPage, TypedBody,
    UntypedBody,
};
use http::Response;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// A simple GET endpoint with no parameters.
#[endpoint {
    method = GET,
    path = "/ping",
}]
/// Health check
async fn ping(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

// A GET endpoint that returns a typed response.
#[derive(Serialize, Deserialize, JsonSchema)]
struct Widget {
    /// The widget's name
    name: String,
    /// How many we have
    count: u32,
    /// Is it enabled?
    enabled: bool,
}

#[derive(Deserialize, JsonSchema)]
#[allow(dead_code)]
struct WidgetPathArgs {
    id: String,
}

#[endpoint {
    method = GET,
    path = "/widgets/{id}",
}]
/// Get a widget
async fn widget_get(
    _rqctx: RequestContext<()>,
    _path: Path<WidgetPathArgs>,
) -> Result<HttpResponseOk<Widget>, HttpError> {
    unimplemented!();
}

// A POST endpoint with a typed body.
#[derive(Deserialize, JsonSchema)]
#[allow(dead_code)]
struct WidgetCreate {
    /// The widget's name
    name: String,
    /// Optional description
    description: Option<String>,
}

#[endpoint {
    method = POST,
    path = "/widgets",
}]
/// Create a widget
async fn widget_create(
    _rqctx: RequestContext<()>,
    _body: TypedBody<WidgetCreate>,
) -> Result<HttpResponseCreated<Widget>, HttpError> {
    unimplemented!();
}

// A PUT endpoint with query params and path params.
#[derive(Deserialize, JsonSchema)]
#[allow(dead_code)]
struct UpdateQuery {
    /// Force the update?
    force: Option<bool>,
}

#[endpoint {
    method = PUT,
    path = "/widgets/{id}",
}]
async fn widget_update(
    _rqctx: RequestContext<()>,
    _path: Path<WidgetPathArgs>,
    _query: Query<UpdateQuery>,
    _body: TypedBody<WidgetCreate>,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    unimplemented!();
}

// A DELETE endpoint.
#[endpoint {
    method = DELETE,
    path = "/widgets/{id}",
    tags = ["widgets"],
}]
/// Delete a widget
async fn widget_delete(
    _rqctx: RequestContext<()>,
    _path: Path<WidgetPathArgs>,
) -> Result<HttpResponseDeleted, HttpError> {
    unimplemented!();
}

// -- Enum types --

#[derive(Serialize, Deserialize, JsonSchema)]
enum Color {
    Red,
    Green,
    Blue,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
enum Shape {
    Circle { radius: f64 },
    Rectangle { width: f64, height: f64 },
}

#[derive(Serialize, JsonSchema)]
struct PaintJob {
    color: Color,
    shape: Shape,
}

#[endpoint {
    method = GET,
    path = "/paint",
}]
/// Get paint job
async fn paint_get(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<PaintJob>, HttpError> {
    unimplemented!();
}

// -- Nullable and free-form JSON --

#[derive(Serialize, JsonSchema)]
struct Flexible {
    /// Required string
    name: String,
    /// Nullable field
    nickname: Option<String>,
    /// Free-form metadata
    metadata: serde_json::Value,
    /// A list of tags
    tags: Vec<String>,
}

#[endpoint {
    method = GET,
    path = "/flexible",
}]
async fn flexible_get(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<Flexible>, HttpError> {
    unimplemented!();
}

// -- Response headers --

#[derive(Serialize, JsonSchema)]
struct EtagHeader {
    etag: String,
}

#[endpoint {
    method = GET,
    path = "/with-etag",
}]
/// Get with etag
async fn with_etag(
    _rqctx: RequestContext<()>,
) -> Result<
    HttpResponseHeaders<HttpResponseOk<Widget>, EtagHeader>,
    HttpError,
> {
    unimplemented!();
}

// -- Free-form body --

#[endpoint {
    method = GET,
    path = "/raw",
}]
/// Raw bytes
async fn raw_get(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<FreeformBody>, HttpError> {
    unimplemented!();
}

// -- Deprecated endpoint --

#[endpoint {
    method = GET,
    path = "/old",
    deprecated = true,
}]
/// Old endpoint
async fn old_get(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<()>, HttpError> {
    unimplemented!();
}

// -- Header parameters --

#[derive(Deserialize, JsonSchema)]
#[allow(dead_code)]
struct AuthHeaders {
    authorization: String,
    x_request_id: Option<String>,
}

#[endpoint {
    method = GET,
    path = "/authed",
}]
async fn authed_get(
    _rqctx: RequestContext<()>,
    _headers: Header<AuthHeaders>,
) -> Result<HttpResponseOk<Widget>, HttpError> {
    unimplemented!();
}

// -- Described string enum --

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum InstanceState {
    /// The instance is running
    Running,
    /// The instance is stopped
    Stopped,
    /// The instance is being created
    Creating,
    /// The instance is being destroyed
    Destroying,
}

#[endpoint {
    method = GET,
    path = "/instance-state",
}]
async fn instance_state_get(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<InstanceState>, HttpError> {
    unimplemented!();
}

// -- Validation constraints --

#[derive(Serialize, JsonSchema)]
struct Constrained {
    /// Must be between 1 and 100
    #[schemars(range(min = 1, max = 100))]
    count: u32,
    /// Short name
    #[schemars(length(min = 1, max = 63))]
    name: String,
    /// DNS-style name
    #[schemars(regex(pattern = r"^[a-z][a-z0-9-]*$"))]
    slug: String,
    /// Between 1 and 10 items
    #[schemars(length(min = 1, max = 10))]
    items: Vec<String>,
}

#[endpoint {
    method = GET,
    path = "/constrained",
}]
async fn constrained_get(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<Constrained>, HttpError> {
    unimplemented!();
}

// -- Default values --

#[derive(Serialize, Deserialize, JsonSchema)]
struct WithDefaults {
    /// Answer to everything
    #[serde(default = "forty_two")]
    answer: u32,
    /// Is it on?
    #[serde(default)]
    enabled: bool,
    /// Tags with default empty list
    #[serde(default)]
    tags: Vec<String>,
    /// Optional name
    #[serde(default = "default_name")]
    label: String,
}

fn forty_two() -> u32 {
    42
}

fn default_name() -> String {
    "unnamed".to_string()
}

#[endpoint {
    method = POST,
    path = "/with-defaults",
}]
async fn with_defaults_create(
    _rqctx: RequestContext<()>,
    _body: TypedBody<WithDefaults>,
) -> Result<HttpResponseCreated<WithDefaults>, HttpError> {
    unimplemented!();
}

// -- Additional status codes --

#[endpoint {
    method = POST,
    path = "/tasks",
}]
/// Start a task
async fn task_start(
    _rqctx: RequestContext<()>,
    _body: TypedBody<WidgetCreate>,
) -> Result<HttpResponseAccepted<Widget>, HttpError> {
    unimplemented!();
}

#[endpoint {
    method = GET,
    path = "/login",
}]
/// Redirect to login
async fn login_redirect(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseSeeOther, HttpError> {
    http_response_see_other("https://example.com".to_string())
}

// -- Untagged unions --

/// A name unique within the parent collection
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "Name")]
struct Name(
    #[schemars(length(min = 1, max = 63), regex(pattern = r"^[a-z]([a-zA-Z0-9-]*[a-zA-Z0-9]+)?$"))]
    String,
);

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
enum NameOrId {
    Id(uuid::Uuid),
    Name(Name),
}

#[derive(Serialize, JsonSchema)]
struct LookupResult {
    id: uuid::Uuid,
    target: NameOrId,
}

#[endpoint {
    method = GET,
    path = "/lookup",
}]
async fn lookup(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<LookupResult>, HttpError> {
    unimplemented!();
}

// -- Pagination (ResultsPage<T>) --

#[derive(Deserialize, JsonSchema, Serialize)]
struct ProjectScanParams {
    #[serde(default)]
    name_or_id: Option<String>,
}

#[derive(Deserialize, JsonSchema, Serialize)]
struct ProjectPageSelector {
    last_seen: String,
}

#[endpoint {
    method = GET,
    path = "/projects",
}]
/// List projects
async fn project_list(
    _rqctx: RequestContext<()>,
    _query: Query<PaginationParams<ProjectScanParams, ProjectPageSelector>>,
) -> Result<HttpResponseOk<ResultsPage<Widget>>, HttpError> {
    unimplemented!();
}

// -- Octet-stream request body --

#[endpoint {
    method = POST,
    path = "/upload",
}]
/// Upload raw bytes
async fn upload_bytes(
    _rqctx: RequestContext<()>,
    _body: UntypedBody,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    unimplemented!();
}

// -- Form-urlencoded request body --

#[derive(Deserialize, JsonSchema)]
#[allow(dead_code)]
struct DeviceAuthRequest {
    client_id: String,
}

#[endpoint {
    method = POST,
    path = "/device/auth",
    content_type = "application/x-www-form-urlencoded",
}]
/// Start device auth
async fn device_auth(
    _rqctx: RequestContext<()>,
    _body: TypedBody<DeviceAuthRequest>,
) -> Result<HttpResponseOk<()>, HttpError> {
    unimplemented!();
}

// -- Freeform response (hand-rolled Response<Body>) --

#[endpoint {
    method = POST,
    path = "/device/token",
}]
/// Get device token
async fn device_token(
    _rqctx: RequestContext<()>,
) -> Result<Response<Body>, HttpError> {
    unimplemented!();
}

// -- Discriminated union with mixed unit/payload variants --
// Like omicron's DiskState: some variants are just a tag, others carry data.

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
enum DiskState {
    /// Disk is being initialized
    Creating,
    /// Disk is detached
    Detached,
    /// Disk is being attached to an instance
    Attaching {
        /// The instance this disk is being attached to
        instance: uuid::Uuid,
    },
    /// Disk is attached to an instance
    Attached {
        /// The instance this disk is attached to
        instance: uuid::Uuid,
    },
    /// Disk has been destroyed
    Destroyed,
}

#[endpoint {
    method = GET,
    path = "/disks/{id}/state",
}]
/// Get disk state
async fn disk_state_get(
    _rqctx: RequestContext<()>,
    _path: Path<WidgetPathArgs>,
) -> Result<HttpResponseOk<DiskState>, HttpError> {
    unimplemented!();
}

// -- Complex struct with nullable refs, arrays of refs, and defaults --
// Like omicron's InstanceCreate: exercises allOf wrapping for nullable
// named types, arrays of named types, and complex default values.

#[derive(Deserialize, JsonSchema)]
#[allow(dead_code)]
struct CompositeCreate {
    /// The widget's name
    name: String,
    /// Desired color (if any)
    color: Option<Color>,
    /// Desired shape (if any)
    shape: Option<Shape>,
    /// Initial set of widgets
    #[serde(default)]
    widgets: Vec<Widget>,
    /// Whether to start immediately
    #[serde(default = "default_true")]
    start: bool,
}

fn default_true() -> bool {
    true
}

#[endpoint {
    method = POST,
    path = "/composites",
}]
/// Create a composite
async fn composite_create(
    _rqctx: RequestContext<()>,
    _body: TypedBody<CompositeCreate>,
) -> Result<HttpResponseCreated<Widget>, HttpError> {
    unimplemented!();
}

fn make_api() -> ApiDescription<()> {
    let mut api = ApiDescription::new();
    api.register(ping).unwrap();
    api.register(widget_get).unwrap();
    api.register(widget_create).unwrap();
    api.register(widget_update).unwrap();
    api.register(widget_delete).unwrap();
    api.register(paint_get).unwrap();
    api.register(flexible_get).unwrap();
    api.register(with_etag).unwrap();
    api.register(raw_get).unwrap();
    api.register(old_get).unwrap();
    api.register(authed_get).unwrap();
    api.register(instance_state_get).unwrap();
    api.register(constrained_get).unwrap();
    api.register(with_defaults_create).unwrap();
    api.register(task_start).unwrap();
    api.register(login_redirect).unwrap();
    api.register(lookup).unwrap();
    api.register(project_list).unwrap();
    api.register(upload_bytes).unwrap();
    api.register(device_auth).unwrap();
    api.register(device_token).unwrap();
    api.register(disk_state_get).unwrap();
    api.register(composite_create).unwrap();
    api
}

#[test]
fn test_typespec_generation() {
    let api = make_api();
    let output =
        api_to_typespec(&api, "Widget Service", &semver::Version::new(1, 0, 0));

    expectorate::assert_contents("tests/test_typespec.tsp", &output);
}
