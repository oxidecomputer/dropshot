// Copyright 2026 Oxide Computer Company

use dropshot::{
    api_to_typespec, endpoint, ApiDescription, FreeformBody, Header, HttpError,
    HttpResponseCreated, HttpResponseDeleted, HttpResponseHeaders,
    HttpResponseOk, HttpResponseUpdatedNoContent, Path, Query, RequestContext,
    TypedBody,
};
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
#[derive(Serialize, JsonSchema)]
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
    api
}

#[test]
fn test_typespec_generation() {
    let api = make_api();
    let output =
        api_to_typespec(&api, "Widget Service", &semver::Version::new(1, 0, 0));

    expectorate::assert_contents("tests/test_typespec.tsp", &output);
}
