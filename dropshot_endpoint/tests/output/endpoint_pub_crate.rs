const _: fn() = || {
    struct NeedRequestContext(
        <RequestContext<()> as dropshot::RequestContextArgument>::Context,
    );
};
const _: fn() = || {
    fn need_exclusive_extractor<T>()
    where
        T: ?Sized + dropshot::ExclusiveExtractor,
    {}
    need_exclusive_extractor::<Query<Q>>();
};
const _: fn() = || {
    fn validate_response_type<T>()
    where
        T: dropshot::HttpResponse,
    {}
    validate_response_type::<Result<HttpResponseOk<()>, HttpError>>();
};
#[allow(non_camel_case_types, missing_docs)]
///API Endpoint: handler_xyz
pub(crate) struct handler_xyz {}
#[allow(non_upper_case_globals, missing_docs)]
///API Endpoint: handler_xyz
pub(crate) const handler_xyz: handler_xyz = handler_xyz {};
impl From<handler_xyz>
for dropshot::ApiEndpoint<
    <RequestContext<()> as dropshot::RequestContextArgument>::Context,
> {
    fn from(_: handler_xyz) -> Self {
        #[allow(clippy::unused_async)]
        pub(crate) async fn handler_xyz(
            _rqctx: RequestContext<()>,
            q: Query<Q>,
        ) -> Result<HttpResponseOk<()>, HttpError> {
            Ok(())
        }
        const _: fn() = || {
            fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
            fn check_future_bounds(arg0: RequestContext<()>, arg1: Query<Q>) {
                future_endpoint_must_be_send(handler_xyz(arg0, arg1));
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
