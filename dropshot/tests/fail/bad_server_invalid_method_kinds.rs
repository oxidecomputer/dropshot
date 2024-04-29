// Copyright 2023 Oxide Computer Company

use dropshot::{dropshot_server, HttpError, HttpResponseOk, RequestContext};
use std::sync::Arc;

#[dropshot_server]
trait MyServer {
    #[endpoint { method = GET, path = "/test" }]
    async fn ref_self_method(
        &self,
        rqctx: RequestContext<Self>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn mut_self_method(
        &mut self,
        rqctx: RequestContext<Self>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn self_method(
        self,
        rqctx: RequestContext<Self>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn self_box_self_method(
        self: Box<Self>,
        rqctx: RequestContext<Self>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn self_arc_self_method(
        self: Arc<Self>,
        rqctx: RequestContext<Self>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    fn non_async_method(
        rqctx: RequestContext<Self>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn non_returning_method(rqctx: RequestContext<Self>);

    #[endpoint { method = GET, path = "/test" }]
    async fn with_type_param<T: Send + Sync + 'static>(
        rqctx: RequestContext<T>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn with_const_param<const N: usize>(
        rqctx: RequestContext<Self>,
        array: [u8; N],
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn with_where_clause(
        rqctx: RequestContext<Self>,
    ) -> Result<HttpResponseOk<()>, HttpError>
    where
        Self: Sized;

    #[endpoint { method = GET, path = "/test" }]
    async fn with_where_clause_2(
        rqctx: RequestContext<Self>,
    ) -> Result<HttpResponseOk<()>, HttpError>
    where
        Self: std::fmt::Debug;

    #[endpoint { method = GET, path = "/test" }]
    async fn with_where_clause_3(
        rqctx: RequestContext<Self>,
    ) -> Result<HttpResponseOk<()>, HttpError>
    where
        usize: std::fmt::Debug;

    /// This method has several things wrong with it; ensure that errors are
    /// generated for all of them.
    #[endpoint { method = GET, path = "/test" }]
    fn many_things_wrong(rqctx: RequestContext<Self>);
}

fn main() {}
