// Copyright 2023 Oxide Computer Company

//! This package defines macro attributes associated with HTTP handlers. These
//! attributes are used both to define an HTTP API and to generate an OpenAPI
//! Spec (OAS) v3 document that describes the API.

#![forbid(unsafe_code)]

use quote::quote;
use serde_tokenstream::Error;

mod channel;
mod doc;
mod endpoint;
mod error_store;
mod params;
mod server;
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

/// Generates a Dropshot server from a trait.
///
/// A server trait consists of:
///
/// 1. A context type, typically `Self::Context`, values of which are shared
///    across all the endpoints.
/// 2. A set of endpoint methods, each of which is an `async fn` defined with
///    the same constraints, and via the same syntax, as [`macro@endpoint`] or
///    [`macro@channel`].
///
/// Server traits can also have arbitrary non-endpoint items, such as helper
/// functions.
///
/// The macro performs a number of checks on endpoint methods, and produces the
/// following items:
///
/// * The trait itself, with the following modifications to enable use as a
///   Dropshot server:
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
/// For more information about trait-based servers, see the Dropshot crate-level
/// documentation.
///
/// ## Arguments
///
/// The `#[dropshot::server]` macro accepts these arguments:
///
/// * `context`: The type of the context on the trait. Optional, and defaults to
///   `Self::Context`.
/// * `factory`: The name of the factory struct that will be generated.
///   Optional, defaulting to `<TraitName>Factory`.
///
/// ## Limitations
///
/// Currently, the `#[dropshot::server]` macro is only supported in module
/// contexts, not function definitions. This is a Rust limitation -- see [Rust
/// issue #79260](https://github.com/rust-lang/rust/issues/79260) for more
/// details.
///
/// ## Example
///
/// With a custom context type:
///
/// ```ignore
/// use dropshot::{RequestContext, HttpResponseUpdatedNoContent, HttpError};
///
/// #[dropshot::server { context = MyContext }]
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
/// # // defining fn main puts the doctest in a module context
/// # fn main() {}
/// ```
#[proc_macro_attribute]
pub fn server(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    do_output(server::do_server(attr.into(), item.into()))
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
