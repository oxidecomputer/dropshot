// Copyright 2020 Oxide Computer Company

use dropshot::{
    endpoint, ApiDescription, HttpError, HttpResponseOk, PaginationParams,
    Query, RequestContext, ResultsPage,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{io::Cursor, str::from_utf8, sync::Arc};

#[derive(JsonSchema, Serialize)]
struct ResponseItem {
    word: String,
}

#[derive(Deserialize, JsonSchema, Serialize)]
struct ScanParams {
    garbage_goes_in: GarbageGoesIn,
}

#[derive(Copy, Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GarbageGoesIn {
    GarbageCan,
}

#[derive(Deserialize, JsonSchema, Serialize)]
struct PageSelector {
    scan: ScanParams,
    last_seen: String,
}

#[endpoint {
    method = GET,
    path = "/super_pages",
}]
async fn handler(
    _rqctx: Arc<RequestContext<()>>,
    _query: Query<PaginationParams<ScanParams, PageSelector>>,
) -> Result<HttpResponseOk<ResultsPage<ResponseItem>>, HttpError> {
    unimplemented!();
}

#[test]
fn test_pagination_schema() -> Result<(), String> {
    let mut api = ApiDescription::new();
    api.register(handler)?;
    let mut output = Cursor::new(Vec::new());

    let _ = api
        .openapi("test", "1985.7")
        .description("gusty winds may exist")
        .contact_name("old mate")
        .license_name("CDDL")
        .terms_of_service("no hat, no cane? no service!")
        .write(&mut output);
    let actual = from_utf8(&output.get_ref()).unwrap();

    expectorate::assert_contents("tests/test_pagination_schema.json", actual);
    Ok(())
}
