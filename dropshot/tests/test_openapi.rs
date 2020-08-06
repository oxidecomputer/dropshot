// Copyright 2020 Oxide Computer Company

use difference::assert_diff;
use dropshot::{
    endpoint, ApiDescription, HttpError, HttpResponseCreated,
    HttpResponseDeleted, HttpResponseOk, TypedBody, PaginationParams, Path,
    Query, RequestContext, ResultsPage,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fs, io::Cursor, str::from_utf8, sync::Arc};

#[endpoint {
    method = GET,
    path = "/test/person",
}]
async fn handler1(
    _rqctx: Arc<RequestContext>,
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

#[derive(Deserialize, JsonSchema)]
struct QueryArgs {
    _tomax: String,
    _xamot: Option<String>,
    _destro: Vec<u16>,
}

#[endpoint {
    method = GET,
    path = "/test/woman",
}]
async fn handler2(
    _rqctx: Arc<RequestContext>,
    _query: Query<QueryArgs>,
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
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
    _rqctx: Arc<RequestContext>,
    _path: Path<PathArgs>,
) -> Result<HttpResponseDeleted, HttpError> {
    Ok(HttpResponseDeleted())
}

#[derive(JsonSchema, Deserialize)]
struct BodyParam {
    _x: String,
}

#[derive(Serialize, JsonSchema)]
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
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
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

    fixture("tests/test_openapi.json", actual)
}

fn fixture(path: &str, actual: &str) -> Result<(), String> {
    if let Ok(_) = std::env::var("FIXTURE") {
        fs::write(path, actual).map_err(|e| e.to_string())?;
    } else {
        let mut expected_s =
            fs::read_to_string(path).map_err(|e| e.to_string())?;
        if cfg!(windows) {
            expected_s = expected_s.replace("\r\n", "\n");
        }
        let expected = expected_s.as_str();

        println!("set FIXTURE= if these changes are intentional");
        assert_diff!(expected, actual, "\n", 0);
    }

    Ok(())
}
