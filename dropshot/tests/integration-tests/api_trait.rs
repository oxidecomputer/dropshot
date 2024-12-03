// Copyright 2024 Oxide Computer Company

use dropshot::{
    test_util::read_json, EndpointTagPolicy, HandlerTaskMode, HttpError,
    HttpResponseOk, HttpResponseUpdatedNoContent, RequestContext, UntypedBody,
};
use http::{Method, StatusCode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common;

// Max request body sizes are often specified as literals, but ensure that
// constants also work.
const LARGE_REQUEST_SIZE: usize = 2048;

#[derive(Deserialize, Serialize, JsonSchema)]
struct DemoUntyped {
    pub nbytes: usize,
}

#[dropshot::api_description]
trait BasicApi {
    type Context;

    #[endpoint { method = GET, path = "/test" }]
    async fn get_test(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;

    // Test one of the custom request_body_max_bytes cases. The other cases are
    // covered in `test_demo`.
    #[endpoint {
        method = PUT,
        path = "/test/large_untyped_body",
        request_body_max_bytes = LARGE_REQUEST_SIZE,
    }]
    async fn large_untyped_body(
        _rqctx: RequestContext<Self::Context>,
        body: UntypedBody,
    ) -> Result<HttpResponseOk<DemoUntyped>, HttpError>;
}

enum BasicImpl {}

impl BasicApi for BasicImpl {
    type Context = ();

    async fn get_test(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }

    async fn large_untyped_body(
        _rqctx: RequestContext<Self::Context>,
        body: UntypedBody,
    ) -> Result<HttpResponseOk<DemoUntyped>, HttpError> {
        Ok(HttpResponseOk(DemoUntyped { nbytes: body.as_bytes().len() }))
    }
}

#[tokio::test]
async fn test_api_trait_basic() {
    basic_api_mod::stub_api_description().unwrap();

    let api = basic_api_mod::api_description::<BasicImpl>().unwrap();
    let testctx = common::test_setup_with_context(
        "api_trait_basic",
        api,
        (),
        HandlerTaskMode::Detached,
    );

    // Success case: large body endpoint.
    let large_body = vec![0u8; 2048];
    let mut response = testctx
        .client_testctx
        .make_request_with_body(
            Method::PUT,
            "/test/large_untyped_body",
            large_body.into(),
            StatusCode::OK,
        )
        .await
        .expect("expected success");
    let json: DemoUntyped = read_json(&mut response).await;
    assert_eq!(json.nbytes, 2048);

    // Error case: large body endpoint failure.
    let large_body = vec![0u8; 2049];
    let error = testctx
        .client_testctx
        .make_request_with_body(
            Method::PUT,
            "/test/large_untyped_body",
            large_body.into(),
            StatusCode::BAD_REQUEST,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "request body exceeded maximum size of 2048 bytes"
    );

    testctx.teardown().await;
}

#[dropshot::api_description {
    tag_config = {
        tags = {},
    }
}]
trait ApiWithEmptyTagConfig {
    type Context;

    #[endpoint { method = GET, path = "/test" }]
    async fn get_test(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum ImplWithEmptyTagConfig {}

impl ApiWithEmptyTagConfig for ImplWithEmptyTagConfig {
    type Context = ();

    async fn get_test(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

#[test]
fn test_api_trait_with_empty_tag_config() {
    let api_description = api_with_empty_tag_config_mod::api_description::<
        ImplWithEmptyTagConfig,
    >()
    .unwrap();
    // Ensure that the endpoint tag policy is Any.
    assert_eq!(api_description.get_tag_config().policy, EndpointTagPolicy::Any);

    let stub_description =
        api_with_empty_tag_config_mod::stub_api_description().unwrap();
    // Ensure that the endpoint tag policy is Any.
    assert_eq!(
        stub_description.get_tag_config().policy,
        EndpointTagPolicy::Any
    );
}

#[dropshot::api_description {
    tag_config = {
        // This means that tags are not allowed.
        allow_other_tags = false,
        tags = {},
    }
}]
trait ApiWithDisallowedTags {
    type Context;

    #[endpoint { method = GET, path = "/test", tags = ["foo"] }]
    async fn get_test(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum ImplWithDisallowedTags {}

impl ApiWithDisallowedTags for ImplWithDisallowedTags {
    type Context = ();

    async fn get_test(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

#[test]
fn test_api_trait_with_disallowed_tags() {
    let errors = match api_with_disallowed_tags_mod::api_description::<
        ImplWithDisallowedTags,
    >() {
        Ok(_) => panic!("expected error"),
        Err(e) => e,
    };

    assert_eq!(errors.errors().len(), 1);
    assert_eq!(errors.errors()[0].message(), "Invalid tag: foo");

    let errors = match api_with_disallowed_tags_mod::stub_api_description() {
        Ok(_) => panic!("expected error"),
        Err(e) => e,
    };

    assert_eq!(errors.errors().len(), 1);
    assert_eq!(errors.errors()[0].message(), "Invalid tag: foo");
}

#[dropshot::api_description {
    tag_config = {
        allow_other_tags = false,
        policy = EndpointTagPolicy::Any,
        tags = {
            // Test out every allowed tag configuration.
            "tag1" = {},
            "tag2" = {
                description = "tag2",
            },
            "tag3" = {
                external_docs = {
                    url = "https://example.com/tag3",
                }
            },
            "tag4" = {
                external_docs = {
                    description = "tag4",
                    url = "https://example.com/tag4",
                },
            },
            "tag5" = {
                description = "tag5",
                external_docs = {
                    description = "External docs for tag5",
                    url = "https://example.com/tag5",
                },
            },
        },
    }
}]
trait ApiWithComplexTags {
    type Context;

    #[endpoint { method = GET, path = "/test", tags = ["tag1"] }]
    async fn get_test(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;

    #[endpoint { method = GET, path = "/test2", tags = ["tag1", "tag2"] }]
    async fn get_test2(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;

    #[endpoint {
        method = GET,
        path = "/test3",
        tags = ["tag1", "tag2", "tag3", "tag4", "tag5"],
    }]
    async fn get_test3(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum ImplWithComplexTags {}

impl ApiWithComplexTags for ImplWithComplexTags {
    type Context = ();

    async fn get_test(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }

    async fn get_test2(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }

    async fn get_test3(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

#[test]
fn test_api_trait_with_complex_tags() {
    api_with_complex_tags_mod::api_description::<ImplWithComplexTags>()
        .unwrap();
    api_with_complex_tags_mod::stub_api_description().unwrap();
}
