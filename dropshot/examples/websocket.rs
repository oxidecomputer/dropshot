// Copyright 2022 Oxide Computer Company
/*!
 * Example use of Dropshot with a websocket endpoint.
 */

use dropshot::channel;
use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpServerStarter;
use dropshot::Query;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;
use futures_util::SinkExt;
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;
use tungstenite::protocol::Role;
use tungstenite::Message;

#[tokio::main]
async fn main() -> Result<(), String> {
    /*
     * We must specify a configuration with a bind address.  We'll use 127.0.0.1
     * since it's available and won't expose this server outside the host.  We
     * request port 0, which allows the operating system to pick any available
     * port.
     */
    let config_dropshot: ConfigDropshot = Default::default();

    /*
     * For simplicity, we'll configure an "info"-level logger that writes to
     * stderr assuming that it's a terminal.
     */
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    /*
     * Build a description of the API.
     */
    let mut api = ApiDescription::new();
    api.register(example_api_websocket_counter).unwrap();

    /*
     * Set up the server.
     */
    let server = HttpServerStarter::new(&config_dropshot, api, (), &log)
        .map_err(|error| format!("failed to create server: {}", error))?
        .start();

    /*
     * Wait for the server to stop.  Note that there's not any code to shut down
     * this server, so we should never get past this point.
     */
    server.await
}

/*
 * HTTP API interface
 */

#[derive(Deserialize, JsonSchema)]
struct QueryParams {
    start: Option<u8>,
}

/**
 * An eternally-increasing sequence of bytes, wrapping on overflow, starting
 * from the value given for the query parameter "start."
 */
#[channel {
    protocol = WEBSOCKETS,
    path = "/counter",
}]
async fn example_api_websocket_counter(
    _rqctx: Arc<RequestContext<()>>,
    upgraded: WebsocketConnection,
    qp: Query<QueryParams>,
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
