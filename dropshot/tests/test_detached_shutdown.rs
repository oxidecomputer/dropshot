// Copyright 2023 Oxide Computer Company

//! Test cases for graceful shutdown of a server running tasks in
//! `HandlerTaskMode::Detached`.

use dropshot::Body;
use dropshot::{
    endpoint, ApiDescription, HandlerTaskMode, HttpError, RequestContext,
};
use http::{Method, Response, StatusCode};
use std::time::Duration;
use tokio::sync::mpsc;

pub mod common;

struct Context {
    endpoint_started_tx: mpsc::UnboundedSender<()>,
    release_endpoint_rx: async_channel::Receiver<()>,
}

fn api() -> ApiDescription<Context> {
    let mut api = ApiDescription::new();
    api.register(root).unwrap();
    api
}

#[endpoint {
    method = GET,
    path = "/",
}]
async fn root(
    rqctx: RequestContext<Context>,
) -> Result<Response<Body>, HttpError> {
    let ctx = rqctx.context();

    // Notify test driver we've started handling a request.
    ctx.endpoint_started_tx.send(()).unwrap();

    // Wait until the test driver tells us to return.
    () = ctx.release_endpoint_rx.recv().await.unwrap();

    Ok(Response::builder().status(StatusCode::OK).body(Body::empty())?)
}

#[tokio::test]
async fn test_graceful_shutdown_with_detached_handler() {
    let (endpoint_started_tx, mut endpoint_started_rx) =
        mpsc::unbounded_channel();
    let (release_endpoint_tx, release_endpoint_rx) = async_channel::unbounded();

    let api = api();
    let testctx = common::test_setup_with_context(
        "graceful_shutdown_with_detached_handler",
        api,
        Context { endpoint_started_tx, release_endpoint_rx },
        HandlerTaskMode::Detached,
    );
    let client = testctx.client_testctx.clone();

    // Spawn a task sending a request to our endpoint.
    let client_task = tokio::spawn(async move {
        client
            .make_request_no_body(Method::GET, "/", StatusCode::OK)
            .await
            .expect("Expected GET request to succeed")
    });

    // Wait for the handler to start running.
    () = endpoint_started_rx.recv().await.unwrap();

    // Kill the client, which cancels the dropshot server future that spawned
    // our detached handler, but does not cancel the endpoint future itself
    // (because we're using HandlerTaskMode::Detached).
    client_task.abort();

    // Create a future to tear down the server.
    let teardown_fut = testctx.teardown();
    tokio::pin!(teardown_fut);

    // Actually tearing down should time out, because it's waiting for the
    // handler to return (which in turn is waiting on us to signal it!).
    if tokio::time::timeout(Duration::from_secs(2), &mut teardown_fut)
        .await
        .is_ok()
    {
        panic!("server shutdown returned while handler running");
    }

    // Signal the handler to complete.
    release_endpoint_tx.send(()).await.unwrap();

    // Now we can finish waiting for server shutdown.
    teardown_fut.await;
}
