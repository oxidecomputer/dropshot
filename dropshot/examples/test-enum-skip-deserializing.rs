// Copyright 2022 Oxide Computer Company

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::HttpServerStarter;
use dropshot::RequestContext;
use dropshot::TypedBody;
use parse_display::Display;
use parse_display::FromStr;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_with::DeserializeFromStr;
use serde_with::SerializeDisplay;
use slog::info;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), String> {
    let config_dropshot: ConfigDropshot = Default::default();
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("test-skip-deserializing")
        .map_err(|error| format!("failed to create logger: {}", error))?;
    let mut api = ApiDescription::new();
    api.register(example_api_echo_1).unwrap();
    api.register(example_api_echo_2).unwrap();
    api.openapi("test-deserialize", "0.0.0").write(&mut std::io::stdout()).unwrap();
    let api_context = ();
    let server =
        HttpServerStarter::new(&config_dropshot, api, api_context, &log)
            .map_err(|error| format!("failed to create server: {}", error))?
            .start();
    server.await
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
struct MyThing1 {
    kind: MyThingKind1,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
enum MyThingKind1 {
    Variant1,
    Variant2,
    #[serde(skip_deserializing)]
    Variant3,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
struct MyThing2 {
    kind: MyThingKind2,
}

#[derive(
    Display, FromStr, SerializeDisplay, DeserializeFromStr, Debug, JsonSchema,
)]
enum MyThingKind2 {
    Variant1,
    Variant2,
    #[serde(skip_deserializing)]
    Variant3,
}

type ExampleContext = ();

/** Return the thing we were given */
#[endpoint {
    method = PUT,
    path = "/thing1",
}]
async fn example_api_echo_1(
    rqctx: Arc<RequestContext<ExampleContext>>,
    given: TypedBody<MyThing1>,
) -> Result<HttpResponseOk<MyThing1>, HttpError> {
    info!(rqctx.log, "got"; "given" => ?given);
    Ok(HttpResponseOk(given.into_inner()))
}

/** Return the thing we were given */
#[endpoint {
    method = PUT,
    path = "/thing2",
}]
async fn example_api_echo_2(
    rqctx: Arc<RequestContext<ExampleContext>>,
    given: TypedBody<MyThing2>,
) -> Result<HttpResponseOk<MyThing2>, HttpError> {
    info!(rqctx.log, "got"; "given" => ?given);
    Ok(HttpResponseOk(given.into_inner()))
}
