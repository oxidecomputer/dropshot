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
    trait TypeEq {
        type This: ?Sized;
    }
    impl<T: ?Sized> TypeEq for T {
        type This = Self;
    }
    fn validate_websocket_connection_type<T>()
    where
        T: ?Sized + TypeEq<This = topspin::WebsocketConnection>,
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
for topspin::ApiEndpoint<
    <RequestContext<()> as topspin::RequestContextArgument>::Context,
> {
    fn from(_: my_channel) -> Self {
        #[allow(clippy::unused_async)]
        async fn my_channel(
            rqctx: RequestContext<()>,
            query: Query<Q>,
            conn: WebsocketConnection,
        ) -> WebsocketChannelResult {
            Ok(())
        }
        const _: fn() = || {
            fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
            fn check_future_bounds(
                arg0: RequestContext<()>,
                arg1: Query<Q>,
                __dropshot_websocket: WebsocketConnection,
            ) {
                future_endpoint_must_be_send(
                    my_channel(arg0, arg1, __dropshot_websocket),
                );
            }
        };
        async fn my_channel_adapter(
            arg0: RequestContext<()>,
            arg1: Query<Q>,
            __dropshot_websocket: topspin::WebsocketUpgrade,
        ) -> topspin::WebsocketEndpointResult {
            __dropshot_websocket
                .handle(move |__dropshot_websocket: WebsocketConnection| async move {
                    my_channel(arg0, arg1, __dropshot_websocket).await
                })
        }
        topspin::ApiEndpoint::new(
            "my_channel".to_string(),
            my_channel_adapter,
            topspin::Method::GET,
            "application/json",
            "/my/ws/channel",
            topspin::ApiEndpointVersions::All,
        )
    }
}
