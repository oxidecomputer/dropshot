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
use dropshot::PaginatedResource;
use dropshot::PaginationParams;
use dropshot::Query;
use dropshot::RequestContext;
use dropshot::WhichPage;
use http::Method;
use http::StatusCode;
use serde::Deserialize;
use serde::Serialize;
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

#[endpoint {
    method = GET,
    path = "/testing/intapi",
}]
async fn demo_handler_integers(
    rqctx: Arc<RequestContext>,
    query: Query<PaginationParams<PaginatedIntegers>>,
) -> Result<HttpResponseOkPage<usize>, HttpError> {
    let pag_params = query.into_inner();
    let limit = rqctx.page_limit(&pag_params)?.get();

    let start = match &pag_params.page_params {
        WhichPage::FirstPage {
            ..
        } => 0,
        WhichPage::NextPage {
            page_token,
        } => {
            let IntegersPageSelector::ByNum(n) = page_token.page_start;
            n as usize
        }
    };

    let results = Range {
        start: start + 1,
        end: start + limit + 1,
    }
    .collect();

    Ok(HttpResponseOkPage::new_with_paginator::<PaginatedIntegers, _>(
        results,
        &IntegersScanMode::ByNum,
        page_selector_for,
    )?)
}

#[derive(Deserialize)] // XXX
struct PaginatedIntegers;
#[derive(Debug, Deserialize, ExtractedParameter)]
enum IntegersScanMode {
    ByNum,
}
#[derive(Debug, Deserialize, ExtractedParameter, Serialize)]
enum IntegersPageSelector {
    ByNum(usize),
}
impl PaginatedResource for PaginatedIntegers {
    type ScanMode = IntegersScanMode;
    type PageSelector = IntegersPageSelector;
    type Item = usize;
}

fn page_selector_for(n: &usize, _p: &IntegersScanMode) -> IntegersPageSelector {
    IntegersPageSelector::ByNum(*n)
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
            path: "/testing/intapi?limit=0",
            message: "unable to parse query string: expected a non-zero value",
        },
        ErrorTestCase {
            path: "/testing/intapi?limit=-3",
            message: "unable to parse query string: invalid digit found in \
                      string",
        },
        ErrorTestCase {
            path: "/testing/intapi?limit=seven",
            message: "unable to parse query string: invalid digit found in \
                      string",
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

    let page =
        objects_list_page::<usize>(&client, "/testing/intapi?limit=5").await;
    assert_eq!(page.items, vec![1, 2, 3, 4, 5]);
    eprintln!("page: {:?}", page);

    let page =
        objects_list_page::<usize>(&client, "/testing/intapi?limit=8").await;
    assert_eq!(page.items, vec![1, 2, 3, 4, 5, 6, 7, 8]);

    testctx.teardown().await;
}
