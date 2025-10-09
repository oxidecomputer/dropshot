// Copyright 2025 Oxide Computer Company

use dropshot::{
    ApiDescription, ApiDescriptionRegisterError, FreeformBody, Header,
    HttpError, HttpResponseAccepted, HttpResponseCreated, HttpResponseDeleted,
    HttpResponseFound, HttpResponseHeaders, HttpResponseOk,
    HttpResponseSeeOther, HttpResponseTemporaryRedirect,
    HttpResponseUpdatedNoContent, MultipartBody, PaginationParams, Path, Query,
    RequestContext, ResultsPage, TagConfig, TagDetails, TypedBody, UntypedBody,
    channel, endpoint, http_response_found, http_response_see_other,
    http_response_temporary_redirect,
};
use dropshot::{Body, WebsocketConnection};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io::Cursor, str::from_utf8};

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
    _rqctx: RequestContext<()>,
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
/// C-style comment
///
/// This is a multi-
/// line comment.
async fn handler2(
    _rqctx: RequestContext<()>,
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
    _rqctx: RequestContext<()>,
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
    _rqctx: RequestContext<()>,
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
    _rqctx: RequestContext<()>,
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
    a_mandatory_string: String,
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
    _rqctx: RequestContext<()>,
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
    _rqctx: RequestContext<()>,
    _dump: UntypedBody,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    unimplemented!();
}

// Test that we do not generate duplicate type definitions when the same type is
// returned by two different handler functions.

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
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<NeverDuplicatedResponseTopLevel>, HttpError> {
    unimplemented!();
}

#[endpoint {
    method = GET,
    path = "/dup2",
    tags = ["it"],
}]
async fn handler9(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<NeverDuplicatedResponseTopLevel>, HttpError> {
    unimplemented!();
}

// Similarly, test that we do not generate duplicate type definitions when the
// same type is accepted as a typed body to two different handler functions.

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
    _rqctx: RequestContext<()>,
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
    _rqctx: RequestContext<()>,
    _b: TypedBody<NeverDuplicatedBodyTopLevel>,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    unimplemented!();
}

// Finally, test that we do not generate duplicate type definitions when the
// same type is used in two different places.

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
    _rqctx: RequestContext<()>,
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
    _rqctx: RequestContext<()>,
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
    _rqctx: RequestContext<()>,
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
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<()>, HttpError> {
    unimplemented!();
}

#[endpoint {
    method = GET,
    path = "/too/smart/for/my/own/good",
    tags = ["it"],
}]
async fn handler16(
    _rqctx: RequestContext<()>,
) -> Result<http::Response<Body>, HttpError> {
    unimplemented!();
}

#[derive(Serialize, JsonSchema)]
struct SomeHeaders {
    /// eee! a tag
    #[serde(rename = "Etag")]
    etag: String,
    /// this is a foo that is non-required
    #[serde(rename = "x-foo-mobile")]
    foo: Option<Foo>,
}

#[derive(Serialize, JsonSchema)]
struct Foo(String);

#[endpoint {
    method = GET,
    path = "/with/headers",
    tags = ["it"],
}]
async fn handler17(
    _rqctx: RequestContext<()>,
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
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<FreeformBody>, HttpError> {
    let body = Body::empty();
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
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<ObjectWithExample>, HttpError> {
    Ok(HttpResponseOk(example_object_with_example()))
}

#[endpoint {
    method = POST,
    path = "/test/urlencoded",
    content_type = "application/x-www-form-urlencoded",
    tags = ["it"]
}]
async fn handler20(
    _rqctx: RequestContext<()>,
    _body: TypedBody<BodyParam>,
) -> Result<HttpResponseCreated<Response>, HttpError> {
    Ok(HttpResponseCreated(Response {}))
}

#[endpoint {
    method = GET,
    path = "/test/302_found",
    tags = [ "it"],
}]
async fn handler21(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseFound, HttpError> {
    Ok(http_response_found(String::from("/path1")).unwrap())
}

#[endpoint {
    method = GET,
    path = "/test/303_see_other",
    tags = [ "it"],
}]
async fn handler22(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseSeeOther, HttpError> {
    Ok(http_response_see_other(String::from("/path2")).unwrap())
}

#[endpoint {
    method = GET,
    path = "/test/307_temporary_redirect",
    tags = [ "it"],
}]
async fn handler23(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseTemporaryRedirect, HttpError> {
    Ok(http_response_temporary_redirect(String::from("/path3")).unwrap())
}

#[endpoint {
    method = GET,
    path = "/test/deprecated",
    tags = [ "it"],
    deprecated = true,
}]
async fn handler24(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseTemporaryRedirect, HttpError> {
    unimplemented!()
}

#[endpoint {
    method = POST,
    path = "/test/multipart-form-data",
    tags = ["it"]
}]
async fn handler25(
    _rqctx: RequestContext<()>,
    _body: MultipartBody,
) -> Result<HttpResponseCreated<Response>, HttpError> {
    Ok(HttpResponseCreated(Response {}))
}

// test: Overridden operation id
#[endpoint {
    operation_id = "vzeroupper",
    method = GET,
    path = "/first_thing",
    tags = ["it"]
}]
async fn handler26(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseCreated<Response>, HttpError> {
    Ok(HttpResponseCreated(Response {}))
}

// test: websocket using overridden operation id
#[channel {
    protocol = WEBSOCKETS,
    operation_id = "vzerolower",
    path = "/other_thing",
    tags = ["it"]
}]
async fn handler27(
    _rqctx: RequestContext<()>,
    _: WebsocketConnection,
) -> dropshot::WebsocketChannelResult {
    Ok(())
}

#[derive(Deserialize, JsonSchema)]
#[allow(dead_code)]
struct MyHeaders {
    a: String,
    b: Option<String>,
}

// test: header params
#[endpoint {
    operation_id = "hparam",
    method = GET,
    path = "/thing_with_headers",
    tags = ["it"]
}]
async fn handler28(
    _rqctx: RequestContext<()>,
    _headers: Header<MyHeaders>,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    Ok(HttpResponseUpdatedNoContent())
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct CoolStruct {
    #[serde(flatten)]
    cool_enum: CoolEnum,

    another_thing: u16,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum CoolEnum {
    Foo { thing_one: String },
    Bar { thing_one: String, thing_two: String },
}

#[endpoint {
    operation_id = "big_flat",
    method = GET,
    path = "/flattened",
    tags = ["it"]
}]
async fn handler29(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<CoolStruct>, HttpError> {
    todo!()
}

#[derive(Deserialize, JsonSchema)]
struct PathArgs30 {
    #[expect(unused)]
    aa: WithXRustType<XRustAParam>,
    #[expect(unused)]
    bb: WithXRustType<XRustBParam>,
}

#[derive(Deserialize, Debug)]
struct WithXRustType<T> {
    _data: T,
}

impl<T: JsonSchema> JsonSchema for WithXRustType<T> {
    fn schema_name() -> String {
        format!("WithXRustTypeFor{}", T::schema_name())
    }

    fn json_schema(
        r#gen: &mut schemars::r#gen::SchemaGenerator,
    ) -> schemars::schema::Schema {
        use schemars::schema::*;

        let mut schema = SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(
                InstanceType::String,
            ))),
            ..Default::default()
        };

        // Add the x-rust-type extension.
        let mut extensions = schemars::Map::new();
        let rust_type = serde_json::json!({
            "crate": "foo",
            "version": "*",
            "path": "foo",
            "parameters": [
                r#gen.subschema_for::<T>(),
            ],
        });
        extensions.insert("x-rust-type".to_string(), rust_type);
        schema.extensions = extensions;

        Schema::Object(schema)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct XRustAParam {
    #[expect(unused)]
    data: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
enum XRustBParam {}

#[endpoint {
    method = PUT,
    path = "/testing/{aa}/{bb}",
    tags = ["it"]
}]
async fn handler30(
    _: RequestContext<()>,
    _: Path<PathArgs30>,
) -> Result<HttpResponseOk<CoolStruct>, HttpError> {
    todo!();
}

#[derive(Deserialize, JsonSchema)]
struct PathArgs31 {
    #[expect(unused)]
    aa: String,
}

#[derive(Deserialize, JsonSchema)]
struct Headers31 {
    #[expect(unused)]
    header_a: String,
}

#[derive(Deserialize, JsonSchema)]
struct Query31 {
    #[expect(unused)]
    query_a: String,
}

#[endpoint {
    method = GET,
    path = "/testing/{aa}",
    tags = ["it"]
}]
async fn handler31(
    _: RequestContext<()>,
    _: Path<PathArgs31>,
    _: Header<Headers31>,
    _: Query<Query31>,
    _: UntypedBody,
) -> Result<HttpResponseOk<CoolStruct>, HttpError> {
    todo!();
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CustomShared32 {
    pub a: String,
}

#[async_trait::async_trait]
impl dropshot::SharedExtractor for CustomShared32 {
    async fn from_request<Context: dropshot::ServerContext>(
        _rqctx: &RequestContext<Context>,
    ) -> Result<Self, HttpError> {
        Ok(Self { a: "test".to_string() })
    }

    fn metadata(
        _body_content_type: dropshot::ApiEndpointBodyContentType,
    ) -> dropshot::ExtractorMetadata {
        dropshot::ExtractorMetadata {
            extension_mode: dropshot::ExtensionMode::None,
            parameters: vec![],
        }
    }
}

#[endpoint {
    method = GET,
    path = "/testing32/{aa}",
    tags = ["it"]
}]
async fn handler32(
    _: RequestContext<()>,
    _: Path<PathArgs31>,
    _: Header<Headers31>,
    _: Query<Query31>,
    _: CustomShared32,
    _: UntypedBody,
) -> Result<HttpResponseOk<CoolStruct>, HttpError> {
    todo!();
}

fn make_api(
    maybe_tag_config: Option<TagConfig>,
) -> Result<ApiDescription<()>, ApiDescriptionRegisterError> {
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
    api.register(handler20)?;
    api.register(handler21)?;
    api.register(handler22)?;
    api.register(handler23)?;
    api.register(handler24)?;
    api.register(handler25)?;
    api.register(handler26)?;
    api.register(handler27)?;
    api.register(handler28)?;
    api.register(handler29)?;
    api.register(handler30)?;
    api.register(handler31)?;
    api.register(handler32)?;
    Ok(api)
}

#[test]
fn test_openapi() -> anyhow::Result<()> {
    let api = make_api(None)?;
    let mut output = Cursor::new(Vec::new());

    let _ =
        api.openapi("test", semver::Version::new(3, 5, 0)).write(&mut output);
    let actual = from_utf8(output.get_ref()).unwrap();

    expectorate::assert_contents("tests/test_openapi.json", actual);
    Ok(())
}

#[test]
fn test_openapi_fuller() -> anyhow::Result<()> {
    let mut tags = HashMap::new();
    tags.insert(
        "it".to_string(),
        TagDetails {
            description: Some("Now you are the one who is it.".to_string()),
            external_docs: None,
        },
    );
    let tag_config = TagConfig {
        allow_other_tags: true,
        policy: dropshot::EndpointTagPolicy::AtLeastOne,
        tags,
    };
    let api = make_api(Some(tag_config))?;
    let mut output = Cursor::new(Vec::new());

    let _ = api
        .openapi("test", semver::Version::new(1985, 7, 0))
        .description("gusty winds may exist")
        .contact_name("old mate")
        .license_name("CDDL")
        .terms_of_service("no hat, no cane? no service!")
        .write(&mut output);
    let actual = from_utf8(output.get_ref()).unwrap();

    expectorate::assert_contents("tests/test_openapi_fuller.json", actual);
    Ok(())
}

#[test]
fn test_openapi_custom_error_types() -> anyhow::Result<()> {
    let api = super::custom_errors::api();
    let mut output = Cursor::new(Vec::new());

    let _ =
        api.openapi("test", semver::Version::new(3, 5, 0)).write(&mut output);
    let actual = from_utf8(output.get_ref()).unwrap();

    expectorate::assert_contents(
        "tests/test_openapi_custom_error_types.json",
        actual,
    );
    Ok(())
}

#[test]
fn test_openapi_custom_error_types_trait_based() -> anyhow::Result<()> {
    let api =
        super::custom_errors::custom_error_api_mod::stub_api_description()
            .unwrap();
    let mut output = Cursor::new(Vec::new());

    let _ =
        api.openapi("test", semver::Version::new(3, 5, 0)).write(&mut output);
    let actual = from_utf8(output.get_ref()).unwrap();

    expectorate::assert_contents(
        "tests/test_openapi_custom_error_types_trait_based.json",
        actual,
    );
    Ok(())
}
