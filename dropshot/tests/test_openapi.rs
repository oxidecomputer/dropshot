// Copyright 2020 Oxide Computer Company

use dropshot::{
    endpoint, ApiDescription, HttpError, HttpResponseAccepted,
    HttpResponseCreated, HttpResponseDeleted, HttpResponseOk,
    HttpResponseUpdatedNoContent, PaginationParams, Path, Query,
    RequestContext, ResultsPage, TypedBody, UntypedBody,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{io::Cursor, str::from_utf8};

#[endpoint {
    method = GET,
    path = "/test/person",
}]
/// This is a multi-
/// line comment.
/// It uses Rust-style.
async fn handler1(
    _rqctx: &RequestContext<()>,
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

#[derive(Deserialize, JsonSchema)]
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
/**
 * This is a multi-
 * line comment.
 * It uses C-style.
 */
async fn handler2(
    _rqctx: &RequestContext<()>,
    _query: Query<QueryArgs>,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    Ok(HttpResponseUpdatedNoContent())
}

#[derive(Deserialize, JsonSchema)]
#[allow(dead_code)]
struct PathArgs {
    x: String,
}

#[endpoint {
    method = DELETE,
    path = "/test/man/{x}",
}]
async fn handler3(
    _rqctx: &RequestContext<()>,
    _path: Path<PathArgs>,
) -> Result<HttpResponseDeleted, HttpError> {
    Ok(HttpResponseDeleted())
}

#[derive(JsonSchema, Deserialize)]
struct BodyParam {
    _x: String,
    _any: serde_json::Value,
}

#[derive(Serialize, JsonSchema)]
struct Response {}

#[endpoint {
    method = POST,
    path = "/test/camera",
}]
async fn handler4(
    _rqctx: &RequestContext<()>,
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
    _rqctx: &RequestContext<()>,
    _path: Path<PathArgs>,
    _query: Query<QueryArgs>,
    _body: TypedBody<BodyParam>,
) -> Result<HttpResponseAccepted<()>, HttpError> {
    Ok(HttpResponseAccepted(()))
}

#[derive(JsonSchema, Serialize)]
struct ResponseItem {
    word: String,
}

#[derive(Deserialize, JsonSchema, Serialize)]
struct ExampleScanParams {
    #[serde(default)]
    a_number: u16,
}

#[derive(Deserialize, JsonSchema, Serialize)]
struct ExamplePageSelector {
    scan: ExampleScanParams,
    last_seen: String,
}

#[endpoint {
    method = GET,
    path = "/impairment",
}]
async fn handler6(
    _rqctx: &RequestContext<()>,
    _query: Query<PaginationParams<ExampleScanParams, ExamplePageSelector>>,
) -> Result<HttpResponseOk<ResultsPage<ResponseItem>>, HttpError> {
    unimplemented!();
}

#[endpoint {
    method = PUT,
    path = "/datagoeshere",
}]
async fn handler7(
    _rqctx: &RequestContext<()>,
    _dump: UntypedBody,
) -> Result<HttpResponseOk<()>, HttpError> {
    unimplemented!();
}

/*
 * Test that we do not generate duplicate type definitions when the same type is
 * returned by two different handler functions.
 */

#[derive(JsonSchema, Serialize)]
struct NeverDuplicatedResponseTopLevel {
    b: NeverDuplicatedResponseNextLevel,
}

#[derive(JsonSchema, Serialize)]
struct NeverDuplicatedResponseNextLevel {
    v: bool,
}

#[endpoint {
    method = GET,
    path = "/dup1",
}]
async fn handler8(
    _rqctx: &RequestContext<()>,
) -> Result<HttpResponseOk<NeverDuplicatedResponseTopLevel>, HttpError> {
    unimplemented!();
}

#[endpoint {
    method = GET,
    path = "/dup2",
}]
async fn handler9(
    _rqctx: &RequestContext<()>,
) -> Result<HttpResponseOk<NeverDuplicatedResponseTopLevel>, HttpError> {
    unimplemented!();
}

/*
 * Similarly, test that we do not generate duplicate type definitions when the
 * same type is accepted as a query parameter to two different handler
 * functions.
 */

#[derive(Deserialize, JsonSchema)]
struct NeverDuplicatedParamTopLevel {
    _b: NeverDuplicatedParamNextLevel,
}

#[derive(Deserialize, JsonSchema)]
struct NeverDuplicatedParamNextLevel {
    _v: bool,
}

#[endpoint {
    method = PUT,
    path = "/dup3",
}]
async fn handler10(
    _rqctx: &RequestContext<()>,
    _q: Query<NeverDuplicatedParamTopLevel>,
) -> Result<HttpResponseOk<()>, HttpError> {
    unimplemented!();
}

#[endpoint {
    method = PUT,
    path = "/dup4",
}]
async fn handler11(
    _rqctx: &RequestContext<()>,
    _q: Query<NeverDuplicatedParamTopLevel>,
) -> Result<HttpResponseOk<()>, HttpError> {
    unimplemented!();
}

/*
 * Similarly, test that we do not generate duplicate type definitions when the
 * same type is accepted as a typed body to two different handler functions.
 */

#[derive(Deserialize, JsonSchema)]
struct NeverDuplicatedBodyTopLevel {
    _b: NeverDuplicatedBodyNextLevel,
}

#[derive(Deserialize, JsonSchema)]
struct NeverDuplicatedBodyNextLevel {
    _v: bool,
}

#[endpoint {
    method = PUT,
    path = "/dup5",
}]
async fn handler12(
    _rqctx: &RequestContext<()>,
    _b: TypedBody<NeverDuplicatedBodyTopLevel>,
) -> Result<HttpResponseOk<()>, HttpError> {
    unimplemented!();
}

#[endpoint {
    method = PUT,
    path = "/dup6",
}]
async fn handler13(
    _rqctx: &RequestContext<()>,
    _b: TypedBody<NeverDuplicatedBodyTopLevel>,
) -> Result<HttpResponseOk<()>, HttpError> {
    unimplemented!();
}

/*
 * Finally, test that we do not generate duplicate type definitions when the
 * same type is used in two different places.
 */

#[derive(Deserialize, JsonSchema, Serialize)]
struct NeverDuplicatedTop {
    _b: NeverDuplicatedNext,
}

#[derive(Deserialize, JsonSchema, Serialize)]
struct NeverDuplicatedNext {
    _v: bool,
}

#[endpoint {
    method = PUT,
    path = "/dup7",
}]
async fn handler14(
    _rqctx: &RequestContext<()>,
    _b: TypedBody<NeverDuplicatedTop>,
) -> Result<HttpResponseOk<()>, HttpError> {
    unimplemented!();
}

#[endpoint {
    method = GET,
    path = "/dup8",
}]
async fn handler15(
    _rqctx: &RequestContext<()>,
) -> Result<HttpResponseOk<NeverDuplicatedTop>, HttpError> {
    unimplemented!();
}

fn make_api() -> Result<ApiDescription<()>, String> {
    let mut api = ApiDescription::new();
    api.register(handler1)?;
    api.register(handler2)?;
    api.register(handler3)?;
    api.register(handler4)?;
    api.register(handler5)?;
    api.register(handler6)?;
    api.register(handler7)?;
    api.register(handler8)?;
    api.register(handler9)?;
    api.register(handler10)?;
    api.register(handler11)?;
    api.register(handler12)?;
    api.register(handler13)?;
    api.register(handler14)?;
    api.register(handler15)?;
    Ok(api)
}

#[test]
fn test_openapi_old() -> Result<(), String> {
    let api = make_api()?;
    let mut output = Cursor::new(Vec::new());

    #[allow(deprecated)]
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

    expectorate::assert_contents("tests/test_openapi_old.json", actual);
    Ok(())
}

#[test]
fn test_openapi() -> Result<(), String> {
    let api = make_api()?;
    let mut output = Cursor::new(Vec::new());

    let _ = api.openapi("test", "threeve").write(&mut output);
    let actual = from_utf8(&output.get_ref()).unwrap();

    expectorate::assert_contents("tests/test_openapi.json", actual);
    Ok(())
}

#[test]
fn test_openapi_fuller() -> Result<(), String> {
    let api = make_api()?;
    let mut output = Cursor::new(Vec::new());

    let _ = api
        .openapi("test", "1985.7")
        .description("gusty winds may exist")
        .contact_name("old mate")
        .license_name("CDDL")
        .terms_of_service("no hat, no cane? no service!")
        .write(&mut output);
    let actual = from_utf8(&output.get_ref()).unwrap();

    expectorate::assert_contents("tests/test_openapi_fuller.json", actual);
    Ok(())
}
