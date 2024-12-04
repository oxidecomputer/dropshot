const _: fn() = || {
    struct NeedRequestContext(
        <RequestContext<()> as dropshot::RequestContextArgument>::Context,
    );
};
const _: fn() = || {
    fn need_shared_extractor<T>()
    where
        T: ?Sized + dropshot::SharedExtractor,
    {}
    need_shared_extractor::<Query<Q>>();
};
const _: fn() = || {
    fn need_shared_extractor<T>()
    where
        T: ?Sized + dropshot::SharedExtractor,
    {}
    need_shared_extractor::<Path<P>>();
};
const _: fn() = || {
    fn need_exclusive_extractor<T>()
    where
        T: ?Sized + dropshot::ExclusiveExtractor,
    {}
    need_exclusive_extractor::<TypedBody<T>>();
};
const _: fn() = || {
    trait ResultTrait {
        type T;
        type E;
    }
    impl<TT, EE> ResultTrait for Result<TT, EE> {
        type T = TT;
        type E = EE;
    }
    fn validate_response_type<T>()
    where
        T: dropshot::HttpResponse,
    {}
    fn validate_error_type<T>()
    where
        T: dropshot::HttpResponseError,
    {}
    validate_response_type::<
        <Result<HttpResponseUpdatedNoContent, HttpError> as ResultTrait>::T,
    >();
    validate_error_type::<
        <Result<HttpResponseUpdatedNoContent, HttpError> as ResultTrait>::E,
    >();
};
#[allow(non_camel_case_types, missing_docs)]
///API Endpoint: handler_xyz
struct handler_xyz {}
#[allow(non_upper_case_globals, missing_docs)]
///API Endpoint: handler_xyz
const handler_xyz: handler_xyz = handler_xyz {};
impl From<handler_xyz>
for dropshot::ApiEndpoint<
    <RequestContext<()> as dropshot::RequestContextArgument>::Context,
> {
    fn from(_: handler_xyz) -> Self {
        #[allow(clippy::unused_async)]
        async fn handler_xyz(
            _: RequestContext<()>,
            _: Query<Q>,
            _: Path<P>,
            _: TypedBody<T>,
        ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
            Ok(())
        }
        const _: fn() = || {
            fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
            fn check_future_bounds(
                arg0: RequestContext<()>,
                arg1: Query<Q>,
                arg2: Path<P>,
                arg3: TypedBody<T>,
            ) {
                future_endpoint_must_be_send(handler_xyz(arg0, arg1, arg2, arg3));
            }
        };
        dropshot::ApiEndpoint::new(
            "handler_xyz".to_string(),
            handler_xyz,
            dropshot::Method::GET,
            "application/json",
            "/test",
            dropshot::ApiEndpointVersions::All,
        )
    }
}
