// Copyright 2022 Oxide Computer Company
//! Example use of Dropshot with a websocket endpoint.

use dropshot::ApiDescription;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::Query;
use dropshot::RequestContext;
use dropshot::ServerBuilder;
use dropshot::WebsocketConnection;
use dropshot::channel;
use futures::SinkExt;
use schemars::JsonSchema;
use serde::Deserialize;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::Role;

#[tokio::main]
async fn main() -> Result<(), String> {
    // See dropshot/examples/basic.rs for more details on most of these pieces.
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    let mut api = ApiDescription::new();
    api.register(example_api_websocket_counter).unwrap();

    let server = ServerBuilder::new(api, (), log)
        .start()
        .map_err(|error| format!("failed to create server: {}", error))?;

    server.await
}

// HTTP API interface

#[derive(Deserialize, JsonSchema)]
struct QueryParams {
    start: Option<u8>,
}

/// An eternally-increasing sequence of bytes, wrapping on overflow, starting
/// from the value given for the query parameter "start."
#[channel {
    protocol = WEBSOCKETS,
    path = "/counter",
}]
async fn example_api_websocket_counter(
    _rqctx: RequestContext<()>,
    qp: Query<QueryParams>,
    upgraded: WebsocketConnection,
) -> dropshot::WebsocketChannelResult {
    let mut ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
        upgraded.into_inner(),
        Role::Server,
        None,
    )
    .await;
    let mut count = qp.into_inner().start.unwrap_or(0);
    while ws.send(Message::Binary(vec![count])).await.is_ok() {
        count = count.wrapping_add(1);
    }
    Ok(())
}
