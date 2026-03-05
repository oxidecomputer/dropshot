// Copyright 2024 Oxide Computer Company

//! Test cases for API handler functions that use pagination.

use chrono::DateTime;
use chrono::Utc;
use dropshot::endpoint;
use dropshot::test_util::iter_collection;
use dropshot::test_util::object_get;
use dropshot::test_util::objects_list_page;
use dropshot::test_util::ClientTestContext;
use dropshot::test_util::LogContext;
use dropshot::ApiDescription;
use dropshot::Body;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingIfExists;
use dropshot::ConfigLoggingLevel;
use dropshot::EmptyScanParams;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::PaginationOrder;
use dropshot::PaginationParams;
use dropshot::Query;
use dropshot::RequestContext;
use dropshot::ResultsPage;
use dropshot::WhichPage;
use http::Method;
use http::StatusCode;
use hyper::Request;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeSet;
use std::env::current_exe;
use std::fmt::Debug;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::ops::Bound;
use std::sync::atomic::AtomicU16;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use subprocess::Exec;
use subprocess::Job;
use subprocess::Redirection;
use uuid::Uuid;

use crate::common;

// Common helpers

/// Given a test context and URL path, assert that a GET request to that path
/// (with an empty body) produces a 400 response with the given error message.
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

/// Given an array of integers, check that they're sequential starting at
/// "offset".
fn assert_sequence_from(items: &Vec<u16>, offset: u16, count: u16) {
    let nchecked = AtomicU16::new(0);
    items.iter().enumerate().for_each(|(i, c)| {
        assert_eq!(*c, (i as u16) + offset);
        nchecked.fetch_add(1, Ordering::SeqCst);
    });
    assert_eq!(nchecked.load(Ordering::SeqCst) as usize, items.len());
    assert_eq!(count as usize, items.len());
}

/// Iterate the paginated collection using several different "limit" values to
/// validate that it always produces the same collection (no dups or missing
/// records around page breaks).
/// TODO This should move into test_util so that consumers can use it to test
/// their own APIs.
async fn assert_collection_iter<T>(
    client: &ClientTestContext,
    path: &str,
    initial_params: &str,
) -> Vec<T>
where
    T: Clone + Debug + Eq + DeserializeOwned,
{
    // Use a modest small number for our initial limit.
    let (itemsby100, npagesby100) =
        iter_collection::<T>(client, path, initial_params, 100).await;
    let expected_npages = itemsby100.len() / 100
        + 1
        + (if itemsby100.len() % 100 != 0 { 1 } else { 0 });
    assert_eq!(expected_npages, npagesby100);

    // Assert that there are between 100 and 10000 items.  It's not really a
    // problem for there to be a number outside this range.  However, our goal
    // here is to independently exercise a modest limit (small but larger than
    // 1) and also to check it against a max-limit request that's expected to
    // have all the results, and we can't do that easily unless this condition
    // holds.  We could skip these checks if it's useful to have tests that work
    // that way, but for now we assert this so that we find out if we're somehow
    // not testing what we expect.
    assert!(itemsby100.len() > 100);
    assert!(itemsby100.len() <= 10000);

    // Use a max limit to fetch everything at once to make sure it's the same.
    let (itemsbymax, npagesbymax) =
        iter_collection::<T>(client, path, initial_params, 10000).await;
    assert_eq!(2, npagesbymax);
    assert_eq!(itemsby100, itemsbymax);

    // Iterate by one to make sure that edge case works, too.
    let (itemsby1, npagesby1) =
        iter_collection::<T>(client, path, initial_params, 1).await;
    assert_eq!(itemsby100.len() + 1, npagesby1);
    assert_eq!(itemsby1, itemsby100);

    itemsbymax
}

/// Page selector for a set of "u16" values
///
/// This is used for several resources below.
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct IntegersPageSelector {
    last_seen: u16,
}

fn page_selector_for(n: &u16, _p: &EmptyScanParams) -> IntegersPageSelector {
    IntegersPageSelector { last_seen: *n }
}

/// Define an API with a couple of different endpoints that allow us to exercise
/// various functionality.
fn paginate_api() -> ApiDescription<usize> {
    let mut api = ApiDescription::new();
    api.register(api_integers).unwrap();
    api.register(api_empty).unwrap();
    api.register(api_with_extra_params).unwrap();
    api.register(api_with_required_params).unwrap();
    api.register(api_dictionary).unwrap();
    api
}

fn range_u16(start: u16, limit: u16) -> Vec<u16> {
    if start < std::u16::MAX {
        let start = start + 1;
        let end = start.saturating_add(limit);
        (start..end).collect()
    } else {
        Vec::new()
    }
}

// Basic tests

/// "/intapi": a collection of positive values of "u16" (excepting u16::MAX).
/// The marker is simply the last number seen.
#[endpoint {
    method = GET,
    path = "/intapi",
}]
async fn api_integers(
    rqctx: RequestContext<usize>,
    query: Query<PaginationParams<EmptyScanParams, IntegersPageSelector>>,
) -> Result<HttpResponseOk<ResultsPage<u16>>, HttpError> {
    let pag_params = query.into_inner();
    let limit = rqctx.page_limit(&pag_params)?.get() as u16;

    let start = match &pag_params.page {
        WhichPage::First(..) => 0,
        WhichPage::Next(IntegersPageSelector { last_seen }) => *last_seen,
    };

    Ok(HttpResponseOk(ResultsPage::new(
        range_u16(start, limit),
        &EmptyScanParams {},
        page_selector_for,
    )?))
}

#[tokio::test]
async fn test_paginate_errors() {
    let api = paginate_api();
    let testctx = common::test_setup("errors", api);
    let client = &testctx.client_testctx;

    struct ErrorTestCase {
        path: String,
        message: &'static str,
    }
    let test_cases = vec![
        ErrorTestCase {
            path: "/intapi?limit=0".to_string(),
            message: "unable to parse query string: invalid value: \
                integer `0`, expected a nonzero u32",
        },
        ErrorTestCase {
            path: "/intapi?limit=-3".to_string(),
            message: "unable to parse query string: invalid digit found in \
                      string",
        },
        ErrorTestCase {
            path: "/intapi?limit=seven".to_string(),
            message: "unable to parse query string: invalid digit found in \
                      string",
        },
        ErrorTestCase {
            path: format!("/intapi?limit={}", u128::from(std::u64::MAX) + 1),
            message: "unable to parse query string: number too large to fit \
                      in target type",
        },
        ErrorTestCase {
            path: "/intapi?page_token=q".to_string(),
            message: "unable to parse query string: failed to parse \
                      pagination token: Invalid input length: 1",
        },
    ];

    for tc in test_cases {
        assert_error(client, &tc.path, tc.message).await;
    }

    testctx.teardown().await;
}

#[tokio::test]
async fn test_paginate_basic() {
    let api = paginate_api();
    let testctx = common::test_setup("basic", api);
    let client = &testctx.client_testctx;

    // "First page" test cases

    // Test the default value of "limit".  This test will have to be updated if
    // we change the default count of items, but it's important to check that
    // the default actually works and is reasonable.
    let expected_default = 100;
    let page = objects_list_page::<u16>(client, "/intapi").await;
    assert_sequence_from(&page.items, 1, expected_default);
    assert!(page.next_page.is_some());

    // Test the maximum value of "limit" by providing a value much higher than
    // we support and observing it get clamped.  As with the previous test, this
    // will have to be updated if we change the maximum count, but it's worth it
    // to test this case.
    let expected_max = 10000;
    let page = objects_list_page::<u16>(
        client,
        &format!("/intapi?limit={}", 2 * expected_max),
    )
    .await;
    assert_sequence_from(&page.items, 1, expected_max);

    // Limits in between the default and the max should also work.  This
    // exercises the `page_limit()` function.
    let count = 2 * expected_default;
    assert!(count > expected_default);
    assert!(count < expected_max);
    let page =
        objects_list_page::<u16>(client, &format!("/intapi?limit={}", count))
            .await;
    assert_sequence_from(&page.items, 1, count);

    // "Next page" test cases

    // Run the same few limit tests as above.
    let next_page_start = page.items.last().unwrap() + 1;
    let next_page_token = page.next_page.unwrap();

    let page = objects_list_page::<u16>(
        client,
        &format!("/intapi?page_token={}", next_page_token,),
    )
    .await;
    assert_sequence_from(&page.items, next_page_start, expected_default);
    assert!(page.next_page.is_some());

    let page = objects_list_page::<u16>(
        client,
        &format!(
            "/intapi?page_token={}&limit={}",
            next_page_token,
            2 * expected_max
        ),
    )
    .await;
    assert_sequence_from(&page.items, next_page_start, expected_max);
    assert!(page.next_page.is_some());

    let page = objects_list_page::<u16>(
        client,
        &format!("/intapi?page_token={}&limit={}", next_page_token, count),
    )
    .await;
    assert_sequence_from(&page.items, next_page_start, count);
    assert!(page.next_page.is_some());

    // Loop through the entire collection.
    let mut next_item = 1u16;
    let mut page = objects_list_page::<u16>(
        client,
        &format!("/intapi?limit={}", expected_max),
    )
    .await;
    loop {
        if let Some(ref next_token) = page.next_page {
            if page.items.len() != expected_max as usize {
                assert!(!page.items.is_empty());
                assert!(page.items.len() < expected_max as usize);
                assert_eq!(*page.items.last().unwrap(), std::u16::MAX - 1);
            }
            assert_sequence_from(
                &page.items,
                next_item,
                page.items.len() as u16,
            );
            next_item += page.items.len() as u16;
            page = objects_list_page::<u16>(
                client,
                &format!(
                    "/intapi?page_token={}&limit={}",
                    &next_token, expected_max
                ),
            )
            .await;
        } else {
            assert_eq!(page.items.len(), 0);
            break;
        }
    }

    testctx.teardown().await;
}

// Tests for an empty collection

/// "/empty": an empty collection of u16s, useful for testing the case where the
/// first request in a scan returns no results.
#[endpoint {
    method = GET,
    path = "/empty",
}]
async fn api_empty(
    _rqctx: RequestContext<usize>,
    _query: Query<PaginationParams<EmptyScanParams, IntegersPageSelector>>,
) -> Result<HttpResponseOk<ResultsPage<u16>>, HttpError> {
    Ok(HttpResponseOk(ResultsPage::new(
        Vec::new(),
        &EmptyScanParams {},
        page_selector_for,
    )?))
}

// Tests various cases related to an empty collection, particularly making sure
// that basic parsing of query parameters still does what we expect and that we
// get a valid results page with no objects.
#[tokio::test]
async fn test_paginate_empty() {
    let api = paginate_api();
    let testctx = common::test_setup("empty", api);
    let client = &testctx.client_testctx;

    let page = objects_list_page::<u16>(client, "/empty").await;
    assert_eq!(page.items.len(), 0);
    assert!(page.next_page.is_none());

    let page = objects_list_page::<u16>(client, "/empty?limit=10").await;
    assert_eq!(page.items.len(), 0);
    assert!(page.next_page.is_none());

    assert_error(
        client,
        "/empty?limit=0",
        "unable to parse query string: invalid value: integer `0`, \
        expected a nonzero u32",
    )
    .await;

    assert_error(
        client,
        "/empty?page_token=q",
        "unable to parse query string: failed to parse pagination token: \
        Invalid input length: 1",
    )
    .await;

    testctx.teardown().await;
}

// Test extra query parameters and response properties

/// "/ints_extra": also a paginated collection of "u16" values.  This
/// API exercises consuming additional query parameters ("debug") and sending a
/// more complex response type.

#[endpoint {
    method = GET,
    path = "/ints_extra",
}]
async fn api_with_extra_params(
    rqctx: RequestContext<usize>,
    query_pag: Query<PaginationParams<EmptyScanParams, IntegersPageSelector>>,
    query_extra: Query<ExtraQueryParams>,
) -> Result<HttpResponseOk<ExtraResultsPage>, HttpError> {
    let pag_params = query_pag.into_inner();
    let limit = rqctx.page_limit(&pag_params)?.get() as u16;
    let extra_params = query_extra.into_inner();

    let start = match &pag_params.page {
        WhichPage::First(..) => 0,
        WhichPage::Next(IntegersPageSelector { last_seen }) => *last_seen,
    };

    Ok(HttpResponseOk(ExtraResultsPage {
        debug_was_set: extra_params.debug.is_some(),
        debug_value: extra_params.debug.unwrap_or(false),
        page: ResultsPage::new(
            range_u16(start, limit),
            &EmptyScanParams {},
            page_selector_for,
        )?,
    }))
}

// TODO-coverage check generated OpenAPI spec
#[derive(Deserialize, JsonSchema)]
struct ExtraQueryParams {
    debug: Option<bool>,
}

// TODO-coverage check generated OpenAPI spec
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct ExtraResultsPage {
    debug_was_set: bool,
    debug_value: bool,
    #[serde(flatten)]
    page: ResultsPage<u16>,
}

#[tokio::test]
async fn test_paginate_extra_params() {
    let api = paginate_api();
    let testctx = common::test_setup("extra_params", api);
    let client = &testctx.client_testctx;

    // Test that the extra query parameter is optional.
    let page =
        object_get::<ExtraResultsPage>(client, "/ints_extra?limit=5").await;
    assert!(!page.debug_was_set);
    assert!(!page.debug_value);
    assert_eq!(page.page.items, vec![1, 2, 3, 4, 5]);
    let token = page.page.next_page.unwrap();

    // Provide a value for the extra query parameter in the FirstPage case.
    let page = object_get::<ExtraResultsPage>(
        client,
        "/ints_extra?limit=5&debug=true",
    )
    .await;
    assert!(page.debug_was_set);
    assert!(page.debug_value);
    assert_eq!(page.page.items, vec![1, 2, 3, 4, 5]);
    assert!(page.page.next_page.is_some());

    // Provide a value for the extra query parameter in the NextPage case.
    let page = object_get::<ExtraResultsPage>(
        client,
        &format!("/ints_extra?page_token={}&debug=false&limit=7", token),
    )
    .await;
    assert_eq!(page.page.items, vec![6, 7, 8, 9, 10, 11, 12]);
    assert!(page.debug_was_set);
    assert!(!page.debug_value);
    assert!(page.page.next_page.is_some());

    testctx.teardown().await;
}

// Test an endpoint that requires scan parameters.

#[derive(Deserialize, JsonSchema)]
struct ReqScanParams {
    doit: bool,
}

/// "/required": similar to "/intapi", but with a required start parameter
#[endpoint {
    method = GET,
    path = "/required",
}]
async fn api_with_required_params(
    rqctx: RequestContext<usize>,
    query: Query<PaginationParams<ReqScanParams, IntegersPageSelector>>,
) -> Result<HttpResponseOk<ResultsPage<u16>>, HttpError> {
    let pag_params = query.into_inner();
    let limit = rqctx.page_limit(&pag_params)?.get() as u16;

    let start = match &pag_params.page {
        WhichPage::First(ReqScanParams { doit }) => {
            if !doit {
                return Err(HttpError::for_bad_request(
                    None,
                    String::from("you did not say to do it"),
                ));
            }

            0
        }
        WhichPage::Next(IntegersPageSelector { last_seen }) => *last_seen,
    };

    Ok(HttpResponseOk(ResultsPage::new(
        range_u16(start, limit),
        &EmptyScanParams {},
        page_selector_for,
    )?))
}

#[tokio::test]
async fn test_paginate_with_required_params() {
    let api = paginate_api();
    let testctx = common::test_setup("required_params", api);
    let client = &testctx.client_testctx;

    // Test that the extra query parameter is optional...
    let error = client
        .make_request_error(
            Method::GET,
            "/required?limit=3",
            StatusCode::BAD_REQUEST,
        )
        .await;
    // TODO-polish the message here is pretty poor.  See comments in the
    // automated tests in src/pagination.rs.
    assert!(error.message.starts_with("unable to parse query string"));

    // ... and that it's getting passed through to the handler function
    let error = client
        .make_request_error(
            Method::GET,
            "/required?limit=3&doit=false",
            StatusCode::BAD_REQUEST,
        )
        .await;
    assert_eq!(error.message, "you did not say to do it");

    let page =
        objects_list_page::<u16>(client, "/required?limit=3&doit=true").await;
    assert_eq!(page.items.len(), 3);

    testctx.teardown().await;
}

// Test an endpoint with scan options that returns custom structures.  Our
// endpoint will return a list of words, with the marker being the last word
// seen.

lazy_static! {
    static ref WORD_LIST: BTreeSet<String> = make_word_list();
}

fn make_word_list() -> BTreeSet<String> {
    let word_list = include_str!("../wordlist.txt");
    word_list.lines().map(|s| s.to_string()).collect()
}

// The use of a structure here is kind of pointless except to exercise the case
// of endpoints that return a custom structure.
#[derive(Debug, Deserialize, Clone, Eq, JsonSchema, PartialEq, Serialize)]
struct DictionaryWord {
    word: String,
    length: usize,
}

#[derive(Clone, Deserialize, JsonSchema, Serialize)]
struct DictionaryScanParams {
    #[serde(default = "ascending")]
    order: PaginationOrder,
    #[serde(default)]
    min_length: usize,
}

fn ascending() -> PaginationOrder {
    PaginationOrder::Ascending
}

#[derive(Deserialize, JsonSchema, Serialize)]
struct DictionaryPageSelector {
    scan: DictionaryScanParams,
    last_seen: String,
}

#[endpoint {
    method = GET,
    path = "/dictionary",
}]
async fn api_dictionary(
    rqctx: RequestContext<usize>,
    query: Query<
        PaginationParams<DictionaryScanParams, DictionaryPageSelector>,
    >,
) -> Result<HttpResponseOk<ResultsPage<DictionaryWord>>, HttpError> {
    let pag_params = query.into_inner();
    let limit = rqctx.page_limit(&pag_params)?.get() as usize;
    let dictionary: &BTreeSet<String> = &WORD_LIST;

    let (bound, scan_params) = match &pag_params.page {
        WhichPage::First(scan) => (Bound::Unbounded, scan),
        WhichPage::Next(DictionaryPageSelector { scan, last_seen }) => {
            (Bound::Excluded(last_seen), scan)
        }
    };

    let (range_bounds, reverse) = match scan_params.order {
        PaginationOrder::Ascending => ((bound, Bound::Unbounded), true),
        PaginationOrder::Descending => ((Bound::Unbounded, bound), false),
    };

    let iter = dictionary.range::<String, _>(range_bounds);
    let iter: Box<dyn Iterator<Item = &String>> =
        if reverse { Box::new(iter) } else { Box::new(iter.rev()) };
    let iter = iter.filter_map(|word| {
        if word.len() >= scan_params.min_length {
            Some(DictionaryWord { word: word.to_string(), length: word.len() })
        } else {
            None
        }
    });

    Ok(HttpResponseOk(ResultsPage::new(
        iter.take(limit).collect(),
        scan_params,
        |item: &DictionaryWord, scan_params: &DictionaryScanParams| {
            DictionaryPageSelector {
                scan: scan_params.clone(),
                last_seen: item.word.clone(),
            }
        },
    )?))
}

// These tests exercise the behavior of a paginated API with filtering and
// multiple sort options.  In some ways, these just test our test API.  But it's
// an important validation that it's possible to build such an API that works
// the way we expect it to.
#[tokio::test]
async fn test_paginate_dictionary() {
    let api = paginate_api();
    let testctx = common::test_setup("dictionary", api);
    let client = &testctx.client_testctx;

    // simple case
    let page =
        objects_list_page::<DictionaryWord>(client, "/dictionary?limit=3")
            .await;
    let found_words =
        page.items.iter().map(|dw| dw.word.as_str()).collect::<Vec<&str>>();
    assert_eq!(found_words, vec!["A&M", "A&P", "AAA",]);
    let token = page.next_page.unwrap();
    let page = objects_list_page::<DictionaryWord>(
        client,
        &format!("/dictionary?limit=3&page_token={}", token),
    )
    .await;
    let found_words =
        page.items.iter().map(|dw| dw.word.as_str()).collect::<Vec<&str>>();
    assert_eq!(found_words, vec!["AAAS", "ABA", "AC",]);

    // Reverse the order.
    let page = objects_list_page::<DictionaryWord>(
        client,
        "/dictionary?limit=3&order=descending",
    )
    .await;
    let found_words =
        page.items.iter().map(|dw| dw.word.as_str()).collect::<Vec<&str>>();
    assert_eq!(found_words, vec!["zygote", "zucchini", "zounds",]);
    let token = page.next_page.unwrap();
    // Critically, we don't have to pass order=descending again.
    let page = objects_list_page::<DictionaryWord>(
        client,
        &format!("/dictionary?limit=3&page_token={}", token),
    )
    .await;
    let found_words =
        page.items.iter().map(|dw| dw.word.as_str()).collect::<Vec<&str>>();
    assert_eq!(found_words, vec!["zooplankton", "zoom", "zoology",]);

    // Apply a filter.
    let page = objects_list_page::<DictionaryWord>(
        client,
        "/dictionary?limit=3&min_length=12",
    )
    .await;
    let found_words =
        page.items.iter().map(|dw| dw.word.as_str()).collect::<Vec<&str>>();
    assert_eq!(
        found_words,
        vec!["Addressograph", "Aristotelean", "Aristotelian",]
    );
    let token = page.next_page.unwrap();
    let page = objects_list_page::<DictionaryWord>(
        client,
        &format!("/dictionary?limit=3&page_token={}", token),
    )
    .await;
    let found_words =
        page.items.iter().map(|dw| dw.word.as_str()).collect::<Vec<&str>>();
    assert_eq!(
        found_words,
        vec!["Bhagavadgita", "Brontosaurus", "Cantabrigian",]
    );

    // Let's page through the filtered collection one item at a time.  This is
    // an edge case that only works if the marker is implemented correctly.
    let (sortedby1, npagesby1) = iter_collection::<DictionaryWord>(
        client,
        "/dictionary",
        "min_length=12",
        1,
    )
    .await;
    assert_eq!(sortedby1.len(), 1069);
    assert_eq!(npagesby1, sortedby1.len() + 1);
    assert_eq!(sortedby1[0].word, "Addressograph");
    assert_eq!(sortedby1[sortedby1.len() - 1].word, "wholehearted");

    // Page through it again one at a time, but in reverse order to make sure
    // the marker works correctly in that direction as well.
    let (rsortedby1, rnpagesby1) = iter_collection::<DictionaryWord>(
        client,
        "/dictionary",
        "min_length=12&order=descending",
        1,
    )
    .await;
    assert_eq!(npagesby1, rnpagesby1);
    assert_eq!(
        sortedby1,
        rsortedby1.iter().rev().cloned().collect::<Vec<DictionaryWord>>()
    );

    // Fetch the whole thing in one go to make sure we didn't hit any edge cases
    // around the markers.
    let (sortedbybig, npagesbybig) = iter_collection::<DictionaryWord>(
        client,
        "/dictionary",
        "min_length=12&order=ascending",
        10000,
    )
    .await;
    assert_eq!(sortedby1, sortedbybig);
    // There's currently an extra request as part of any scan because Dropshot
    // doesn't know not to put the marker in the response unless it sees an
    // empty response.
    assert_eq!(npagesbybig, 2);

    testctx.teardown().await;
}

struct ExampleContext {
    child: Job,
    client: ClientTestContext,
    logctx: Option<LogContext>,
}

impl ExampleContext {
    fn cleanup_successful(&mut self) {
        if let Some(l) = self.logctx.take() {
            l.cleanup_successful()
        }
    }
}

impl Drop for ExampleContext {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// For one of the example programs that starts a Dropshot server on localhost
/// using a TCP port provided as a command-line argument, this function starts
/// the requested example on the requested TCP port and attempts to wait for the
/// Dropshot server to become available.  It returns a handle to the child
/// process and a ClientTestContext for making requests against that server.
async fn start_example(path: &str, port: u16) -> ExampleContext {
    let logctx = LogContext::new(
        path,
        &ConfigLogging::File {
            level: ConfigLoggingLevel::Info,
            path: "UNUSED".into(),
            if_exists: ConfigLoggingIfExists::Fail,
        },
    );

    let log = logctx.log.new(o!());
    let cmd_path = {
        let mut my_path = current_exe().expect("failed to find test program");
        my_path.pop();
        assert_eq!(my_path.file_name().unwrap(), "deps");
        my_path.pop();
        my_path.push("examples");
        my_path.push(path);
        my_path
    };

    // We redirect stderr to /dev/null to avoid spamming the user's terminal.
    // It would be better to put this in some log file that we manage similarly
    // to a LogContext so that it would be available for debugging when wanted
    // but removed upon successful completion of the test.
    let config =
        Exec::cmd(cmd_path).arg(port.to_string()).stderr(Redirection::Null);
    let cmdline = config.to_cmdline_lossy();
    info!(&log, "starting child process"; "cmdline" => &cmdline);
    let child = config.start().unwrap();

    // Wait up to 10 seconds for the actual HTTP server to start up.  We'll
    // continue trying to make requests against it until they fail for an
    // HTTP-level error.
    let start = Instant::now();
    let server_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let client = ClientTestContext::new(server_addr, logctx.log.new(o!()));
    let url = client.url("/");
    let raw_client = hyper_util::client::legacy::Client::builder(
        hyper_util::rt::TokioExecutor::new(),
    )
    .build(hyper_util::client::legacy::connect::HttpConnector::new());
    let rv = ExampleContext { child, client, logctx: Some(logctx) };

    while start.elapsed().as_secs() < 10 {
        trace!(&log, "making test request to see if the server is up");
        let response = raw_client
            .request(
                Request::builder()
                    .method(Method::GET)
                    .uri(url.clone())
                    .body(Body::empty())
                    .expect("attempted to construct invalid request"),
            )
            .await;
        if response.is_ok() {
            return rv;
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    panic!(
        "failed to connect to example \"{}\" at \"{}\" after {} ms",
        cmdline,
        server_addr,
        start.elapsed().as_millis()
    );
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct ExampleProject {
    name: String,
}

/// Tests the "pagination-basic" example, which just lists 999 projects.
#[tokio::test]
async fn test_example_basic() {
    // We specify a port on which to run the example servers.  It would be
    // better to let them pick a port on startup (as they will do if we don't
    // provide an argument) and use that.
    let mut exctx = start_example("pagination-basic", 12230).await;
    let client = &exctx.client;

    let alltogether =
        assert_collection_iter::<ExampleProject>(client, "/projects", "").await;
    assert_eq!(alltogether.len(), 999);
    assert_eq!(alltogether[0].name, "project001");
    assert_eq!(alltogether[alltogether.len() - 1].name, "project999");

    exctx.cleanup_successful();
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct ExampleProjectMtime {
    name: String,
    mtime: DateTime<Utc>,
}

/// Tests the "pagination-multiple-sorts" example.
#[tokio::test]
async fn test_example_multiple_sorts() {
    let mut exctx = start_example("pagination-multiple-sorts", 12231).await;
    let client = &exctx.client;

    // default sort
    let byname =
        assert_collection_iter::<ExampleProjectMtime>(client, "/projects", "")
            .await;
    assert_eq!(byname.len(), 999);
    assert_eq!(byname[0].name, "project001");
    assert_eq!(byname[byname.len() - 1].name, "project999");

    // ascending sort by name
    let byname_asc = assert_collection_iter::<ExampleProjectMtime>(
        client,
        "/projects",
        "sort=by-name-ascending",
    )
    .await;
    assert_eq!(byname, byname_asc);

    // descending sort by name
    let byname_desc = assert_collection_iter::<ExampleProjectMtime>(
        client,
        "/projects",
        "sort=by-name-descending",
    )
    .await;
    assert_eq!(
        byname_desc,
        byname_asc.iter().rev().cloned().collect::<Vec<ExampleProjectMtime>>()
    );

    // ascending sort by mtime
    let bymtime_asc = assert_collection_iter::<ExampleProjectMtime>(
        client,
        "/projects",
        "sort=by-mtime-ascending",
    )
    .await;
    assert_ne!(bymtime_asc, byname_asc);
    assert_ne!(bymtime_asc, byname_desc);
    bymtime_asc.windows(2).for_each(|slice| {
        assert!(
            slice[0].mtime.timestamp_nanos_opt().unwrap()
                <= slice[1].mtime.timestamp_nanos_opt().unwrap()
        );
    });

    // descending sort by mtime
    let bymtime_desc = assert_collection_iter::<ExampleProjectMtime>(
        client,
        "/projects",
        "sort=by-mtime-descending",
    )
    .await;
    assert_ne!(bymtime_desc, byname_asc);
    assert_ne!(bymtime_desc, byname_desc);
    assert_ne!(bymtime_desc, bymtime_asc);
    assert_eq!(
        bymtime_desc,
        bymtime_asc.iter().rev().cloned().collect::<Vec<ExampleProjectMtime>>()
    );

    exctx.cleanup_successful();
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct ExampleObject {
    id: Uuid,
    name: String,
}

/// Tests the "pagination-multiple-resources" example.
#[tokio::test]
async fn test_example_multiple_resources() {
    let mut exctx = start_example("pagination-multiple-resources", 12232).await;
    let client = &exctx.client;

    let resources = ["/projects", "/disks", "/instances"];
    for resource in &resources[..] {
        // Scan parameters are not necessary.
        let no_args =
            objects_list_page::<ExampleObject>(client, "/projects?limit=3")
                .await;
        assert_eq!(no_args.items.len(), 3);

        let by_name_asc = assert_collection_iter::<ExampleObject>(
            client,
            resource,
            "sort=by-name-ascending",
        )
        .await;
        let by_name_desc = assert_collection_iter::<ExampleObject>(
            client,
            resource,
            "sort=by-name-descending",
        )
        .await;
        assert_eq!(
            by_name_desc,
            by_name_asc.iter().rev().cloned().collect::<Vec<ExampleObject>>()
        );
    }

    exctx.cleanup_successful();
}
