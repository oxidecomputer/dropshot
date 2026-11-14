// Copyright 2026 Oxide Computer Company

//! Cloned API descriptions can serve independent context values.

use dropshot::ApiDescription;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use dropshot::ServerBuilder;
use dropshot::endpoint;
use dropshot::test_util::ClientTestContext;
use dropshot::test_util::read_json;
use http::Method;
use http::StatusCode;

struct NonCloneContext {
    name: String,
}

#[endpoint {
    method = GET,
    path = "/context",
}]
async fn context(
    rqctx: RequestContext<NonCloneContext>,
) -> Result<HttpResponseOk<String>, HttpError> {
    Ok(HttpResponseOk(rqctx.context().name.clone()))
}

#[tokio::test]
async fn test_cloned_api_distinct_contexts() {
    let logctx =
        crate::common::create_log_context("test_cloned_api_distinct_contexts");
    let mut api = ApiDescription::new();
    api.register(context).unwrap();
    let first = ServerBuilder::new(
        api.clone(),
        NonCloneContext { name: "first".to_string() },
        logctx.log.clone(),
    )
    .start()
    .unwrap();
    let second = ServerBuilder::new(
        api,
        NonCloneContext { name: "second".to_string() },
        logctx.log.clone(),
    )
    .start()
    .unwrap();

    for (server, expected) in [(&first, "first"), (&second, "second")] {
        let client =
            ClientTestContext::new(server.local_addr(), logctx.log.clone());
        let mut response = client
            .make_request_no_body(Method::GET, "/context", StatusCode::OK)
            .await
            .unwrap();
        let name: String = read_json(&mut response).await;
        assert_eq!(name, expected);
    }
    first.close().await.unwrap();
    second.close().await.unwrap();
    logctx.cleanup_successful();
}
