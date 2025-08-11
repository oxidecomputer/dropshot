const _: fn() = || {
    #[allow(dead_code)]
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
    trait TypeEq {
        type This: ?Sized;
    }
    impl<T: ?Sized> TypeEq for T {
        type This = Self;
    }
    fn validate_websocket_connection_type<T>()
    where
        T: ?Sized + TypeEq<This = dropshot::WebsocketConnection>,
    {}
    validate_websocket_connection_type::<WebsocketConnection>();
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
            _: WebsocketConnection,
        ) -> WebsocketChannelResult {
            Ok(())
        }
        const _: fn() = || {
            fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
            fn check_future_bounds(
                arg0: RequestContext<()>,
                arg1: Query<Q>,
                arg2: Path<P>,
                __dropshot_websocket: WebsocketConnection,
            ) {
                future_endpoint_must_be_send(
                    handler_xyz(arg0, arg1, arg2, __dropshot_websocket),
                );
            }
        };
        async fn handler_xyz_adapter(
            arg0: RequestContext<()>,
            arg1: Query<Q>,
            arg2: Path<P>,
            __dropshot_websocket: dropshot::WebsocketUpgrade,
        ) -> dropshot::WebsocketEndpointResult {
            __dropshot_websocket
                .handle(move |__dropshot_websocket: WebsocketConnection| async move {
                    handler_xyz(arg0, arg1, arg2, __dropshot_websocket).await
                })
        }
        dropshot::ApiEndpoint::new(
            "handler_xyz".to_string(),
            handler_xyz_adapter,
            dropshot::Method::GET,
            "application/json",
            "/my/ws/channel",
            dropshot::ApiEndpointVersions::All,
        )
    }
}
