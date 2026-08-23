// Copyright 2026 Oxide Computer Company

//! Tests for the `query-parameters` example.

use dropshot::test_util::read_json;
use http::Method;
use http::StatusCode;

use crate::common;

#[path = "../../examples/query-parameters.rs"]
#[allow(dead_code)]
mod example;

#[tokio::test]
async fn test_query_parameter_lists() {
    let error = example::register_vec()
        .expect_err("expected Dropshot to reject a Vec query parameter");
    assert_eq!(
        error.to_string(),
        "failed to register endpoint 'get_derived_vector': for endpoint get_derived_vector the parameter 'state' must have a scalar type",
    );

    let error =
        example::deserialize_derived_vector("state=active&state=closed")
            .expect_err(
                "expected serde_urlencoded to reject repeated Vec values",
            );
    assert_eq!(
        error.to_string(),
        "invalid type: string \"active\", expected a sequence",
    );

    let error = example::deserialize_derived_vector("state=active,closed")
        .expect_err(
            "expected serde_urlencoded to reject a comma-separated Vec",
        );
    assert_eq!(
        error.to_string(),
        "invalid type: string \"active,closed\", expected a sequence",
    );

    let testctx = common::test_setup("query_parameter_lists", example::api());

    let mut response = testctx
        .client_testctx
        .make_request_no_body(
            Method::GET,
            "/state/scalar?state=active",
            StatusCode::OK,
        )
        .await
        .expect("expected a scalar query parameter to work");
    assert_eq!(
        read_json::<example::State>(&mut response).await,
        example::State::Active,
    );

    let error = testctx
        .client_testctx
        .make_request_no_body(
            Method::GET,
            "/state/scalar?state=active&state=closed",
            StatusCode::BAD_REQUEST,
        )
        .await
        .expect_err("expected a repeated scalar query parameter to fail");
    assert_eq!(
        error.message,
        "unable to parse query string: duplicate field `state`",
    );

    let mut response = testctx
        .client_testctx
        .make_request_no_body(
            Method::GET,
            "/states/repeated?state=active&state=closed",
            StatusCode::OK,
        )
        .await
        .expect("expected custom repeated query parameters to work");
    assert_eq!(
        read_json::<Vec<example::State>>(&mut response).await,
        vec![example::State::Active, example::State::Closed],
    );

    let mut response = testctx
        .client_testctx
        .make_request_no_body(
            Method::GET,
            "/states/comma-separated?state=active,closed",
            StatusCode::OK,
        )
        .await
        .expect("expected custom comma-separated query parameters to work");
    assert_eq!(
        read_json::<Vec<example::State>>(&mut response).await,
        vec![example::State::Active, example::State::Closed],
    );

    testctx.teardown().await;
}
