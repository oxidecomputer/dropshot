// Copyright 2023 Oxide Computer Company

//! This package defines macro attributes associated with HTTP handlers. These
//! attributes are used both to define an HTTP API and to generate an OpenAPI
//! Spec (OAS) v3 document that describes the API.

#![forbid(unsafe_code)]

use quote::quote;
use serde_tokenstream::Error;

mod api_trait;
mod channel;
mod doc;
mod endpoint;
mod error_store;
mod metadata;
mod params;
mod syn_parsing;
#[cfg(test)]
mod test_util;
mod util;

/// This attribute transforms a handler function into a Dropshot endpoint
/// suitable to be used as a parameter to
/// [`ApiDescription::register()`](../dropshot/struct.ApiDescription.html#method.register).
/// It encodes information relevant to the operation of an API endpoint beyond
/// what is expressed by the parameter and return types of a handler function.
///
/// ```ignore
/// #[endpoint {
///     // Required fields
///     method = { DELETE | HEAD | GET | OPTIONS | PATCH | POST | PUT },
///     path = "/path/name/with/{named}/{variables}",
///
///     // Optional tags for the operation's description
///     tags = [ "all", "your", "OpenAPI", "tags" ],
///     // Specifies the media type used to encode the request body
///     content_type = { "application/json" | "application/x-www-form-urlencoded" | "multipart/form-data" }
///     // A value of `true` marks the operation as deprecated
///     deprecated = { true | false },
///     // A value of `true` causes the operation to be omitted from the API description
///     unpublished = { true | false },
/// }]
/// ```
///
/// See the dropshot documentation for
/// [how to specify an endpoint](../dropshot/index.html#api-handler-functions)
/// or
/// [a description of the attribute parameters](../dropshot/index.html#endpoint----attribute-parameters)
#[proc_macro_attribute]
pub fn endpoint(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    do_output(endpoint::do_endpoint(attr.into(), item.into()))
}

/// As with [`macro@endpoint`], this attribute turns a handler function into a
/// Dropshot endpoint, but first wraps the handler function in such a way
/// that is spawned asynchronously and given the upgraded connection of
/// the given `protocol` (i.e. `WEBSOCKETS`).
///
/// The first argument still must be a `RequestContext<_>`.
///
/// The last argument passed to the handler function must be a
/// [`WebsocketConnection`](../dropshot/struct.WebsocketConnection.html).
///
/// The function must return a
/// [`WebsocketChannelResult`](dropshot/type.WebsocketChannelResult.html)
/// (which is a general-purpose `Result<(), Box<dyn Error + Send + Sync +
/// 'static>>`). Returned error values will be written to the RequestContext's
/// log.
///
/// ```ignore
/// #[dropshot::channel { protocol = WEBSOCKETS, path = "/my/ws/channel/{id}" }]
/// ```
#[proc_macro_attribute]
pub fn channel(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    do_output(channel::do_channel(attr.into(), item.into()))
}

/// Generates a Dropshot API description from a trait.
///
/// An API trait consists of:
///
/// 1. A context type, typically `Self::Context`, values of which are shared
///    across all the endpoints.
/// 2. A set of endpoint methods, each of which is an `async fn` defined with
///    the same constraints, and via the same syntax, as [`macro@endpoint`] or
///    [`macro@channel`].
///
/// API traits can also have arbitrary non-endpoint items, such as helper
/// functions.
///
/// The macro performs a number of checks on endpoint methods, and produces the
/// following items:
///
/// * The trait itself, with the following modifications to enable use as a
///   Dropshot API:
///
///     1. The trait itself has a `'static` bound added to it.
///     2. The context type has a `dropshot::ServerContext + 'static` bound
///        added to it, making it `Send + Sync + 'static`.
///     3. Each endpoint `async fn` is modified to become a function that
///        returns a `Send + 'static` future. (Implementations can continue to
///        define endpoints via the `async fn` syntax.)
///
///   Non-endpoint items are left unchanged.
///
/// * A support module, typically with the same name as the trait but in
///   `snake_case`, with two functions:
///
///     1. `api_description()`, which accepts an implementation of the trait as
///        a type argument and generates an `ApiDescription`.
///     2. `stub_api_description()`, which generates a _stub_ `ApiDescription`
///        that can be used to generate an OpenAPI spec without having an
///        implementation of the trait available.
///
/// For more information about API traits, see the Dropshot crate-level
/// documentation.
///
/// ## Arguments
///
/// The `#[dropshot::api_description]` macro accepts these arguments:
///
/// * `context`: The type of the context on the trait. Optional, defaults to
///   `Self::Context`.
/// * `module`: The name of the support module. Optional, defaults to the
///   `{T}_mod`, where `T` is the snake_case version of the trait name.
///
///    For example, for a trait called `MyApi` the corresponding module name
///    would be `my_api_mod`.
///
///    (The suffix `_mod` is added to module names so that a crate called
///    `my-api` can define a trait `MyApi`, avoiding name conflicts.)
/// * `tag_config`: Trait-wide tag configuration. _Optional._ For more
///   information, see [_Tag configuration_](#tag-configuration) below.
///
/// ### Example: specify a custom context type
///
/// ```ignore
/// use dropshot::{RequestContext, HttpResponseUpdatedNoContent, HttpError};
///
/// #[dropshot::api_description { context = MyContext }]
/// trait MyTrait {
///     type MyContext;
///
///     #[endpoint {
///         method = PUT,
///         path = "/test",
///     }]
///     async fn put_test(
///         rqctx: RequestContext<Self::MyContext>,
///     ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
/// }
/// # // defining fn main puts thoe doctest in a module context
/// # fn main() {}
/// ```
///
/// ### Tag configuration
///
/// Endpoints can have tags associated with them that appear in the OpenAPI
/// document. These provide a means of organizing an API. Use the `tag_config`
/// argument to specify tag information for the trait.
///
/// `tag_config` is optional. If not specified, the default is to allow any tag
/// value and any number of tags associated with the endpoint.
///
/// If `tag_config` is specified, compliance with it is evaluated at runtime
/// while registering endpoints. A failure to comply--for example, if `policy =
/// at_least_one` is specified and some endpoint has no associated tags--results
/// in the `api_description` and `stub_api_description` functions returning an
/// error.
///
/// The shape of `tag_config` is broadly similar to that of
/// `dropshot::TagConfig`. It has the following fields:
///
/// * `tags`: A map of tag names with information about them. _Required, but can
///   be empty._
///
///   The keys are tag names, which are strings. The values are objects that
///   consist of:
///
///   * `description`: A string description of the tag. _Optional._
///   * `external_docs`: External documentation for the tag. _Optional._ This
///     has the following fields:
///     * `description`: A string description of the external documentation.
///       _Optional._
///     * `url`: The URL for the external documentation. _Required._
///
/// * `allow_other_tags`: Whether to allow tags not explicitly defined in
///   `tags`. _Optional, defaults to false. But if `tag_config` as a whole is
///   not specified, all tags are allowed._
///
/// * `policy`: Must be an expression of type `EndpointTagPolicy`; typically just
///   the enum variant.
///
///   _Optional, defaults to `EndpointTagPolicy::Any`._
///
/// ### Example: tag configuration
///
/// ```ignore
/// use dropshot::{
///     EndpointTagPolicy, RequestContext, HttpResponseUpdatedNoContent,
///     HttpError,
/// };
///
/// #[dropshot::api_description {
///     tag_config = {
///         // If tag_config is specified, tags is required (but can be empty).
///         tags = {
///             "tag1" = {
///                 // The description is optional.
///                 description = "Tag 1",
///                 // external_docs is optional.
///                 external_docs = {
///                     // The description is optional.
///                     description = "External docs for tag1",
///                     // If external_docs is present, url is required.
///                     url = "https://example.com/tag1",
///                 },
///             },
///         },
///         policy = EndpointTagPolicy::ExactlyOne,
///         allow_other_tags = false,
///     },
/// }]
/// trait MyTrait {
///     type Context;
///
///     #[endpoint {
///         method = PUT,
///         path = "/test",
///         tags = ["tag1"],
///     }]
///     async fn put_test(
///         rqctx: RequestContext<Self::MyContext>,
///     ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
/// }
/// # // defining fn main puts the doctest in a module context
/// # fn main() {}
/// ```
///
/// ## Limitations
///
/// Currently, the `#[dropshot::api_description]` macro is only supported in
/// module contexts, not function definitions. This is a Rust limitation -- see
/// [Rust issue #79260](https://github.com/rust-lang/rust/issues/79260) for more
/// details.
///
/// ## More information
///
/// For more information about the design decisions behind API traits, see
/// [Oxide RFD 479](https://rfd.shared.oxide.computer/rfd/0479)
#[proc_macro_attribute]
pub fn api_description(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    do_output(api_trait::do_trait(attr.into(), item.into()))
}

fn do_output(
    (endpoint, errors): (proc_macro2::TokenStream, Vec<Error>),
) -> proc_macro::TokenStream {
    let compiler_errors = errors.iter().map(|err| err.to_compile_error());

    let output = quote! {
        #endpoint
        #( #compiler_errors )*
    };

    output.into()
}
