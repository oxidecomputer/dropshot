// Copyright 2023 Oxide Computer Company

use dropshot::{dropshot_server, HttpError, HttpResponseOk, RequestContext};

#[dropshot_server]
trait MyServer: Send + Sync + Sized + 'static {
    #[endpoint { method = GET, path = "/test" }]
    async fn non_result_method(
        rqctx: RequestContext<Self>,
    ) -> HttpResponseOk<()>;

    #[endpoint { method = GET, path = "/test" }]
    async fn non_http_error_method(
        rqctx: RequestContext<Self>,
    ) -> Result<HttpResponseOk<()>, ()>;

    #[endpoint { method = GET, path = "/test" }]
    async fn non_http_error_result_unit_method(
        rqctx: RequestContext<Self>,
    ) -> Result<(), HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn non_http_error_result_unit_unit_method(
        rqctx: RequestContext<Self>,
    ) -> Result<(), ()>;
}

fn main() {}
