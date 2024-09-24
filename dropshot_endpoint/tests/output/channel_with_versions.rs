const _: fn() = || {
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
///API Endpoint: my_channel
struct my_channel {}
#[allow(non_upper_case_globals, missing_docs)]
///API Endpoint: my_channel
const my_channel: my_channel = my_channel {};
impl From<my_channel>
for dropshot::ApiEndpoint<
    <RequestContext<()> as dropshot::RequestContextArgument>::Context,
> {
    fn from(_: my_channel) -> Self {
        #[allow(clippy::unused_async)]
        async fn my_channel(
            _rqctx: RequestContext<()>,
            _conn: WebsocketConnection,
        ) -> WebsocketChannelResult {
            Ok(())
        }
        const _: fn() = || {
            fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
            fn check_future_bounds(
                arg0: RequestContext<()>,
                __dropshot_websocket: WebsocketConnection,
            ) {
                future_endpoint_must_be_send(my_channel(arg0, __dropshot_websocket));
            }
        };
        async fn my_channel_adapter(
            arg0: RequestContext<()>,
            __dropshot_websocket: dropshot::WebsocketUpgrade,
        ) -> dropshot::WebsocketEndpointResult {
            __dropshot_websocket
                .handle(move |__dropshot_websocket: WebsocketConnection| async move {
                    my_channel(arg0, __dropshot_websocket).await
                })
        }
        dropshot::ApiEndpoint::new(
            "my_channel".to_string(),
            my_channel_adapter,
            dropshot::Method::GET,
            "application/json",
            "/my/ws/channel",
            dropshot::ApiEndpointVersions::Until(semver::Version::new(1u64, 2u64, 3u64)),
        )
    }
}
