// Copyright 2023 Oxide Computer Company

use dropshot::{dropshot_server, HttpError, HttpResponseOk, RequestContext};
use std::sync::Arc;

#[dropshot_server]
trait MyServer {
    #[endpoint { method = GET, path = "/test" }]
    async fn static_method(
        rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn mut_self_method(
        &mut self,
        rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn self_method(
        self,
        rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn self_box_self_method(
        self: Box<Self>,
        rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn self_arc_self_method(
        self: Arc<Self>,
        rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    fn non_async_method(
        &self,
        rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn non_returning_method(&self, rqctx: RequestContext<()>);

    #[endpoint { method = GET, path = "/test" }]
    async fn with_type_param<T: Send + Sync + 'static>(
        &self,
        rqctx: RequestContext<T>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn with_const_param<const N: usize>(
        &self,
        rqctx: RequestContext<()>,
        array: [u8; N],
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn with_where_clause(
        &self,
        rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<()>, HttpError>
    where
        Self: Sized;

    #[endpoint { method = GET, path = "/test" }]
    async fn with_where_clause_2(
        &self,
        rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<()>, HttpError>
    where
        Self: std::fmt::Debug;

    #[endpoint { method = GET, path = "/test" }]
    async fn with_where_clause_3(
        &self,
        rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<()>, HttpError>
    where
        usize: std::fmt::Debug;

    /// This method has several things wrong with it; ensure that errors are
    /// generated for all of them.
    #[endpoint { method = GET, path = "/test" }]
    fn many_things_wrong(&self);
}

fn main() {}
