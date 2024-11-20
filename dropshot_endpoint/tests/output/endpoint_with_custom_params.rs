const _: fn() = || {
    struct NeedRequestContext(
        <RequestContext<()> as topspin::RequestContextArgument>::Context,
    );
};
const _: fn() = || {
    fn need_shared_extractor<T>()
    where
        T: ?Sized + topspin::SharedExtractor,
    {}
    need_shared_extractor::<Query<Q>>();
};
const _: fn() = || {
    fn need_exclusive_extractor<T>()
    where
        T: ?Sized + topspin::ExclusiveExtractor,
    {}
    need_exclusive_extractor::<Path<P>>();
};
const _: fn() = || {
    fn validate_response_type<T>()
    where
        T: topspin::HttpResponse,
    {}
    validate_response_type::<Result<HttpResponseOk<()>, HttpError>>();
};
#[allow(non_camel_case_types, missing_docs)]
///API Endpoint: handler_xyz
struct handler_xyz {}
#[allow(non_upper_case_globals, missing_docs)]
///API Endpoint: handler_xyz
const handler_xyz: handler_xyz = handler_xyz {};
impl From<handler_xyz>
for topspin::ApiEndpoint<
    <RequestContext<()> as topspin::RequestContextArgument>::Context,
> {
    fn from(_: handler_xyz) -> Self {
        #[allow(clippy::unused_async)]
        async fn handler_xyz(
            _rqctx: RequestContext<()>,
            query: Query<Q>,
            path: Path<P>,
        ) -> Result<HttpResponseOk<()>, HttpError> {
            Ok(())
        }
        const _: fn() = || {
            fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
            fn check_future_bounds(
                arg0: RequestContext<()>,
                arg1: Query<Q>,
                arg2: Path<P>,
            ) {
                future_endpoint_must_be_send(handler_xyz(arg0, arg1, arg2));
            }
        };
        topspin::ApiEndpoint::new(
            "handler_xyz".to_string(),
            handler_xyz,
            topspin::Method::GET,
            "application/json",
            "/a/b/c",
        )
    }
}
