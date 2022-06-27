// Copyright 2021 Oxide Computer Company

//! Test cases for websockets.

use dropshot::{
    endpoint, ApiDescription, HttpError, HttpResponseUpgraded, RequestContext,
    WebSocketExt,
};
use futures::{FutureExt, SinkExt, StreamExt};
use http::{Method, StatusCode};
use std::sync::Arc;

extern crate slog;

pub mod common;

fn api() -> ApiDescription<usize> {
    let mut api = ApiDescription::new();
    api.register(websocket).unwrap();
    api
}

#[allow(unused_variables)]
#[endpoint {
    method = GET,
    path = "/echo",
}]
/// Echo a message back to the client.
async fn websocket(
    rqctx: Arc<RequestContext<usize>>,
) -> Result<HttpResponseUpgraded, HttpError> {
    rqctx
        .upgrade(|ws| {
            // Just echo all messages back...
            let (tx, rx) = ws.split();
            rx.forward(tx).map(|result| {
                if let Err(e) = result {
                    eprintln!("websocket error: {:?}", e);
                }
            })
        })
        .await
}

#[tokio::test]
async fn test_websocket_server_websocket_client() {
    let api = api();
    let testctx = common::test_setup("websocket_server_websocket_client", api);
    let client = &testctx.client_testctx;

    let url = client.url("/echo").to_string().replace("http://", "ws://");

    // Actually make a websocket connection.
    let (mut ws_stream, _) =
        tokio_tungstenite::connect_async(url).await.unwrap();

    // Write a message.
    ws_stream
        .send(tokio_tungstenite::tungstenite::Message::Text(
            "Hello, world!".to_string(),
        ))
        .await
        .unwrap();

    // Get the first message.
    let msg = ws_stream.next().await.unwrap().unwrap();
    if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
        assert_eq!(text, "Hello, world!");
    } else {
        unreachable!();
    }

    testctx.teardown().await;
}

#[tokio::test]
async fn test_websocket_server_no_upgrade_header() {
    let api = api();
    let testctx = common::test_setup("websocket_server_no_upgrade_header", api);
    let client = &testctx.client_testctx;

    if let Err(e) = client
        .make_request_no_body(Method::GET, "/echo", StatusCode::BAD_REQUEST)
        .await
    {
        assert_eq!(e.message, "Connection header not sent");
    } else {
        unreachable!();
    }
}

#[tokio::test]
async fn test_websocket_server_upgrade_header_incorrect() {
    let api = api();
    let testctx =
        common::test_setup("websocket_server_upgrade_header_incorrect", api);
    let client = &testctx.client_testctx;

    let uri = client.url("/echo");
    let request = hyper::Request::builder()
        .method(Method::GET)
        .header("Connection", "BAD")
        .uri(uri)
        .body(hyper::Body::empty())
        .expect("attempted to construct invalid request");

    if let Err(e) =
        client.make_request_with_request(request, StatusCode::BAD_REQUEST).await
    {
        assert_eq!(e.message, "Connection header did not include 'upgrade'");
    } else {
        unreachable!();
    }
}

#[tokio::test]
async fn test_websocket_server_no_websocket_key() {
    let api = api();
    let testctx = common::test_setup("websocket_server_no_websocket_key", api);
    let client = &testctx.client_testctx;

    let uri = client.url("/echo");
    let request = hyper::Request::builder()
        .method(Method::GET)
        .header("Connection", "Upgrade")
        .uri(uri)
        .body(hyper::Body::empty())
        .expect("attempted to construct invalid request");

    if let Err(e) =
        client.make_request_with_request(request, StatusCode::BAD_REQUEST).await
    {
        assert_eq!(e.message, "Websocket key not sent");
    } else {
        unreachable!();
    }
}
