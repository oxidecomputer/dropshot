// Copyright 2026 Oxide Computer Company

use dropshot::{
    api_to_typespec, endpoint, ApiDescription, HttpError, HttpResponseCreated,
    HttpResponseDeleted, HttpResponseOk, HttpResponseUpdatedNoContent, Path,
    Query, RequestContext, TypedBody,
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

fn make_api() -> ApiDescription<()> {
    let mut api = ApiDescription::new();
    api.register(ping).unwrap();
    api.register(widget_get).unwrap();
    api.register(widget_create).unwrap();
    api.register(widget_update).unwrap();
    api.register(widget_delete).unwrap();
    api
}

#[test]
fn test_typespec_generation() {
    let api = make_api();
    let output =
        api_to_typespec(&api, "Widget Service", &semver::Version::new(1, 0, 0));

    expectorate::assert_contents("tests/test_typespec.tsp", &output);
}
