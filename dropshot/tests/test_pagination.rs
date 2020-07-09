// Copyright 2020 Oxide Computer Company
/*!
 * Test cases for API handler functions that use pagination.
 */

use dropshot::endpoint;
use dropshot::test_util::objects_list_page;
use dropshot::ApiDescription;
use dropshot::ExtractedParameter;
use dropshot::HttpError;
use dropshot::HttpResponseOkPage;
use dropshot::PaginationParams;
use dropshot::Query;
use dropshot::RequestContext;
use schemars::JsonSchema;
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

#[derive(Deserialize, ExtractedParameter, JsonSchema, Serialize)]
struct IntegersByNumber {
    n: u32,
}

#[endpoint {
    method = GET,
    path = "/testing/the_integers",
}]
async fn demo_handler_integers(
    _rqctx: Arc<RequestContext>,
    query: Query<PaginationParams<IntegersByNumber>>,
) -> Result<HttpResponseOkPage<IntegersByNumber, u32>, HttpError> {
    let pag_params = query.into_inner();
    let start = pag_params.marker.as_ref().map(|m| m.page_start.n).unwrap_or(1);

    /* XXX disallow limit=0 */

    let limit = if let Some(limit) = pag_params.limit { limit } else { 100 };
    let range = Range {
        start,
        end: start + (limit as u32),
    };

    let results = range.collect();

    Ok(HttpResponseOkPage(pag_params, results))
}

impl From<&u32> for IntegersByNumber {
    fn from(last_seen: &u32) -> IntegersByNumber {
        IntegersByNumber {
            n: *last_seen,
        }
    }
}

#[tokio::test]
async fn test_paginate_basic() {
    let api = paginate_api();
    let testctx = common::test_setup("demo1", api);
    let client = &testctx.client_testctx;

    let numbers = objects_list_page::<IntegersByNumber, u32>(
        &client,
        "/testing/the_integers?limit=5",
    )
    .await;
    assert_eq!(numbers, vec![ 1, 2, 3, 4, 5 ]);

    let numbers = objects_list_page::<IntegersByNumber, u32>(
        &client,
        "/testing/the_integers?limit=8",
    )
    .await;
    assert_eq!(numbers, vec![ 1, 2, 3, 4, 5, 6, 7, 8 ]);

    let numbers = objects_list_page::<IntegersByNumber, u32>(
        &client,
        "/testing/the_integers?limit=8&order=descending",
    )
    .await;
    assert_eq!(numbers, vec![ 8, 7, 6, 5, 4, 3, 2, 1 ]);

    testctx.teardown().await;
}
