// Copyright 2023 Oxide Computer Company

use dropshot::{HttpError, HttpResponseUpdatedNoContent, RequestContext};

#[dropshot::api_description]
trait BasicApi {
    type Context;

    #[endpoint { method = GET, path = "/test" }]
    async fn get_test(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum BasicImpl {}

impl BasicApi for BasicImpl {
    type Context = ();

    async fn get_test(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

#[test]
fn test_api_trait_basic() {
    basic_api::api_description::<BasicImpl>().unwrap();
    basic_api::stub_api_description().unwrap();
}

#[dropshot::api_description {
    tag_config = {
        tag_definitions = {},
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
    api_with_empty_tag_config::api_description::<ImplWithEmptyTagConfig>()
        .unwrap();
    api_with_empty_tag_config::stub_api_description().unwrap();
}

#[dropshot::api_description {
    tag_config = {
        // This means that tags are not allowed.
        allow_other_tags = false,
        tag_definitions = {},
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
    let errors = match api_with_disallowed_tags::api_description::<
        ImplWithDisallowedTags,
    >() {
        Ok(_) => panic!("expected error"),
        Err(e) => e,
    };

    assert_eq!(errors.errors().len(), 1);
    assert_eq!(errors.errors()[0].message(), "Invalid tag: foo");

    let errors = match api_with_disallowed_tags::stub_api_description() {
        Ok(_) => panic!("expected error"),
        Err(e) => e,
    };

    assert_eq!(errors.errors().len(), 1);
    assert_eq!(errors.errors()[0].message(), "Invalid tag: foo");
}

#[dropshot::api_description {
    tag_config = {
        allow_other_tags = false,
        endpoint_tag_policy = at_least_one,
        tag_definitions = {
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
    api_with_complex_tags::api_description::<ImplWithComplexTags>().unwrap();
    api_with_complex_tags::stub_api_description().unwrap();
}
