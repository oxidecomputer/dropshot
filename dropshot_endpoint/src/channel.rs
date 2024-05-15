// Copyright 2023 Oxide Computer Company

//! Support for WebSocket `#[channel]` macros.

use crate::endpoint;
use crate::endpoint::EndpointMetadata;
use crate::error_store::ErrorSink;
use crate::error_store::ErrorStore;
use crate::syn_parsing::ItemFnForSignature;
use crate::util::APPLICATION_JSON;
use quote::quote;
use quote::ToTokens;
use serde::Deserialize;
use serde_tokenstream::from_tokenstream;
use serde_tokenstream::Error;
use std::ops::DerefMut;
use syn::spanned::Spanned;

/// Channel usage message, produced if there were parameter errors.
const USAGE: &str = "channel handlers must have the following signature:
    async fn(
        rqctx: dropshot::RequestContext<MyContext>,
        [query_params: Query<Q>,]
        [path_params: Path<P>,]
        websocket_connection: dropshot::WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult";

pub(crate) fn do_channel(
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> Result<(proc_macro2::TokenStream, Vec<Error>), Error> {
    let metadata: ChannelMetadata = from_tokenstream(&attr)?;
    let item: ItemFnForSignature = syn::parse2(item)?;

    let mut error_store = ErrorStore::new();

    let errors = error_store.sink();
    let channel = match ParsedChannel::new(metadata, attr, item.clone(), errors)
    {
        Some(channel) => channel,
        None => {
            // For now, None always means that there was a parameter
            // error.
            let mut errors = error_store.into_inner();
            errors.insert(0, Error::new_spanned(&item.sig, USAGE));
            return Ok((quote! {}, errors));
        }
    };

    let errors = error_store.sink();
    let output = do_channel_inner(channel, errors)?;

    let mut errors = error_store.into_inner();
    if output.has_param_errors {
        errors.insert(0, Error::new_spanned(&item.sig, USAGE));
    }

    Ok((output.output, errors))
}

/// Parsed output of a `#[channel]` macro.
///
/// Currently, we implement channels by turning them into endpoints and calling
/// the endpoint machinery. This struct contains this transformed form, ready
/// for the endpoint machinery to be called.
///
/// (This scheme, while convenient, leads to suboptimal error reporting -- so we
/// should consider doing our own, completely separate, implementation at some
/// point.)
struct ParsedChannel {
    metadata: EndpointMetadata,
    attr: proc_macro2::TokenStream,
    new_item: proc_macro2::TokenStream,
}

impl ParsedChannel {
    fn new(
        metadata: ChannelMetadata,
        attr: proc_macro2::TokenStream,
        item: ItemFnForSignature,
        errors: ErrorSink<'_, Error>,
    ) -> Option<Self> {
        let ChannelMetadata {
            // protocol was already used to determine the type of channel
            protocol,
            path,
            tags,
            unpublished,
            deprecated,
            _dropshot_crate,
        } = metadata;

        match protocol {
            ChannelProtocol::WEBSOCKETS => {
                // Here we construct a wrapper function and mutate the arguments a bit
                // for the outer layer: we replace WebsocketConnection, which is not
                // an extractor, with WebsocketUpgrade, which is.
                let ItemFnForSignature { attrs, vis, mut sig, _block: body } =
                    item;

                let inner_args = sig.inputs.clone();
                let inner_output = sig.output.clone();

                let arg_names: Vec<_> = inner_args
                    .iter()
                    .map(|arg: &syn::FnArg| match arg {
                        syn::FnArg::Receiver(r) => {
                            r.self_token.to_token_stream()
                        }
                        syn::FnArg::Typed(syn::PatType { pat, .. }) => {
                            pat.to_token_stream()
                        }
                    })
                    .collect();
                let found = sig.inputs.iter_mut().last().and_then(|arg| {
                    if let syn::FnArg::Typed(syn::PatType { pat, ty, .. }) = arg
                    {
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
                let (conn_name, conn_type) = match found {
                    Some(f) => f,
                    None => {
                        errors.push(Error::new_spanned(
                    &sig,
                    "An argument of type dropshot::WebsocketConnection must be provided last.",
                ));
                        return None;
                    }
                };

                sig.output =
                    syn::parse2(quote!(-> dropshot::WebsocketEndpointResult))
                        .expect("valid ReturnType");

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
                    content_type: Some(APPLICATION_JSON.to_string()),
                    _dropshot_crate,
                };

                Some(Self { metadata, attr, new_item })
            }
        }
    }
}

/// The result of calling [`ParsedChannel::to_output`].
struct ChannelOutput {
    /// The actual output.
    output: proc_macro2::TokenStream,

    /// Whether there were any parameter-related errors.
    has_param_errors: bool,
}

fn do_channel_inner(
    parsed: ParsedChannel,
    errors: ErrorSink<'_, Error>,
) -> Result<ChannelOutput, Error> {
    let ParsedChannel { metadata, attr, new_item } = parsed;
    let endpoint =
        endpoint::do_endpoint_inner(metadata, attr, new_item, errors)?;

    Ok(ChannelOutput {
        output: endpoint.output,
        has_param_errors: endpoint.has_param_errors,
        // Note: unlike EndpointOutput, we don't return the parsed
        // ItemFnForSignature. That's because that would be the endpoint we
        // generate up in Self::new, which is not the right span to report
        // errors with. Instead, whatever code generates the channel has the
        // original ItemFnForSignature.
    })
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
