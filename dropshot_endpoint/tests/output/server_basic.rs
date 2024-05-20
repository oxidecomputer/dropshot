pub trait MyTrait: 'static {
    type Context: dropshot::ServerContext + 'static;
    fn handler_xyz(
        rqctx: RequestContext<Self::Context>,
    ) -> impl ::core::future::Future<
        Output = Result<HttpResponseOk<()>, HttpError>,
    > + Send + 'static;
}
const _: fn() = || {
    struct NeedRequestContext(
        <RequestContext<()> as dropshot::RequestContextArgument>::Context,
    );
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
    struct NeedHttpResponse(<Result<HttpResponseOk<()>, HttpError> as ResultTrait>::T);
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
        <Result<HttpResponseOk<()>, HttpError> as ResultTrait>::E,
    >();
};
#[automatically_derived]
pub fn MyTrait_api_description<ServerImpl: MyTrait>() -> ::std::result::Result<
    dropshot::ApiDescription<<ServerImpl as MyTrait>::Context>,
    dropshot::ApiDescriptionBuildError,
> {
    let mut dropshot_api = dropshot::ApiDescription::new();
    let mut dropshot_errors: Vec<String> = Vec::new();
    {
        let endpoint_handler_xyz = dropshot::ApiEndpoint::new(
            "handler_xyz".to_string(),
            <ServerImpl as MyTrait>::handler_xyz,
            dropshot::Method::GET,
            "application/json",
            "/xyz",
        );
        if let Err(error) = dropshot_api.register(endpoint_handler_xyz) {
            dropshot_errors.push(error);
        }
    }
    if !dropshot_errors.is_empty() {
        Err(dropshot::ApiDescriptionBuildError::new(dropshot_errors))
    } else {
        Ok(dropshot_api)
    }
}
#[automatically_derived]
pub fn MyTrait_stub_api_description() -> ::std::result::Result<
    dropshot::ApiDescription<dropshot::StubContext>,
    dropshot::ApiDescriptionBuildError,
> {
    let mut dropshot_api = dropshot::ApiDescription::new();
    let mut dropshot_errors: Vec<String> = Vec::new();
    {
        let endpoint_handler_xyz = dropshot::ApiEndpoint::new_stub::<
            (),
            Result<HttpResponseOk<()>, HttpError>,
        >("handler_xyz".to_string(), dropshot::Method::GET, "application/json", "/xyz");
        if let Err(error) = dropshot_api.register(endpoint_handler_xyz) {
            dropshot_errors.push(error);
        }
    }
    if !dropshot_errors.is_empty() {
        Err(dropshot::ApiDescriptionBuildError::new(dropshot_errors))
    } else {
        Ok(dropshot_api)
    }
}
