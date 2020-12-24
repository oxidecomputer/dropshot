// Copyright 2020 Oxide Computer Company

use dropshot::{
    endpoint, ApiDescription, ExtractedParameter, HttpError,
    HttpResponseAccepted, HttpResponseCreated, HttpResponseDeleted,
    HttpResponseOk, HttpResponseUpdatedNoContent, PaginationParams, Path,
    Query, RequestContext, ResultsPage, TypedBody,
};
use serde::{Deserialize, Serialize};
use std::{io::Cursor, str::from_utf8, sync::Arc};

#[endpoint {
    method = GET,
    path = "/test/person",
}]
async fn handler1(
    _rqctx: Arc<RequestContext>,
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

#[derive(Deserialize, ExtractedParameter)]
#[allow(dead_code)]
struct QueryArgs {
    _tomax: String,
    _xamot: Option<String>,
    _destro: Vec<u16>,
}

#[endpoint {
    method = PUT,
    path = "/test/woman",
}]
async fn handler2(
    _rqctx: Arc<RequestContext>,
    _query: Query<QueryArgs>,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    Ok(HttpResponseUpdatedNoContent())
}

#[derive(Deserialize, ExtractedParameter)]
#[allow(dead_code)]
struct PathArgs {
    x: String,
}

#[endpoint {
    method = DELETE,
    path = "/test/man/{x}",
}]
async fn handler3(
    _rqctx: Arc<RequestContext>,
    _path: Path<PathArgs>,
) -> Result<HttpResponseDeleted, HttpError> {
    Ok(HttpResponseDeleted())
}

#[derive(ExtractedParameter, Deserialize)]
struct BodyParam {
    _x: String,
}

#[derive(Serialize, ExtractedParameter)]
struct Response {}

#[endpoint {
    method = POST,
    path = "/test/camera",
}]
async fn handler4(
    _rqctx: Arc<RequestContext>,
    _body: TypedBody<BodyParam>,
) -> Result<HttpResponseCreated<Response>, HttpError> {
    Ok(HttpResponseCreated(Response {}))
}

#[endpoint {
    method = POST,
    path = "/test/tv/{x}",
    tags = [ "person", "woman", "man", "camera", "tv"]
}]
async fn handler5(
    _rqctx: Arc<RequestContext>,
    _path: Path<PathArgs>,
    _query: Query<QueryArgs>,
    _body: TypedBody<BodyParam>,
) -> Result<HttpResponseAccepted<()>, HttpError> {
    Ok(HttpResponseAccepted(()))
}

#[derive(ExtractedParameter, Serialize)]
struct ResponseItem {
    word: String,
}

#[derive(Deserialize, ExtractedParameter, Serialize)]
struct ExampleScanParams {
    #[serde(default)]
    a_number: u16,
}

#[derive(Deserialize, ExtractedParameter, Serialize)]
struct ExamplePageSelector {
    scan: ExampleScanParams,
    last_seen: String,
}

#[endpoint {
    method = GET,
    path = "/impairment",
}]
async fn handler6(
    _rqctx: Arc<RequestContext>,
    _query: Query<PaginationParams<ExampleScanParams, ExamplePageSelector>>,
) -> Result<HttpResponseOk<ResultsPage<ResponseItem>>, HttpError> {
    unimplemented!();
}

#[test]
fn test_openapi() -> Result<(), String> {
    let mut api = ApiDescription::new();
    api.register(handler1)?;
    api.register(handler2)?;
    api.register(handler3)?;
    api.register(handler4)?;
    api.register(handler5)?;
    api.register(handler6)?;

    let mut output = Cursor::new(Vec::new());

    let _ = api.print_openapi(
        &mut output,
        &"test",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &"threeve",
    );
    let actual = from_utf8(&output.get_ref()).unwrap();

    expectorate::assert_contents("tests/test_openapi.json", actual);
    Ok(())
}
