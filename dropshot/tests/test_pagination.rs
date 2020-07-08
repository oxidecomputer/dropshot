// Copyright 2020 Oxide Computer Company
/*!
 * Test cases for API handler functions that use pagination.
 */

use dropshot::endpoint;
use dropshot::test_util::objects_list_page;
use dropshot::test_util::LogContext;
use dropshot::test_util::TestContext;
use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingIfExists;
use dropshot::ConfigLoggingLevel;
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

/* XXX commonize with test_demo.rs */
fn test_setup(test_name: &str) -> TestContext {
    let config_dropshot = ConfigDropshot {
        bind_address: "127.0.0.1:0".parse().unwrap(),
    };

    let config_logging = ConfigLogging::File {
        level: ConfigLoggingLevel::Debug,
        path: "UNUSED".to_string(),
        if_exists: ConfigLoggingIfExists::Fail,
    };

    let mut api = ApiDescription::new();
    register_test_endpoints(&mut api);
    let logctx = LogContext::new(test_name, &config_logging);

    let log = logctx.log.new(o!());
    TestContext::new(
        api,
        Arc::new(0 as usize),
        &config_dropshot,
        Some(logctx),
        log,
    )
}

#[tokio::test]
async fn test_paginate_basic() {
    let testctx = test_setup("demo1");
    let client = &testctx.client_testctx;

    let numbers =
        objects_list_page::<IntegersByNumber, u32>(&client, "/testing/the_integers?limit=5").await;
    eprintln!("numbers: {:?}", numbers);

    let numbers =
        objects_list_page::<IntegersByNumber, u32>(&client, "/testing/the_integers?limit=8").await;
    eprintln!("numbers: {:?}", numbers);

    let numbers = objects_list_page::<IntegersByNumber, u32>(
        &client,
        "/testing/the_integers?limit=8&order=descending",
    )
    .await;
    eprintln!("numbers: {:?}", numbers);

    testctx.teardown().await;
}

pub fn register_test_endpoints(api: &mut ApiDescription) {
    api.register(demo_handler_integers).unwrap();
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
