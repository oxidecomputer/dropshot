const _: fn() = || {
    #[allow(dead_code)]
    struct NeedRequestContext(
        <RequestContext<()> as dropshot::RequestContextArgument>::Context,
    );
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
            _rqctx: RequestContext<()>,
            _ws: WebsocketConnection,
        ) -> Result<HttpResponseOk<()>, HttpError> {
            Ok(())
        }
        const _: fn() = || {
            fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
            fn check_future_bounds(
                arg0: RequestContext<()>,
                __dropshot_websocket: WebsocketConnection,
            ) {
                future_endpoint_must_be_send(handler_xyz(arg0, __dropshot_websocket));
            }
        };
        async fn handler_xyz_adapter(
            arg0: RequestContext<()>,
            __dropshot_websocket: dropshot::WebsocketUpgrade,
        ) -> dropshot::WebsocketEndpointResult {
            __dropshot_websocket
                .handle(move |__dropshot_websocket: WebsocketConnection| async move {
                    handler_xyz(arg0, __dropshot_websocket).await
                })
        }
        dropshot::ApiEndpoint::new(
            "vzeroupper".to_string(),
            handler_xyz_adapter,
            dropshot::Method::GET,
            "application/json",
            "/my/ws/channel",
            dropshot::ApiEndpointVersions::All,
        )
    }
}
