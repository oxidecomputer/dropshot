// Copyright 2021 Oxide Computer Company
/*!
 * Test cases for endpoints that specifically request raw request info (headers,
 * body, etc)
 */

 use dropshot::endpoint;
 use dropshot::test_util::read_json;
 use dropshot::test_util::read_string;
 use dropshot::ApiDescription;
 use dropshot::HttpError;
 use dropshot::HttpResponseOk;
 use dropshot::Path;
 use dropshot::Query;
 use dropshot::RequestContext;
 use dropshot::TypedBody;
 use dropshot::UntypedBody;
 use dropshot::CONTENT_TYPE_JSON;
 use http::StatusCode;
 use hyper::Body;
 use hyper::Request;
 use hyper::Response;
 use schemars::JsonSchema;
 use serde::Deserialize;
 use serde::Serialize;
 use uuid::Uuid;

//  #[endpoint {
//     method = GET,
//     path = "/raw_req_1",
// }]
// async fn demo_handler_path_param_impossible(
//     _rqctx: &RequestContext<usize>,
//     raw_request: &mut Request<Body>,
// ) -> Result<Response<Body>, HttpError> {
//     todo!()
// }

// #[tokio::test]
// async fn test_basic_raw_request_endpoint() {

// }

