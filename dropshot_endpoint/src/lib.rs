// Copyright 2023 Oxide Computer Company

//! Macro support for the Dropshot HTTP server.
//!
//! For more information about these macros, see the [Dropshot
//! documentation](https://docs.rs/dropshot).

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

// NOTE: We do not define documentation here -- only in the Dropshot crate while
// re-exporting these items. This is so that doctests that depend on the
// Dropshot crate work.

#[proc_macro_attribute]
pub fn endpoint(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    do_output(endpoint::do_endpoint(attr.into(), item.into()))
}

#[proc_macro_attribute]
pub fn channel(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    do_output(channel::do_channel(attr.into(), item.into()))
}

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
