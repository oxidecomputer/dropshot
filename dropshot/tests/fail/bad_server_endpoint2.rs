// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use std::sync::Arc;

#[dropshot::server]
trait MyServer {
    type Context;

    #[endpoint { method = GET, path = "/test" }]
    async fn ref_self_method(
        &self,
        rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn mut_self_method(
        &mut self,
        rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn self_method(
        self,
        rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn self_box_self_method(
        self: Box<Self>,
        rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError>;

    #[endpoint { method = GET, path = "/test" }]
    async fn self_arc_self_method(
        self: Arc<Self>,
        rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError>;
}

enum MyImpl {}

// This should not produce errors about the endpoints being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn ref_self_method(
        &self,
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError> {
        todo!()
    }

    async fn mut_self_method(
        &mut self,
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError> {
        todo!()
    }

    async fn self_method(
        self,
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError> {
        todo!()
    }

    async fn self_box_self_method(
        self: Box<Self>,
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError> {
        todo!()
    }

    async fn self_arc_self_method(
        self: Arc<Self>,
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError> {
        todo!()
    }
}

fn main() {}
