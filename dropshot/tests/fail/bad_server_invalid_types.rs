// Copyright 2023 Oxide Computer Company

use dropshot::{dropshot_server, HttpError, HttpResponseOk, RequestContext};

#[dropshot_server]
trait MyServer: Send + Sync + 'static {
    #[endpoint { method = GET, path = "/test" }]
    async fn non_result_method(
        &self,
        rqctx: RequestContext<()>,
    ) -> HttpResponseOk<()>;

    #[endpoint { method = GET, path = "/test" }]
    async fn non_http_error_method(
        &self,
        rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<()>, ()>;

    #[endpoint { method = GET, path = "/test" }]
    async fn non_http_error_result_unit_method(
        &self,
        rqctx: RequestContext<()>,
    ) -> Result<(), HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn non_http_error_result_unit_unit_method(
        &self,
        rqctx: RequestContext<()>,
    ) -> Result<(), ()>;
}

fn main() {}
