const _: fn() = || {
    struct NeedRequestContext(
        <MyRequestContext as dropshot::RequestContextArgument>::Context,
    );
};
const _: fn() = || {
    fn need_shared_extractor<T>()
    where
        T: ?Sized + dropshot::SharedExtractor,
    {}
    need_shared_extractor::<Query<QueryParams<'static>>>();
};
const _: fn() = || {
    fn need_exclusive_extractor<T>()
    where
        T: ?Sized + dropshot::ExclusiveExtractor,
    {}
    need_exclusive_extractor::<Path<<X as Y>::Z>>();
};
const _: fn() = || {
    fn validate_response_type<T>()
    where
        T: dropshot::HttpResponse,
    {}
    validate_response_type::<Result<HttpResponseUpdatedNoContent, HttpError>>();
};
#[allow(non_camel_case_types, missing_docs)]
/**API Endpoint: handler_xyz
handle "xyz" requests*/
struct handler_xyz {}
#[allow(non_upper_case_globals, missing_docs)]
/**API Endpoint: handler_xyz
handle "xyz" requests*/
const handler_xyz: handler_xyz = handler_xyz {};
impl From<handler_xyz>
for dropshot::ApiEndpoint<
    <MyRequestContext as dropshot::RequestContextArgument>::Context,
> {
    fn from(_: handler_xyz) -> Self {
        #[allow(clippy::unused_async)]
        /// handle "xyz" requests
        async fn handler_xyz(
            _rqctx: MyRequestContext,
            query: Query<QueryParams<'static>>,
            path: Path<<X as Y>::Z>,
        ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
            Ok(())
        }
        const _: fn() = || {
            fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
            fn check_future_bounds(
                arg0: MyRequestContext,
                arg1: Query<QueryParams<'static>>,
                arg2: Path<<X as Y>::Z>,
            ) {
                future_endpoint_must_be_send(handler_xyz(arg0, arg1, arg2));
            }
        };
        dropshot::ApiEndpoint::new(
                "handler_xyz".to_string(),
                handler_xyz,
                dropshot::Method::GET,
                "application/json",
                "/a/b/c",
            )
            .summary("handle \"xyz\" requests")
    }
}
