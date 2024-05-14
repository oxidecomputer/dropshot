// Copyright 2023 Oxide Computer Company

//! Support for WebSocket `#[channel]` macros.

use crate::endpoint;
use crate::syn_parsing::ItemFnForSignature;
use quote::quote;
use quote::ToTokens;
use serde::Deserialize;
use serde_tokenstream::from_tokenstream;
use serde_tokenstream::Error;
use std::ops::DerefMut;
use syn::spanned::Spanned;

pub(crate) fn do_channel(
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> Result<(proc_macro2::TokenStream, Vec<Error>), Error> {
    let ChannelMetadata {
        protocol,
        path,
        tags,
        unpublished,
        deprecated,
        _dropshot_crate,
    } = from_tokenstream(&attr)?;
    match protocol {
        ChannelProtocol::WEBSOCKETS => {
            // Here we construct a wrapper function and mutate the arguments a bit
            // for the outer layer: we replace WebsocketConnection, which is not
            // an extractor, with WebsocketUpgrade, which is.
            let ItemFnForSignature { attrs, vis, mut sig, _block: body } =
                syn::parse2(item)?;

            let inner_args = sig.inputs.clone();
            let inner_output = sig.output.clone();

            let arg_names: Vec<_> = inner_args
                .iter()
                .map(|arg: &syn::FnArg| match arg {
                    syn::FnArg::Receiver(r) => r.self_token.to_token_stream(),
                    syn::FnArg::Typed(syn::PatType { pat, .. }) => {
                        pat.to_token_stream()
                    }
                })
                .collect();
            let found = sig.inputs.iter_mut().last().and_then(|arg| {
                if let syn::FnArg::Typed(syn::PatType { pat, ty, .. }) = arg {
                    if let syn::Pat::Ident(syn::PatIdent {
                        ident,
                        by_ref: None,
                        ..
                    }) = pat.deref_mut()
                    {
                        let conn_type = ty.clone();
                        let conn_name = ident.clone();
                        let span = ident.span();
                        *ident = syn::Ident::new(
                            "__dropshot_websocket_upgrade",
                            span,
                        );
                        *ty = Box::new(syn::Type::Verbatim(
                            quote! { dropshot::WebsocketUpgrade },
                        ));
                        return Some((conn_name, conn_type));
                    }
                }
                return None;
            });
            if found.is_none() {
                return Err(Error::new_spanned(
                    &attr,
                    "An argument of type dropshot::WebsocketConnection must be provided last.",
                ));
            }

            sig.output =
                syn::parse2(quote!(-> dropshot::WebsocketEndpointResult))?;

            let (conn_name, conn_type) = found.unwrap();

            let new_item = quote! {
                #(#attrs)*
                #vis #sig {
                    async fn __dropshot_websocket_handler(#inner_args) #inner_output #body
                    __dropshot_websocket_upgrade.handle(move | #conn_name: #conn_type | async move {
                        __dropshot_websocket_handler(#(#arg_names),*).await
                    })
                }
            };

            let metadata = endpoint::EndpointMetadata {
                method: endpoint::MethodType::GET,
                path,
                tags,
                unpublished,
                deprecated,
                content_type: Some("application/json".to_string()),
                _dropshot_crate,
            };
            endpoint::do_endpoint_inner(metadata, attr, new_item)
        }
    }
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
enum ChannelProtocol {
    WEBSOCKETS,
}

#[derive(Deserialize, Debug)]
struct ChannelMetadata {
    protocol: ChannelProtocol,
    path: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    unpublished: bool,
    #[serde(default)]
    deprecated: bool,
    _dropshot_crate: Option<String>,
}
