pub trait MyTrait: 'static {
    type Context: dropshot::ServerContext;
    fn handler_xyz(
        rqctx: RequestContext<Self::Context>,
    ) -> impl ::core::future::Future<
        Output = Result<HttpResponseOk<()>, HttpError>,
    > + Send + 'static;
    fn handler_ws(
        rqctx: RequestContext<Self::Context>,
        upgraded: WebsocketConnection,
    ) -> impl ::core::future::Future<Output = WebsocketChannelResult> + Send + 'static;
}
/// Support module for the Dropshot API trait [`MyTrait`](MyTrait).
#[automatically_derived]
pub mod my_trait_mod {
    use super::*;
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
    /// Generate a _stub_ API description for [`MyTrait`], meant for OpenAPI
    /// generation.
    ///
    /// Unlike [`api_description`], this function does not require an implementation
    /// of [`MyTrait`] to be available, instead generating handlers that panic.
    /// The return value is of type [`ApiDescription`]`<`[`StubContext`]`>`.
    ///
    /// The main use of this function is in cases where [`MyTrait`] is defined
    /// in a separate crate from its implementation. The OpenAPI spec can then be
    /// generated directly from the stub API description.
    ///
    /// ## Example
    ///
    /// A function that prints the OpenAPI spec to standard output:
    ///
    /// ```rust,ignore
    /// fn print_openapi_spec() {
    ///     let stub = my_trait_mod::stub_api_description().unwrap();
    ///
    ///     // Generate OpenAPI spec from `stub`.
    ///     let spec = stub.openapi("MyTrait", "0.1.0");
    ///     spec.write(&mut std::io::stdout()).unwrap();
    /// }
    /// ```
    ///
    /// [`MyTrait`]: MyTrait
    /// [`api_description`]: my_trait_mod::api_description
    /// [`ApiDescription`]: dropshot::ApiDescription
    /// [`StubContext`]: dropshot::StubContext
    #[automatically_derived]
    pub fn stub_api_description() -> ::std::result::Result<
        dropshot::ApiDescription<dropshot::StubContext>,
        dropshot::ApiDescriptionBuildErrors,
    > {
        let mut dropshot_api = dropshot::ApiDescription::new();
        let mut dropshot_errors: Vec<dropshot::ApiDescriptionRegisterError> = Vec::new();
        {
            let endpoint_handler_xyz = dropshot::ApiEndpoint::new_for_types::<
                (),
                Result<HttpResponseOk<()>, HttpError>,
            >(
                "vzerolower".to_string(),
                dropshot::Method::GET,
                "application/json",
                "/xyz",
                dropshot::ApiEndpointVersions::All,
            );
            if let Err(error) = dropshot_api.register(endpoint_handler_xyz) {
                dropshot_errors.push(error);
            }
        }
        {
            let endpoint_handler_ws = dropshot::ApiEndpoint::new_for_types::<
                (dropshot::WebsocketUpgrade,),
                dropshot::WebsocketEndpointResult,
            >(
                "vzeroupper".to_string(),
                dropshot::Method::GET,
                "application/json",
                "/ws",
                dropshot::ApiEndpointVersions::All,
            );
            if let Err(error) = dropshot_api.register(endpoint_handler_ws) {
                dropshot_errors.push(error);
            }
        }
        if !dropshot_errors.is_empty() {
            Err(dropshot::ApiDescriptionBuildErrors::new(dropshot_errors))
        } else {
            Ok(dropshot_api)
        }
    }
    /// Given an implementation of [`MyTrait`], generate an API description.
    ///
    /// This function accepts a single type argument `ServerImpl`, turning it into a
    /// Dropshot [`ApiDescription`]`<ServerImpl::`[`Context`]`>`.
    /// The returned `ApiDescription` can then be turned into a Dropshot server that
    /// accepts a concrete `Context`.
    ///
    /// ## Example
    ///
    /// ```rust,ignore
    /// /// A type used to define the concrete implementation for `MyTrait`.
    /// ///
    /// /// This type is never constructed -- it is just a place to define your
    /// /// implementation of `MyTrait`.
    /// enum MyTraitImpl {}
    ///
    /// impl MyTrait for MyTraitImpl {
    ///     type Context = /* context type */;
    ///
    ///     // ... trait methods
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     // Generate the description for `MyTraitImpl`.
    ///     let description = my_trait_mod::api_description::<MyTraitImpl>().unwrap();
    ///
    ///     // Create a value of the concrete context type.
    ///     let context = /* some value of type `MyTraitImpl::Context` */;
    ///
    ///     // Create a Dropshot server from the description.
    ///     let log = /* ... */;
    ///     let server = dropshot::ServerBuilder::new(description, context, log)
    ///         .start()
    ///         .unwrap();
    ///
    ///     // Run the server.
    ///     server.await
    /// }
    /// ```
    ///
    /// [`ApiDescription`]: dropshot::ApiDescription
    /// [`MyTrait`]: MyTrait
    /// [`Context`]: MyTrait::Context
    #[automatically_derived]
    pub fn api_description<ServerImpl: MyTrait>() -> ::std::result::Result<
        dropshot::ApiDescription<<ServerImpl as MyTrait>::Context>,
        dropshot::ApiDescriptionBuildErrors,
    > {
        let mut dropshot_api = dropshot::ApiDescription::new();
        let mut dropshot_errors: Vec<dropshot::ApiDescriptionRegisterError> = Vec::new();
        {
            let endpoint_handler_xyz = dropshot::ApiEndpoint::new(
                "vzerolower".to_string(),
                <ServerImpl as MyTrait>::handler_xyz,
                dropshot::Method::GET,
                "application/json",
                "/xyz",
                dropshot::ApiEndpointVersions::All,
            );
            if let Err(error) = dropshot_api.register(endpoint_handler_xyz) {
                dropshot_errors.push(error);
            }
        }
        {
            async fn handler_ws_adapter<ServerImpl: MyTrait>(
                arg0: RequestContext<<ServerImpl as MyTrait>::Context>,
                __dropshot_websocket: dropshot::WebsocketUpgrade,
            ) -> dropshot::WebsocketEndpointResult {
                __dropshot_websocket
                    .handle(move |__dropshot_websocket: WebsocketConnection| async move {
                        <ServerImpl as MyTrait>::handler_ws(arg0, __dropshot_websocket)
                            .await
                    })
            }
            {
                let endpoint_handler_ws = dropshot::ApiEndpoint::new(
                    "vzeroupper".to_string(),
                    handler_ws_adapter::<ServerImpl>,
                    dropshot::Method::GET,
                    "application/json",
                    "/ws",
                    dropshot::ApiEndpointVersions::All,
                );
                if let Err(error) = dropshot_api.register(endpoint_handler_ws) {
                    dropshot_errors.push(error);
                }
            }
        }
        if !dropshot_errors.is_empty() {
            Err(dropshot::ApiDescriptionBuildErrors::new(dropshot_errors))
        } else {
            Ok(dropshot_api)
        }
    }
}
