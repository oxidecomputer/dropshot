// Copyright 2024 Oxide Computer Company

//! Test cases for user-defined error types.

use dropshot::ApiDescription;
use dropshot::ErrorStatusCode;
use dropshot::HttpError;
use dropshot::HttpResponseError;
use dropshot::HttpResponseOk;
use dropshot::Path;
use dropshot::RequestContext;
use dropshot::endpoint;
use dropshot::test_util::TestContext;
use http::Method;
use http::StatusCode;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::common;

// Define an enum error type.
#[derive(Debug, thiserror::Error, Serialize, JsonSchema)]
pub(crate) enum EnumError {
    // A user-defined custom error variant. This one is one of the classic
    // confusing TeX error messages.
    #[error("overfull \\hbox (badness {badness}) at line {line}")]
    OverfullHbox { badness: i32, line: u32 },
    // Variant constructed from Dropshot `HttpError`s.
    #[error("{internal_message}")]
    HttpError {
        message: String,
        error_code: Option<String>,
        #[serde(skip)]
        internal_message: String,
        #[serde(skip)]
        status: ErrorStatusCode,
    },
}

impl From<HttpError> for EnumError {
    fn from(e: HttpError) -> Self {
        EnumError::HttpError {
            message: e.external_message,
            error_code: e.error_code,
            internal_message: e.internal_message,
            status: e.status_code,
        }
    }
}

impl HttpResponseError for EnumError {
    fn status_code(&self) -> ErrorStatusCode {
        match self {
            EnumError::OverfullHbox { .. } => {
                ErrorStatusCode::INTERNAL_SERVER_ERROR
            }
            EnumError::HttpError { status, .. } => *status,
        }
    }
}

/// This is the same as `EnumError`, but with the non-serialized fields removed.
/// This should match what the client receives.
#[derive(Debug, Deserialize, Eq, PartialEq)]
enum DeserializedEnumError {
    OverfullHbox { badness: i32, line: u32 },
    HttpError { message: String, error_code: Option<String> },
}

// Also, define a struct error type wrapping an enum.
#[derive(Debug, thiserror::Error, Serialize, JsonSchema)]
#[error("{message}")]
pub(crate) struct StructError {
    message: String,
    kind: ErrorKind,
    #[serde(skip)]
    status: ErrorStatusCode,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Eq, PartialEq)]
enum ErrorKind {
    /// Can't get ye flask.
    CantGetYeFlask,
    /// Flagrant error,
    Other,
}

/// This is the same as `StructError`, but with the non-serialized fields removed.
/// This should match what the client receives.
#[derive(Debug, Deserialize, Eq, PartialEq)]
struct DeserializedStructError {
    message: String,
    kind: ErrorKind,
}

impl From<HttpError> for StructError {
    fn from(e: HttpError) -> Self {
        Self {
            message: e.external_message,
            kind: ErrorKind::Other,
            status: e.status_code,
        }
    }
}

impl HttpResponseError for StructError {
    fn status_code(&self) -> ErrorStatusCode {
        self.status
    }
}

// The test endpoints take a boolean parameter that determines whether to return
// an error or a success. This is used primarily so that we can send a request
// with a path param that *doesn't* parse as a boolean, in order to trigger an
// extractor error and exercise the conversion of dropshot `HttpError`s into the
// user error type.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct HandlerPathParam {
    should_error: bool,
}

#[endpoint {
    method = GET,
    path = "/test/enum-error/{should_error}",
}]
async fn enum_error_handler(
    _rqctx: RequestContext<()>,
    path: Path<HandlerPathParam>,
) -> Result<HttpResponseOk<()>, EnumError> {
    enum_error_inner(path)
}

#[endpoint {
    method = GET,
    path = "/test/struct-error/{should_error}",
}]
async fn struct_error_handler(
    _rqctx: RequestContext<()>,
    path: Path<HandlerPathParam>,
) -> Result<HttpResponseOk<()>, StructError> {
    struct_error_inner(path)
}

// A handler that returns a `dropshot::HttpError`. This is used by the OpenAPI
// tests for custom errors in order to ensure that handlers returning
// `HttpError` can coexist with those that return user-defined types.
#[endpoint {
    method = GET,
    path = "/test/dropshot-error/",
}]
async fn dropshot_error_handler(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<()>, HttpError> {
    Err(HttpError::for_internal_error("something bad happened".to_string()))
}

fn enum_error_inner(
    path: Path<HandlerPathParam>,
) -> Result<HttpResponseOk<()>, EnumError> {
    let HandlerPathParam { should_error } = path.into_inner();
    if should_error {
        Err(EnumError::OverfullHbox { badness: 10000, line: 42 })
    } else {
        Ok(HttpResponseOk(()))
    }
}

fn struct_error_inner(
    path: Path<HandlerPathParam>,
) -> Result<HttpResponseOk<()>, StructError> {
    let HandlerPathParam { should_error } = path.into_inner();
    if should_error {
        Err(StructError {
            kind: ErrorKind::CantGetYeFlask,
            message: "can't get ye flask".to_string(),
            status: ErrorStatusCode::NOT_FOUND,
        })
    } else {
        Ok(HttpResponseOk(()))
    }
}

pub(crate) fn api() -> ApiDescription<()> {
    let mut api = ApiDescription::new();
    api.register(enum_error_handler).unwrap();
    api.register(struct_error_handler).unwrap();
    api.register(dropshot_error_handler).unwrap();
    api
}

#[dropshot::api_description]
pub(crate) trait CustomErrorApi {
    type Context;

    #[endpoint {
        method = GET,
        path = "/test/enum-error/{should_error}",
    }]
    async fn enum_error(
        rqctx: RequestContext<Self::Context>,
        path: Path<HandlerPathParam>,
    ) -> Result<HttpResponseOk<()>, EnumError>;

    #[endpoint {
        method = GET,
        path = "/test/struct-error/{should_error}",
    }]
    async fn struct_error(
        rqctx: RequestContext<Self::Context>,
        path: Path<HandlerPathParam>,
    ) -> Result<HttpResponseOk<()>, StructError>;

    #[endpoint {
        method = GET,
        path = "/test/dropshot-error/",
    }]
    async fn dropshot_error(
        rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError>;
}

enum ApiImpl {}

impl CustomErrorApi for ApiImpl {
    type Context = ();

    async fn enum_error(
        _rqctx: RequestContext<()>,
        path: Path<HandlerPathParam>,
    ) -> Result<HttpResponseOk<()>, EnumError> {
        enum_error_inner(path)
    }

    async fn struct_error(
        _rqctx: RequestContext<()>,
        path: Path<HandlerPathParam>,
    ) -> Result<HttpResponseOk<()>, StructError> {
        struct_error_inner(path)
    }

    async fn dropshot_error(
        _rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<()>, HttpError> {
        Err(HttpError::for_internal_error("something bad happened".to_string()))
    }
}

// Test case: the enum error handler returns the user-defiend error variant.
#[tokio::test]
async fn test_enum_user_error() {
    let api = api();
    let testctx = common::test_setup_with_context(
        "test_enum_user_error",
        api,
        (),
        dropshot::HandlerTaskMode::Detached,
    );
    do_enum_user_error_test(testctx).await;
}

async fn do_enum_user_error_test(testctx: TestContext<()>) {
    let json = testctx
        .client_testctx
        .with_error_type::<DeserializedEnumError>()
        .make_request_error(
            Method::GET,
            "/test/enum-error/true",
            StatusCode::INTERNAL_SERVER_ERROR,
        )
        .await;
    assert_eq!(
        dbg!(json),
        DeserializedEnumError::OverfullHbox { badness: 10000, line: 42 }
    );

    testctx.teardown().await;
}

// Test case: the enum error handler converts a Dropshot `HttpError` from an
// extractor into its user-defined error type.
#[tokio::test]
async fn test_enum_extractor_error() {
    let api = api();
    let testctx = common::test_setup_with_context(
        "test_enum_extractor_error",
        api,
        (),
        dropshot::HandlerTaskMode::Detached,
    );
    // This time, instead of /test/enum-error/true, let's send something that
    // doesn't parse as a bool, so that the path param extractor returns an error.
    let json = testctx
        .client_testctx
        .with_error_type::<DeserializedEnumError>()
        .make_request_error(
            Method::GET,
            "/test/enum-error/asdf",
            StatusCode::BAD_REQUEST,
        )
        .await;
    assert_eq!(
        dbg!(json),
        DeserializedEnumError::HttpError {
            message:
                "bad parameter in URL path: unable to parse 'asdf' as bool"
                    .to_string(),
            error_code: None
        },
    );

    testctx.teardown().await;
}

// Test case: the struct error handler returns the user-defiend error variant.
#[tokio::test]
async fn test_struct_user_error() {
    let api = api();
    let testctx = common::test_setup_with_context(
        "test_struct_user_error",
        api,
        (),
        dropshot::HandlerTaskMode::Detached,
    );
    do_struct_user_error_test(testctx).await;
}

async fn do_struct_user_error_test(testctx: TestContext<()>) {
    let json = testctx
        .client_testctx
        .with_error_type::<DeserializedStructError>()
        .make_request_error(
            Method::GET,
            "/test/struct-error/true",
            StatusCode::NOT_FOUND,
        )
        .await;
    assert_eq!(
        dbg!(json),
        DeserializedStructError {
            message: "can't get ye flask".to_string(),
            kind: ErrorKind::CantGetYeFlask,
        }
    );

    testctx.teardown().await;
}

// Test case: the struct error handler converts a Dropshot `HttpError` from an
// extractor into its user-defined error type.
#[tokio::test]
async fn test_struct_extractor_error() {
    let api = api();
    let testctx = common::test_setup_with_context(
        "test_struct_extractor_error",
        api,
        (),
        dropshot::HandlerTaskMode::Detached,
    );
    // This time, instead of /test/struct-error/true, let's send something that
    // doesn't parse as a bool, so that the path param extractor returns an error.
    let json = testctx
        .client_testctx
        .with_error_type::<DeserializedStructError>()
        .make_request_error(
            Method::GET,
            "/test/struct-error/this-wont-work",
            StatusCode::BAD_REQUEST,
        )
        .await;
    assert_eq!(
        dbg!(json),
        DeserializedStructError {
            message:
                "bad parameter in URL path: unable to parse 'this-wont-work' as bool"
                    .to_string(),
            kind: ErrorKind::Other,
        },
    );

    testctx.teardown().await;
}

// Test cases for trait-based APIs.

#[test]
fn test_trait_based_api() {
    custom_error_api_mod::stub_api_description().unwrap();
    custom_error_api_mod::api_description::<ApiImpl>().unwrap();
}

#[tokio::test]
async fn test_trait_enum_user_error() {
    let api = custom_error_api_mod::api_description::<ApiImpl>().unwrap();
    let testctx = common::test_setup_with_context(
        "test_trait_enum_user_error",
        api,
        (),
        dropshot::HandlerTaskMode::Detached,
    );
    do_struct_user_error_test(testctx).await;
}

#[tokio::test]
async fn test_trait_struct_user_error() {
    let api = custom_error_api_mod::api_description::<ApiImpl>().unwrap();
    let testctx = common::test_setup_with_context(
        "test_trait_struct_user_error",
        api,
        (),
        dropshot::HandlerTaskMode::Detached,
    );
    do_struct_user_error_test(testctx).await;
}
