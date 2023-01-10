// Copyright 2022 Oxide Computer Company

use dropshot::{
    endpoint, ApiDescription, HttpError, HttpResponseOk, Path, RequestContext,
};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(JsonSchema, Deserialize)]
#[allow(dead_code)]
struct MyPath {
    #[serde(rename = "type")]
    t: String,
    #[serde(rename = "ref")]
    r: String,
    #[serde(rename = "@")]
    at: String,
}

// The path variables are not valid identifiers, but they match the serde
// renames in the corresponding struct.
#[endpoint {
    method = GET,
    path = "/{type}/{ref}/{@}",
}]
async fn handler(
    _rqctx: RequestContext<()>,
    _path: Path<MyPath>,
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

#[test]
fn test_path_names() -> Result<(), String> {
    let mut api = ApiDescription::new();
    api.register(handler)?;
    Ok(())
}
