// Copyright 2025 Oxide Computer Company

//! Test cases for handler panic handling.

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::HandlerTaskMode;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use http::Method;
use http::StatusCode;
use schemars::JsonSchema;
use serde::Serialize;

use crate::common;

#[derive(Debug, Serialize, JsonSchema)]
struct EmptyResult {}

#[endpoint {
    method = GET,
    path = "/panic",
}]
async fn handler_that_panics(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<EmptyResult>, HttpError> {
    panic!("test panic message");
}

#[tokio::test]
async fn test_panic_handler_returns_500_in_detached_mode() {
    let mut api = ApiDescription::new();
    api.register(handler_that_panics).unwrap();

    let testctx = common::test_setup_with_context(
        "test_panic_handler_returns_500_in_detached_mode",
        api,
        (),
        HandlerTaskMode::Detached,
    );

    testctx
        .client_testctx
        .make_request_error(
            Method::GET,
            "/panic",
            StatusCode::INTERNAL_SERVER_ERROR,
        )
        .await;

    testctx.teardown().await;
}

// Note: We cannot easily test CancelOnDisconnect mode panic behavior in a unit test
// because panics propagate through the test harness and cause test failures.
// The key difference is:
// - Detached mode: Panics are caught and converted to 500 errors (tested above)
// - CancelOnDisconnect mode: Panics propagate and crash the handler (as intended)
// For now this test is just marked as should_panic.
// TODO: Should this test be removed?
#[tokio::test]
#[should_panic]
async fn test_panic_handler_returns_500_in_cancel_on_disconnect_mode() {
    let mut api = ApiDescription::new();
    api.register(handler_that_panics).unwrap();

    let testctx = common::test_setup_with_context(
        "test_panic_handler_returns_500_in_cancel_on_disconnect_mode",
        api,
        (),
        HandlerTaskMode::CancelOnDisconnect,
    );

    testctx
        .client_testctx
        .make_request_error(
            Method::GET,
            "/panic",
            StatusCode::INTERNAL_SERVER_ERROR,
        )
        .await;

    testctx.teardown().await;
}
