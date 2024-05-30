// Copyright 2023 Oxide Computer Company

//! This package defines macro attributes associated with HTTP handlers. These
//! attributes are used both to define an HTTP API and to generate an OpenAPI
//! Spec (OAS) v3 document that describes the API.

use quote::quote;
use serde_tokenstream::Error;

mod channel;
mod doc;
mod endpoint;
mod error_store;
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
