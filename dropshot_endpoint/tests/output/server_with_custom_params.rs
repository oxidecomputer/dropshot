pub trait MyTrait: 'static {
    type Situation: topspin::ServerContext;
    fn handler_xyz(
        rqctx: RequestContext<Self::Situation>,
    ) -> impl ::core::future::Future<
        Output = Result<HttpResponseOk<()>, HttpError>,
    > + Send + 'static;
}
/// API description factory for the Dropshot server trait [`MyTrait`](MyTrait).
#[derive(Copy, Clone, Debug)]
#[automatically_derived]
pub enum MyMill {}
impl MyMill {
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
    ///     let stub = MyMill::stub_api_description().unwrap();
    ///
    ///     // Generate OpenAPI spec from `stub`.
    ///     let spec = stub.openapi("MyTrait", "0.1.0");
    ///     spec.write(&mut std::io::stdout()).unwrap();
    /// }
    /// ```
    ///
    /// [`MyTrait`]: MyTrait
    /// [`api_description`]: MyMill::api_description
    /// [`ApiDescription`]: topspin::ApiDescription
    /// [`StubContext`]: topspin::StubContext
    #[automatically_derived]
    pub fn stub_api_description() -> ::std::result::Result<
        topspin::ApiDescription<topspin::StubContext>,
        topspin::ApiDescriptionBuildError,
    > {
        let mut dropshot_api = topspin::ApiDescription::new();
        let mut dropshot_errors: Vec<String> = Vec::new();
        {
            let endpoint_handler_xyz = topspin::ApiEndpoint::new_stub::<
                (),
                Result<HttpResponseOk<()>, HttpError>,
            >(
                "handler_xyz".to_string(),
                topspin::Method::GET,
                "application/json",
                "/xyz",
            );
            if let Err(error) = dropshot_api.register(endpoint_handler_xyz) {
                dropshot_errors.push(error);
            }
        }
        if !dropshot_errors.is_empty() {
            Err(topspin::ApiDescriptionBuildError::new(dropshot_errors))
        } else {
            Ok(dropshot_api)
        }
    }
    /// Given an implementation of [`MyTrait`], generate an API description.
    ///
    /// This function accepts a single type argument `ServerImpl`, turning it into a
    /// Dropshot [`ApiDescription`]`<ServerImpl::`[`Situation`]`>`.
    /// The returned `ApiDescription` can then be turned into a Dropshot server that
    /// accepts a concrete `Situation`.
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
    ///     type Situation = /* context type */;
    ///
    ///     // ... trait methods
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     // Generate the description for `MyTraitImpl`.
    ///     let description = MyMill::api_description::<MyTraitImpl>().unwrap();
    ///
    ///     // Create a value of the concrete context type.
    ///     let context = /* some value of type `MyTraitImpl::Situation` */;
    ///
    ///     // Create a Dropshot server from the description.
    ///     let config = dropshot::ConfigDropshot::default();
    ///     let log = /* ... */;
    ///     let server = dropshot::HttpServerStarter::new(
    ///         &config,
    ///         description,
    ///         context,
    ///         &log,
    ///     ).unwrap();
    ///
    ///     // Run the server.
    ///     server.start().await
    /// }
    /// ```
    ///
    /// [`ApiDescription`]: topspin::ApiDescription
    /// [`MyTrait`]: MyTrait
    /// [`Situation`]: MyTrait::Situation
    #[automatically_derived]
    pub fn api_description<ServerImpl: MyTrait>() -> ::std::result::Result<
        topspin::ApiDescription<<ServerImpl as MyTrait>::Situation>,
        topspin::ApiDescriptionBuildError,
    > {
        let mut dropshot_api = topspin::ApiDescription::new();
        let mut dropshot_errors: Vec<String> = Vec::new();
        {
            let endpoint_handler_xyz = topspin::ApiEndpoint::new(
                "handler_xyz".to_string(),
                <ServerImpl as MyTrait>::handler_xyz,
                topspin::Method::GET,
                "application/json",
                "/xyz",
            );
            if let Err(error) = dropshot_api.register(endpoint_handler_xyz) {
                dropshot_errors.push(error);
            }
        }
        if !dropshot_errors.is_empty() {
            Err(topspin::ApiDescriptionBuildError::new(dropshot_errors))
        } else {
            Ok(dropshot_api)
        }
    }
}
