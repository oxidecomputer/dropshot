// Copyright 2020 Oxide Computer Company
/*!
 * Test cases for API handler functions that use pagination.
 */

use dropshot::endpoint;
use dropshot::test_util::objects_list_page;
use dropshot::test_util::ClientTestContext;
use dropshot::ApiDescription;
use dropshot::ExtractedParameter;
use dropshot::HttpError;
use dropshot::HttpResponseOkPage;
use dropshot::PaginationParams;
use dropshot::Query;
use dropshot::RequestContext;
use http::Method;
use http::StatusCode;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::num::NonZeroU64;
use std::ops::Range;
use std::sync::Arc;

#[macro_use]
extern crate slog;

mod common;

fn paginate_api() -> ApiDescription {
    let mut api = ApiDescription::new();
    api.register(demo_handler_integers).unwrap();
    api
}

#[derive(Debug, Deserialize, ExtractedParameter, JsonSchema, Serialize)]
struct IntegersByNumber {
    n: u64,
}

#[endpoint {
    method = GET,
    path = "/testing/the_integers",
}]
async fn demo_handler_integers(
    _rqctx: Arc<RequestContext>,
    query: Query<PaginationParams<IntegersByNumber>>,
) -> Result<HttpResponseOkPage<IntegersByNumber, u64>, HttpError> {
    let pag_params = query.into_inner();
    let start = pag_params.marker.as_ref().map(|m| m.page_start.n).unwrap_or(0);

    let limit = if let Some(limit) = pag_params.limit {
        limit
    } else {
        NonZeroU64::new(100).unwrap()
    };
    let range = Range {
        start: start + 1,
        end: start + (u64::from(limit)) + 1,
    };

    let results = range.collect();

    Ok(HttpResponseOkPage(pag_params, results))
}

impl From<&u64> for IntegersByNumber {
    fn from(last_seen: &u64) -> IntegersByNumber {
        IntegersByNumber {
            n: *last_seen,
        }
    }
}

#[tokio::test]
async fn test_paginate_basic_errors() {
    let api = paginate_api();
    let testctx = common::test_setup("demo1", api);
    let client = &testctx.client_testctx;

    struct ErrorTestCase {
        path: &'static str,
        message: &'static str,
    };
    let test_cases = vec![
        ErrorTestCase {
            path: "/testing/the_integers?limit=0",
            message: "unable to parse query string: expected a non-zero value",
        },
        ErrorTestCase {
            path: "/testing/the_integers?limit=-3",
            message: "unable to parse query string: invalid digit found in \
                      string",
        },
        ErrorTestCase {
            path: "/testing/the_integers?limit=seven",
            message: "unable to parse query string: invalid digit found in \
                      string",
        },
        ErrorTestCase {
            path: "/testing/the_integers?order=boom",
            message: "unable to parse query string: unknown variant `boom`, \
                      expected `ascending` or `descending`",
        },
    ];

    for tc in test_cases {
        assert_error(client, tc.path, tc.message).await;
    }
}

async fn assert_error(
    client: &ClientTestContext,
    path: &str,
    expected_message: &str,
) {
    let error = client
        .make_request_error(Method::GET, path, StatusCode::BAD_REQUEST)
        .await;
    assert_eq!(error.message, expected_message,);
    assert_eq!(error.error_code, None);
}

#[tokio::test]
async fn test_paginate_basic() {
    let api = paginate_api();
    let testctx = common::test_setup("demo1", api);
    let client = &testctx.client_testctx;

    let page = objects_list_page::<IntegersByNumber, u32>(
        &client,
        "/testing/the_integers?limit=5",
    )
    .await;
    assert_eq!(page.items, vec![1, 2, 3, 4, 5]);
    eprintln!("page: {:?}", page);

    let page = objects_list_page::<IntegersByNumber, u32>(
        &client,
        "/testing/the_integers?limit=8",
    )
    .await;
    assert_eq!(page.items, vec![1, 2, 3, 4, 5, 6, 7, 8]);

    let page = objects_list_page::<IntegersByNumber, u32>(
        &client,
        "/testing/the_integers?limit=8&order=descending",
    )
    .await;
    assert_eq!(page.items, vec![8, 7, 6, 5, 4, 3, 2, 1]);

    // XXX
    let marker: String = serde_json::from_str(&page.next_page.unwrap()).unwrap();
    let page = objects_list_page::<IntegersByNumber, u32>(
        &client,
        &format!("/testing/the_integers?limit=8&order=descending&marker={}", marker),
    )
    .await;
    assert_eq!(page.items, vec![16, 15, 14, 13, 12, 11, 10, 9]);

    testctx.teardown().await;
}
