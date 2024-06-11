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
    need_exclusive_extractor::<dropshot::WebsocketUpgrade>();
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
    struct NeedHttpResponse(<dropshot::WebsocketEndpointResult as ResultTrait>::T);
    trait TypeEq {
        type This: ?Sized;
    }
    impl<T: ?Sized> TypeEq for T {
        type This = Self;
    }
    fn validate_result_error_type<T>()
    where
        T: ?Sized + TypeEq<This = dropshot::HttpError>,
    {}
    validate_result_error_type::<
        <dropshot::WebsocketEndpointResult as ResultTrait>::E,
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
            __dropshot_websocket_upgrade: dropshot::WebsocketUpgrade,
        ) -> dropshot::WebsocketEndpointResult {
            async fn __dropshot_websocket_handler(
                _: RequestContext<()>,
                _: Query<Q>,
                _: Path<P>,
                conn: WebsocketConnection,
            ) -> WebsocketChannelResult {
                Ok(())
            }
            __dropshot_websocket_upgrade
                .handle(move |conn: WebsocketConnection| async move {
                    __dropshot_websocket_handler(_, _, _, conn).await
                })
        }
        const _: fn() = || {
            fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
            fn check_future_bounds(
                arg0: RequestContext<()>,
                arg1: Query<Q>,
                arg2: Path<P>,
                arg3: dropshot::WebsocketUpgrade,
            ) {
                future_endpoint_must_be_send(handler_xyz(arg0, arg1, arg2, arg3));
            }
        };
        dropshot::ApiEndpoint::new(
            "handler_xyz".to_string(),
            handler_xyz,
            dropshot::Method::GET,
            "application/json",
            "/my/ws/channel",
        )
    }
}
