const _: fn() = || {
    struct NeedRequestContext(
        <dropshot::RequestContext<()> as dropshot::RequestContextArgument>::Context,
    );
};
const _: fn() = || {
    trait ResultTrait {
        type T;
        type E;
    }
    impl<TT, EE> ResultTrait for Result<TT, EE>
    where
        TT: dropshot::HttpResponse,
    {
        type T = TT;
        type E = EE;
    }
    struct NeedHttpResponse(
        <std::Result<
            dropshot::HttpResponseOk<()>,
            dropshot::HttpError,
        > as ResultTrait>::T,
    );
    fn validate_result_error_type<T>()
    where
        T: dropshot::error::IntoErrorResponse + std::fmt::Display + Send + 'static,
    {}
    validate_result_error_type::<
        <std::Result<
            dropshot::HttpResponseOk<()>,
            dropshot::HttpError,
        > as ResultTrait>::E,
    >();
};
#[allow(non_camel_case_types, missing_docs)]
///API Endpoint: handler_xyz
pub struct handler_xyz {}
#[allow(non_upper_case_globals, missing_docs)]
///API Endpoint: handler_xyz
pub const handler_xyz: handler_xyz = handler_xyz {};
impl From<handler_xyz>
for dropshot::ApiEndpoint<
    <dropshot::RequestContext<()> as dropshot::RequestContextArgument>::Context,
> {
    fn from(_: handler_xyz) -> Self {
        #[allow(clippy::unused_async)]
        pub async fn handler_xyz(
            _rqctx: dropshot::RequestContext<()>,
        ) -> std::Result<dropshot::HttpResponseOk<()>, dropshot::HttpError> {
            Ok(())
        }
        const _: fn() = || {
            fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
            fn check_future_bounds(arg0: dropshot::RequestContext<()>) {
                future_endpoint_must_be_send(handler_xyz(arg0));
            }
        };
        dropshot::ApiEndpoint::new(
            "handler_xyz".to_string(),
            handler_xyz,
            dropshot::Method::GET,
            "application/json",
            "/a/b/c",
        )
    }
}
