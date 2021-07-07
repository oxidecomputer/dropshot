// Copyright 2021 Oxide Computer Company
/*!
 * Test cases for endpoints that specifically request raw request info (headers,
 * body, etc)
 */

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::HttpError;
use dropshot::RequestContext;
use hyper::Body;
use hyper::Request;
use hyper::Response;

#[endpoint {
    method = GET,
    path = "/raw_req_1",
}]
async fn demo_handler_path_param_impossible(
    _rqctx: &RequestContext<usize>,
    _raw_request: &mut Request<Body>,
) -> Result<Response<Body>, HttpError> {
    todo!()
}

#[tokio::test]
async fn test_basic_raw_request_endpoint() {
    let mut api = ApiDescription::new();
    assert!(matches!(api.register(demo_handler_path_param_impossible), Ok(_)));
}
