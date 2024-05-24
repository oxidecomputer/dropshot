// Copyright 2023 Oxide Computer Company

//! Support for WebSocket `#[channel]` macros.

use crate::endpoint;
use crate::endpoint::EndpointMetadata;
use crate::error_store::ErrorSink;
use crate::error_store::ErrorStore;
use crate::syn_parsing::ItemFnForSignature;
use crate::syn_parsing::TraitItemFnForSignature;
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
) -> (proc_macro2::TokenStream, Vec<Error>) {
    let mut error_store = ErrorStore::new();
    let errors = error_store.sink();

    // Attempt to parse the function as a trait function. If this is successful
    // and there's no block, then it's likely that the user has imported
    // `dropshot::endpoint` and is using that.
    if let Ok(trait_item_fn) =
        syn::parse2::<TraitItemFnForSignature>(item.clone())
    {
        if trait_item_fn.block.is_none() {
            let name = &trait_item_fn.sig.ident;
            errors.push(Error::new_spanned(
                        &trait_item_fn.sig,
                        format!(
                            "endpoint `{name}` appears to be a trait function\n\
                             (did you mean to use `#[dropshot::server]` instead?)",
                        ),
                    ));
            // Don't do any further validation -- just return the original item.
            return (quote! { #item }, error_store.into_inner());
        }
    }

    // Parse attributes. (Do this before parsing the function since that's the
    // order they're in, in source code.)
    let metadata = match from_tokenstream(&attr) {
        Ok(metadata) => Some(metadata),
        Err(e) => {
            // If there is an error while parsing the metadata, report it, but
            // continue to generate the original function.
            errors.push(e);
            None
        }
    };

    // Attempt to parse the function.
    let item_fn = match syn::parse2::<ItemFnForSignature>(item.clone()) {
        Ok(item_fn) => Some(item_fn),
        Err(e) => {
            errors.push(e);
            None
        }
    };

    let output = match (metadata, item_fn.as_ref()) {
        (Some(metadata), Some(item_fn)) => {
            match ParsedChannel::new(metadata, attr, item_fn.clone(), errors) {
                Some(channel) => {
                    // The happy path.
                    let errors = error_store.sink();
                    do_channel_inner(channel, errors)
                }
                None => {
                    // For now, None always means that there was a parameter
                    // error. Generate the original function (but not the
                    // attribute proc macro).
                    ChannelOutput {
                        output: quote! { #item },
                        has_param_errors: true,
                    }
                }
            }
        }
        (None, Some(_)) => {
            // In this case, continue to generate the original function (but not
            // the attribute proc macro).
            ChannelOutput { output: quote! { #item }, has_param_errors: false }
        }
        (_, None) => {
            // Can't do anything here, just return errors.
            ChannelOutput { output: quote! {}, has_param_errors: false }
        }
    };

    let mut errors = error_store.into_inner();
    if output.has_param_errors {
        let item_fn = item_fn
            .as_ref()
            .expect("has_param_errors is true => item_fn is Some");
        errors.insert(0, Error::new_spanned(&item_fn.sig, USAGE));
    }

    (output.output, errors)
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
    endpoint_item: proc_macro2::TokenStream,
    endpoint_fn: ItemFnForSignature,
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
                let name_str = sig.ident.to_string();

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
                            format!(
                                "endpoint `{name_str}` must have a \
                                 WebsocketConnection as its last argument",
                            ),
                        ));
                        return None;
                    }
                };

                sig.output =
                    syn::parse2(quote!(-> dropshot::WebsocketEndpointResult))
                        .expect("valid ReturnType");

                let endpoint_item = quote! {
                    #(#attrs)*
                    #vis #sig {
                        async fn __dropshot_websocket_handler(#inner_args) #inner_output #body
                        __dropshot_websocket_upgrade.handle(move | #conn_name: #conn_type | async move {
                            __dropshot_websocket_handler(#(#arg_names),*).await
                        })
                    }
                };

                // Attempt to parse the new item immediately. This should always
                // work, but if it doesn't, error out right away.
                let endpoint_fn = match syn::parse2::<ItemFnForSignature>(
                    endpoint_item.clone(),
                ) {
                    Ok(endpoint_fn) => endpoint_fn,
                    Err(e) => {
                        errors.push(e);
                        return None;
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

                Some(Self { metadata, attr, endpoint_item, endpoint_fn })
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
) -> ChannelOutput {
    let ParsedChannel { metadata, attr, endpoint_item, endpoint_fn } = parsed;
    let endpoint = endpoint::do_endpoint_inner(
        metadata,
        attr,
        endpoint_item,
        &endpoint_fn,
        errors,
    );

    ChannelOutput {
        output: endpoint.output,
        has_param_errors: endpoint.has_param_errors,
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
