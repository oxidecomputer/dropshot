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
    need_exclusive_extractor::<topspin::WebsocketUpgrade>();
};
const _: fn() = || {
    trait ResultTrait {
        type T;
        type E;
    }
    impl<TT, EE> ResultTrait for Result<TT, EE>
    where
        TT: topspin::HttpResponse,
    {
        type T = TT;
        type E = EE;
    }
    struct NeedHttpResponse(<topspin::WebsocketEndpointResult as ResultTrait>::T);
    trait TypeEq {
        type This: ?Sized;
    }
    impl<T: ?Sized> TypeEq for T {
        type This = Self;
    }
    fn validate_result_error_type<T>()
    where
        T: ?Sized + TypeEq<This = topspin::HttpError>,
    {}
    validate_result_error_type::<<topspin::WebsocketEndpointResult as ResultTrait>::E>();
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
            __dropshot_websocket_upgrade: topspin::WebsocketUpgrade,
        ) -> topspin::WebsocketEndpointResult {
            async fn __dropshot_websocket_handler(
                rqctx: RequestContext<()>,
                query: Query<Q>,
                conn: WebsocketConnection,
            ) -> WebsocketChannelResult {
                Ok(())
            }
            __dropshot_websocket_upgrade
                .handle(move |conn: WebsocketConnection| async move {
                    __dropshot_websocket_handler(rqctx, query, conn).await
                })
        }
        const _: fn() = || {
            fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
            fn check_future_bounds(
                arg0: RequestContext<()>,
                arg1: Query<Q>,
                arg2: topspin::WebsocketUpgrade,
            ) {
                future_endpoint_must_be_send(my_channel(arg0, arg1, arg2));
            }
        };
        topspin::ApiEndpoint::new(
            "my_channel".to_string(),
            my_channel,
            topspin::Method::GET,
            "application/json",
            "/my/ws/channel",
        )
    }
}
