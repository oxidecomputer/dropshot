// Copyright 2022 Oxide Computer Company

use dropshot::{
    endpoint, ApiDescription, FreeformBody, HttpError, HttpResponseAccepted,
    HttpResponseCreated, HttpResponseDeleted, HttpResponseHeaders,
    HttpResponseOk, HttpResponseUpdatedNoContent, PaginationParams, Path,
    Query, RequestContext, ResultsPage, TagConfig, TagDetails, TypedBody,
    UntypedBody,
};
use hyper::Body;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io::Cursor, str::from_utf8, sync::Arc};

#[endpoint {
    method = GET,
    path = "/test/person",
    tags = ["it"],
}]
/// Rust style comment
///
/// This is a multi-
/// line comment.
async fn handler1(
    _rqctx: Arc<RequestContext<()>>,
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

#[derive(Deserialize, JsonSchema)]
#[allow(dead_code)]
struct QueryArgs {
    /// One brother connected by the pain
    tomax: String,
    /// Spoiler: there's a reason this is not required...
    xamot: Option<String>,
}

#[endpoint {
    method = PUT,
    path = "/test/woman",
    tags = ["it"],
}]
/**
 * C-style comment
 *
 * This is a multi-
 * line comment.
 */
async fn handler2(
    _rqctx: Arc<RequestContext<()>>,
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
    tags = ["it"],
}]
async fn handler3(
    _rqctx: Arc<RequestContext<()>>,
    _path: Path<PathArgs>,
) -> Result<HttpResponseDeleted, HttpError> {
    Ok(HttpResponseDeleted())
}

#[derive(JsonSchema, Deserialize)]
#[allow(dead_code)]
struct BodyParam {
    x: String,
    any: serde_json::Value,
    #[serde(default)]
    things: Vec<u32>,
    #[serde(default)]
    maybe: bool,
    #[serde(default = "forty_two")]
    answer: i32,
    #[serde(default = "nested_default")]
    nested: BodyParamNested,
}

fn forty_two() -> i32 {
    42
}

#[derive(JsonSchema, Deserialize, Serialize)]
struct BodyParamNested {
    maybe: Option<bool>,
}

fn nested_default() -> BodyParamNested {
    BodyParamNested { maybe: Some(false) }
}

#[derive(Serialize, JsonSchema)]
struct Response {}

#[endpoint {
    method = POST,
    path = "/test/camera",
    tags = ["it"],
}]
async fn handler4(
    _rqctx: Arc<RequestContext<()>>,
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
    _rqctx: Arc<RequestContext<()>>,
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
    tags = ["it"],
}]
async fn handler6(
    _rqctx: Arc<RequestContext<()>>,
    _query: Query<PaginationParams<ExampleScanParams, ExamplePageSelector>>,
) -> Result<HttpResponseOk<ResultsPage<ResponseItem>>, HttpError> {
    unimplemented!();
}

#[endpoint {
    method = PUT,
    path = "/datagoeshere",
    tags = ["it"],
}]
async fn handler7(
    _rqctx: Arc<RequestContext<()>>,
    _dump: UntypedBody,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    unimplemented!();
}

/*
 * Test that we do not generate duplicate type definitions when the same type is
 * returned by two different handler functions.
 */

/// Best non-duplicated type
#[derive(JsonSchema, Serialize)]
struct NeverDuplicatedResponseTopLevel {
    /// Bee
    b: NeverDuplicatedResponseNextLevel,
}

/// Veritably non-duplicated type
#[derive(JsonSchema, Serialize)]
struct NeverDuplicatedResponseNextLevel {
    /// Vee
    v: bool,
}

#[endpoint {
    method = GET,
    path = "/dup1",
    tags = ["it"],
}]
async fn handler8(
    _rqctx: Arc<RequestContext<()>>,
) -> Result<HttpResponseOk<NeverDuplicatedResponseTopLevel>, HttpError> {
    unimplemented!();
}

#[endpoint {
    method = GET,
    path = "/dup2",
    tags = ["it"],
}]
async fn handler9(
    _rqctx: Arc<RequestContext<()>>,
) -> Result<HttpResponseOk<NeverDuplicatedResponseTopLevel>, HttpError> {
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
#[allow(dead_code)]
struct NeverDuplicatedBodyNextLevel {
    v: bool,
}

#[endpoint {
    method = PUT,
    path = "/dup5",
    tags = ["it"],
}]
async fn handler10(
    _rqctx: Arc<RequestContext<()>>,
    _b: TypedBody<NeverDuplicatedBodyTopLevel>,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    unimplemented!();
}

#[endpoint {
    method = PUT,
    path = "/dup6",
    tags = ["it"],
}]
async fn handler11(
    _rqctx: Arc<RequestContext<()>>,
    _b: TypedBody<NeverDuplicatedBodyTopLevel>,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    unimplemented!();
}

/*
 * Finally, test that we do not generate duplicate type definitions when the
 * same type is used in two different places.
 */

#[derive(Deserialize, JsonSchema, Serialize)]
#[allow(dead_code)]
struct NeverDuplicatedTop {
    b: NeverDuplicatedNext,
}

#[derive(Deserialize, JsonSchema, Serialize)]
#[allow(dead_code)]
struct NeverDuplicatedNext {
    v: bool,
}

#[endpoint {
    method = PUT,
    path = "/dup7",
    tags = ["it"],
}]
async fn handler12(
    _rqctx: Arc<RequestContext<()>>,
    _b: TypedBody<NeverDuplicatedTop>,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    unimplemented!();
}

#[endpoint {
    method = GET,
    path = "/dup8",
    tags = ["it"],
}]
async fn handler13(
    _rqctx: Arc<RequestContext<()>>,
) -> Result<HttpResponseOk<NeverDuplicatedTop>, HttpError> {
    unimplemented!();
}

#[allow(dead_code)]
#[derive(JsonSchema, Deserialize)]
struct AllPath {
    path: Vec<String>,
}

#[endpoint {
    method = GET,
    path = "/ceci_nes_pas_une_endpoint/{path:.*}",
    unpublished = true,
}]
async fn handler14(
    _rqctx: Arc<RequestContext<()>>,
    _path: Path<AllPath>,
) -> Result<HttpResponseOk<NeverDuplicatedTop>, HttpError> {
    unimplemented!();
}

#[endpoint {
    method = GET,
    path = "/unit_please",
    tags = ["it"],
}]
async fn handler15(
    _rqctx: Arc<RequestContext<()>>,
) -> Result<HttpResponseOk<()>, HttpError> {
    unimplemented!();
}

#[endpoint {
    method = GET,
    path = "/too/smart/for/my/own/good",
    tags = ["it"],
}]
async fn handler16(
    _rqctx: Arc<RequestContext<()>>,
) -> Result<http::Response<Body>, HttpError> {
    unimplemented!();
}

#[derive(Serialize, JsonSchema)]
struct SomeHeaders {
    /// eee! a tag
    #[serde(rename = "Etag")]
    etag: String,
    /// this is a foo
    #[serde(rename = "x-foo-mobile")]
    foo: Foo,
}

#[derive(Serialize, JsonSchema)]
struct Foo(String);

#[endpoint {
    method = GET,
    path = "/with/headers",
    tags = ["it"],
}]
async fn handler17(
    _rqctx: Arc<RequestContext<()>>,
) -> Result<
    HttpResponseHeaders<HttpResponseUpdatedNoContent, SomeHeaders>,
    HttpError,
> {
    unimplemented!();
}

#[endpoint {
    method = GET,
    path = "/playing/a/bit/nicer",
    tags = ["it"],
}]
async fn handler18(
    _rqctx: Arc<RequestContext<()>>,
) -> Result<HttpResponseOk<FreeformBody>, HttpError> {
    let (_, body) = Body::channel();
    Ok(HttpResponseOk(body.into()))
}

#[derive(Serialize, JsonSchema)]
#[schemars(example = "example_object_with_example")]
struct ObjectWithExample {
    id: u32,
    name: String,
    nested: NestedObjectWithExample,
}

#[derive(Serialize, JsonSchema)]
#[schemars(example = "example_nested_object_with_example")]
struct NestedObjectWithExample {
    nick_name: String,
}

fn example_object_with_example() -> ObjectWithExample {
    ObjectWithExample {
        id: 456,
        name: "foo bar".into(),
        nested: example_nested_object_with_example(),
    }
}

fn example_nested_object_with_example() -> NestedObjectWithExample {
    NestedObjectWithExample { nick_name: "baz".into() }
}

#[endpoint {
    method = GET,
    path = "/with/example",
    tags = ["it"],
}]
async fn handler19(
    _rqctx: Arc<RequestContext<()>>,
) -> Result<HttpResponseOk<ObjectWithExample>, HttpError> {
    Ok(HttpResponseOk(example_object_with_example()))
}

fn make_api(
    maybe_tag_config: Option<TagConfig>,
) -> Result<ApiDescription<()>, String> {
    let mut api = ApiDescription::new();

    if let Some(tag_config) = maybe_tag_config {
        api = api.tag_config(tag_config);
    }

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
    api.register(handler16)?;
    api.register(handler17)?;
    api.register(handler18)?;
    api.register(handler19)?;
    Ok(api)
}

#[test]
fn test_openapi() -> Result<(), String> {
    let api = make_api(None)?;
    let mut output = Cursor::new(Vec::new());

    let _ = api.openapi("test", "threeve").write(&mut output);
    let actual = from_utf8(output.get_ref()).unwrap();

    expectorate::assert_contents("tests/test_openapi.json", actual);
    Ok(())
}

#[test]
fn test_openapi_fuller() -> Result<(), String> {
    let mut tag_definitions = HashMap::new();
    tag_definitions.insert(
        "it".to_string(),
        TagDetails {
            description: Some("Now you are the one who is it.".to_string()),
            external_docs: None,
        },
    );
    let tag_config = TagConfig {
        allow_other_tags: true,
        endpoint_tag_policy: dropshot::EndpointTagPolicy::AtLeastOne,
        tag_definitions,
    };
    let api = make_api(Some(tag_config))?;
    let mut output = Cursor::new(Vec::new());

    let _ = api
        .openapi("test", "1985.7")
        .description("gusty winds may exist")
        .contact_name("old mate")
        .license_name("CDDL")
        .terms_of_service("no hat, no cane? no service!")
        .write(&mut output);
    let actual = from_utf8(output.get_ref()).unwrap();

    expectorate::assert_contents("tests/test_openapi_fuller.json", actual);
    Ok(())
}
