// Copyright 2023 Oxide Computer Company

use dropshot::{HttpError, HttpResponseOk, Path, RequestContext};
use schemars::JsonSchema;
use std::sync::Arc;

#[dropshot::server]
trait MyServer {
    type Context;

    #[endpoint { method = GET, path = "/test" }]
    fn non_async_method(
        rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn non_returning_method(rqctx: RequestContext<Self::Context>);

    #[endpoint { method = GET, path = "/test" }]
    async fn with_type_param<T: Send + Sync + JsonSchema + 'static>(
        rqctx: RequestContext<Self::Context>,
        path_params: Path<T>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn with_const_param<const N: usize>(
        rqctx: RequestContext<Self::Context>,
        array: [u8; N],
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn with_where_clause(
        rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError>
    where
        Self: Sized;

    #[endpoint { method = GET, path = "/test" }]
    async fn with_where_clause_2(
        rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError>
    where
        Self: std::fmt::Debug;

    #[endpoint { method = GET, path = "/test" }]
    async fn with_where_clause_3(
        rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError>
    where
        usize: std::fmt::Debug;

    /// This method has several things wrong with it; ensure that errors are
    /// generated for all of them.
    #[endpoint { method = GET, path = "/test" }]
    fn many_things_wrong(rqctx: RequestContext<Self::Context>);
}

fn main() {}
