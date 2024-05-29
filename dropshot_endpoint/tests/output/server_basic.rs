pub trait MyTrait: 'static {
    type Context: dropshot::ServerContext + 'static;
    fn handler_xyz(
        rqctx: RequestContext<Self::Context>,
    ) -> impl ::core::future::Future<
        Output = Result<HttpResponseOk<()>, HttpError>,
    > + Send + 'static;
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
