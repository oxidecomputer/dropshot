// Copyright 2023 Oxide Computer Company

//! Support for WebSocket `#[channel]` macros.

use crate::doc::ExtractedDoc;
use crate::error_store::ErrorSink;
use crate::error_store::ErrorStore;
use crate::metadata::ApiEndpointKind;
use crate::metadata::ChannelMetadata;
use crate::params::validate_fn_ast;
use crate::params::ParamValidator;
use crate::params::RqctxKind;
use crate::params::RqctxTy;
use crate::syn_parsing::ItemFnForSignature;
use crate::syn_parsing::TraitItemFnForSignature;
use crate::util::MacroKind;
use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use quote::quote_spanned;
use serde_tokenstream::from_tokenstream;
use serde_tokenstream::Error;
use syn::parse_quote;
use syn::spanned::Spanned;

pub(crate) fn usage_str(context: &str) -> String {
    format!(
        "channel handlers must have the following signature:
    async fn(
        rqctx: dropshot::RequestContext<{context}>,
        [query_params: Query<Q>,]
        [path_params: Path<P>,]
        websocket_connection: dropshot::WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult"
    )
}

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
                     note: did you mean to use `#[dropshot::api_description]` \
                     instead?",
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
            // The happy path.
            do_channel_inner(metadata, attr, item, &item_fn, errors)
        }
        (None, Some(_)) => {
            // In this case, continue to generate the original function (but not
            // the attribute proc macro).
            ChannelOutput {
                output: quote! { #item },
                // We don't validate parameters, so we don't know if there are
                // errors in them.
                has_param_errors: false,
            }
        }
        (_, None) => {
            // Can't do anything here, just return errors.
            ChannelOutput { output: quote! {}, has_param_errors: false }
        }
    };

    let mut errors = error_store.into_inner();

    // If there are any errors, we also want to provide a usage message as an error.
    if output.has_param_errors {
        let item_fn = item_fn
            .as_ref()
            .expect("has_param_errors is true => item_fn is Some");
        errors.insert(
            0,
            Error::new_spanned(&item_fn.sig, usage_str("MyContext")),
        );
    }

    (output.output, errors)
}

/// The result of calling [`do_channel`].
struct ChannelOutput {
    /// The actual output.
    output: proc_macro2::TokenStream,

    /// Whether there were any parameter-related errors.
    has_param_errors: bool,
}

fn do_channel_inner(
    metadata: ChannelMetadata,
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
    item_fn: &ItemFnForSignature,
    errors: ErrorSink<'_, Error>,
) -> ChannelOutput {
    let dropshot = metadata.dropshot_crate();

    let name = &item_fn.sig.ident;
    let name_str = name.to_string();

    // Perform validations first.
    let metadata =
        metadata.validate(&name_str, &attr, MacroKind::Function, &errors);
    let params = ChannelParams::new(
        &dropshot,
        &item_fn.sig,
        RqctxKind::Function,
        &errors,
    );

    let visibility = &item_fn.vis;

    let doc = ExtractedDoc::from_attrs(&item_fn.attrs);
    let comment_text = doc.comment_text(&name_str);

    let description_doc_comment = quote! {
        #[doc = #comment_text]
    };

    // If the params are valid, output the corresponding type checks and impl
    // statement.
    let (has_param_errors, type_checks, from_impl) =
        if let Some(params) = &params {
            let type_checks = params.to_type_checks();
            let impl_checks = params.to_impl_checks(name);
            let adapter_fn = params.to_adapter_fn();

            // If the metadata is valid, output the corresponding ApiEndpoint.
            let construct = if let Some(metadata) = metadata {
                metadata.to_api_endpoint_fn(
                    &dropshot,
                    &name_str,
                    &ApiEndpointKind::Regular(&params.adapter_name),
                    &doc,
                )
            } else {
                quote! {
                    unreachable!()
                }
            };

            let rqctx_context = params.rqctx_context();

            let from_impl = quote! {
                impl From<#name>
                    for #dropshot::ApiEndpoint< #rqctx_context >
                {
                    fn from(_: #name) -> Self {
                        #[allow(clippy::unused_async)]
                        #item

                        // The checks on the implementation require #name to be in
                        // scope, which is provided by #item, hence we place these
                        // checks here instead of above with the others.
                        #impl_checks

                        // Generate the adapter function after the checks so
                        // that errors from the checks show up first.
                        #adapter_fn

                        #construct
                    }
                }
            };

            (false, type_checks, from_impl)
        } else {
            (true, quote! {}, quote! {})
        };

    // For reasons that are not well understood unused constants that use the
    // (default) call_site() Span do not trigger the dead_code lint. Because
    // defining but not using an endpoint is likely a programming error, we
    // want to be sure to have the compiler flag this. We force this by using
    // the span from the name of the function to which this macro was applied.
    let span = item_fn.sig.ident.span();
    let const_struct = quote_spanned! {span=>
        #visibility const #name: #name = #name {};
    };

    // The final TokenStream returned will have a few components that reference
    // `#name`, the name of the function to which this macro was applied...
    let stream = quote! {
        // ... type validation for parameter and return types
        #type_checks

        // ... a struct type called `#name` that has no members
        #[allow(non_camel_case_types, missing_docs)]
        #description_doc_comment
        #visibility struct #name {}
        // ... a constant of type `#name` whose identifier is also #name
        #[allow(non_upper_case_globals, missing_docs)]
        #description_doc_comment
        #const_struct

        // ... an impl of `From<#name>` for ApiEndpoint that allows the constant
        // `#name` to be passed into `ApiDescription::register()`
        #from_impl
    };

    ChannelOutput { output: stream, has_param_errors }
}

pub(crate) struct ChannelParams<'ast> {
    dropshot: TokenStream,
    sig: &'ast syn::Signature,
    rqctx_ty: RqctxTy<'ast>,
    shared_extractors: Vec<&'ast syn::Type>,
    websocket_conn: &'ast syn::Type,
    pub(crate) ret_ty: &'ast syn::Type,
    pub(crate) adapter_name: syn::Ident,

    // Types used in the adapter function, generated at construction time.
    websocket_upgrade_ty: syn::Type,
    pub(crate) endpoint_result_ty: syn::Type,
}

impl<'ast> ChannelParams<'ast> {
    pub(crate) fn new(
        dropshot: &TokenStream,
        sig: &'ast syn::Signature,
        rqctx_kind: RqctxKind<'_>,
        errors: &ErrorSink<'_, Error>,
    ) -> Option<Self> {
        let name_str = sig.ident.to_string();
        let errors = errors.new();

        validate_fn_ast(sig, &name_str, &errors);

        let mut params = ParamValidator::new(sig, &name_str);
        params.maybe_discard_self_arg(&errors);

        let (rqctx_ty, websocket_conn_ty) = params
            .next_rqctx_and_last_websocket_args(
                rqctx_kind,
                &sig.paren_token.span,
                &errors,
            );

        // All other parameters must impl SharedExtractor.
        let shared_extractors = params.rest_extractor_args(&errors);

        // Validate the websocket connection type.
        let websocket_conn =
            websocket_conn_ty.and_then(|ty| ty.validate(&name_str, &errors));

        let ret_ty = params.return_type(&errors);

        let websocket_upgrade_ty = parse_quote! { #dropshot::WebsocketUpgrade };
        let endpoint_result_ty =
            parse_quote! { #dropshot::WebsocketEndpointResult };

        // Use the entire function signature as the span for the generated
        // identifier, to avoid confusing rust-analyzer (attributing multiple
        // items to a single function make makes ctrl-click worse).
        let adapter_name =
            format_ident!("{}_adapter", sig.ident, span = sig.span());

        // errors.has_errors() must be checked first, because it's possible for
        // the below variables to all be Some, but one of the extractors to have
        // errored out.
        if errors.has_errors() {
            None
        } else if let (Some(rqctx_ty), Some(websocket_conn), Some(ret_ty)) =
            (rqctx_ty, websocket_conn, ret_ty)
        {
            Some(Self {
                dropshot: dropshot.clone(),
                sig,
                rqctx_ty,
                ret_ty,
                shared_extractors,
                websocket_conn,
                adapter_name,
                websocket_upgrade_ty,
                endpoint_result_ty,
            })
        } else {
            unreachable!(
                "no param errors, but rqctx_ty, \
                 websocket_upgrade or ret_ty is None"
            );
        }
    }

    fn to_type_checks(&self) -> TokenStream {
        let dropshot = &self.dropshot;
        let rqctx_context = self.rqctx_context();
        let rqctx_check = quote_spanned! { self.rqctx_ty.orig_span()=>
            const _: fn() = || {
                #[allow(dead_code)]
                struct NeedRequestContext(#rqctx_context);
            };
        };

        let shared_extractor_checks = self.shared_extractors.iter().map(|ty| {
            quote_spanned! { ty.span()=>
                const _: fn() = || {
                    fn need_shared_extractor<T>()
                    where
                        T: ?Sized + #dropshot::SharedExtractor,
                    {
                    }
                    need_shared_extractor::<#ty>();
                };
            }
        });

        let trait_type_checks = self.to_trait_type_checks();

        quote! {
            #rqctx_check

            #(#shared_extractor_checks)*

            #trait_type_checks
        }
    }

    /// Generate type checks suitable for server traits.
    ///
    /// Unlike with endpoints, we do need to generate some of the type checks
    /// within traits. That's because we use an adapter function -- so types
    /// that are unique to the user function need to be checked.
    pub(crate) fn to_trait_type_checks(&self) -> TokenStream {
        let dropshot = &self.dropshot;

        let websocket_conn = self.websocket_conn;
        let websocket_conn_check = quote_spanned! { websocket_conn.span()=>
            const _: fn() = || {
                trait TypeEq {
                    type This: ?Sized;
                }
                impl<T: ?Sized> TypeEq for T {
                    type This = Self;
                }
                fn validate_websocket_connection_type<T>()
                where
                    T: ?Sized + TypeEq<This = #dropshot::WebsocketConnection>,
                {
                }
                validate_websocket_connection_type::<#websocket_conn>();
            };
        };

        // Checking the return type doesn't appear to be necessary, because the
        // `WebsocketUpgrade::handle` function produces a decent error message
        // if it's wrong. (In any case, an explicit check doesn't do a better
        // job and just produces noise.)

        quote! {
            #websocket_conn_check
        }
    }

    /// Returns a token stream that obtains the rqctx context type.
    fn rqctx_context(&self) -> TokenStream {
        self.rqctx_ty.to_context(&self.dropshot)
    }

    /// Returns a list of generated argument names.
    fn arg_names(&self) -> impl Iterator<Item = syn::Ident> + '_ {
        // The total number of arguments is 1 (rqctx) + the number of shared
        // extractors + 1 (websocket_upgrade). The last argument we'll name
        // specially so it can be referred to below.
        let arg_count = 1 + self.shared_extractors.len();

        (0..arg_count)
            .map(|i| format_ident!("arg{}", i))
            .chain(std::iter::once(format_ident!("__dropshot_websocket")))
    }

    /// Returns a list of all argument types, including the request context.
    fn arg_types(&self) -> impl Iterator<Item = &syn::Type> + '_ {
        std::iter::once(self.rqctx_ty.transformed_unit_type())
            .chain(self.shared_extractors.iter().copied())
            .chain(std::iter::once(self.websocket_conn))
    }

    /// Returns a list of all the argument types as they should show up in the
    /// adapter function.
    fn adapter_arg_types(&self) -> impl Iterator<Item = &syn::Type> + '_ {
        std::iter::once(self.rqctx_ty.transformed_server_impl_type())
            .chain(self.extractor_types())
    }

    /// Returns a list of the extractor types.
    ///
    /// The exclusive extractor in this situation is `WebsocketUpgrade`.
    pub(crate) fn extractor_types(
        &self,
    ) -> impl Iterator<Item = &syn::Type> + '_ {
        self.shared_extractors
            .iter()
            .copied()
            .chain(std::iter::once(&self.websocket_upgrade_ty))
    }

    /// Constructs implementation checks for the endpoint.
    ///
    /// These checks are placed in the same scope as the definition of the
    /// endpoint.
    fn to_impl_checks(&self, name: &syn::Ident) -> TokenStream {
        // We want to construct a function that will call the user's endpoint,
        // so we can check the future it returns for bounds that otherwise
        // produce inscrutable error messages (like returning a non-`Send`
        // future). We produce a wrapper function that takes all the same
        // argument types, which requires building up a list of argument names:
        // we can't use the original definition's argument names since they
        // could have multiple args named `_`, so we use "arg0", "arg1", etc.
        let arg_names: Vec<_> = self.arg_names().collect();
        let arg_types = self.arg_types();

        quote! {
            const _: fn() = || {
                fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
                fn check_future_bounds(#( #arg_names: #arg_types ),*) {
                    future_endpoint_must_be_send(#name(#(#arg_names),*));
                }
            };
        }
    }

    /// Constructs the adapter function that will call the user's endpoint.
    ///
    /// Currently, channels are implemented as adapters over endpoints. This
    /// function translates channel functions into endpoint functions.
    fn to_adapter_fn(&self) -> TokenStream {
        let arg_names = self.arg_names();
        let arg_names_2 = self.arg_names();
        let adapter_arg_types = self.adapter_arg_types();
        let websocket_conn = self.websocket_conn;
        let name = &self.sig.ident;
        let adapter_name = &self.adapter_name;
        let endpoint_result_ty = &self.endpoint_result_ty;

        quote_spanned! {self.sig.span()=>
            async fn #adapter_name(
                #( #arg_names: #adapter_arg_types ),*
            ) -> #endpoint_result_ty {
                __dropshot_websocket.handle(
                    move | __dropshot_websocket: #websocket_conn | async move {
                        #name(#(#arg_names_2),*).await
                    },
                )
            }
        }
    }

    /// Constructs the adapter function that calls the user's endpoint on a
    /// server trait.
    ///
    /// This is a bit more complicated than the regular adapter function, since
    /// the server trait must be referred to carefully.
    pub(crate) fn to_trait_adapter_fn(
        &self,
        trait_ident: &syn::Ident,
    ) -> TokenStream {
        let arg_names = self.arg_names();
        let arg_names_2 = self.arg_names();
        let adapter_arg_types = self.adapter_arg_types();
        let websocket_conn = self.websocket_conn;
        let name = &self.sig.ident;
        let adapter_name = &self.adapter_name;
        let endpoint_result_ty = &self.endpoint_result_ty;

        quote_spanned! {self.sig.span()=>
            async fn #adapter_name<ServerImpl: #trait_ident>(
                #( #arg_names: #adapter_arg_types ),*
            ) -> #endpoint_result_ty {
                __dropshot_websocket.handle(
                    move | __dropshot_websocket: #websocket_conn | async move {
                        <ServerImpl as #trait_ident>::#name(#(#arg_names_2),*).await
                    },
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use expectorate::assert_contents;
    use syn::parse_quote;

    use crate::{
        test_util::{assert_banned_idents, find_idents},
        util::DROPSHOT,
    };

    use super::*;

    #[test]
    fn test_channel_with_custom_params() {
        let input = quote! {
            async fn my_channel(
                rqctx: RequestContext<()>,
                query: Query<Q>,
                conn: WebsocketConnection,
            ) -> WebsocketChannelResult {
                Ok(())
            }
        };

        let (item, errors) = do_channel(
            quote! {
                protocol = WEBSOCKETS,
                path = "/my/ws/channel",
                _dropshot_crate = "topspin",
            },
            input.clone(),
        );

        assert!(errors.is_empty());

        let file = parse_quote! { #item };
        // Write out the file before checking it for banned idents, so that we
        // can see what it looks like.
        assert_contents(
            "tests/output/channel_with_custom_params.rs",
            &prettyplease::unparse(&file),
        );

        // Check banned identifiers.
        let banned = [DROPSHOT];
        assert_banned_idents(&file, banned);

        // Without _dropshot_crate, the generated output must contain
        // "dropshot".
        let (item, errors) = do_channel(
            quote! {
                protocol = WEBSOCKETS,
                path = "/my/ws/channel",
            },
            input,
        );

        assert!(errors.is_empty());
        let file = parse_quote! { #item };
        assert_eq!(
            find_idents(&file, banned).into_iter().collect::<Vec<_>>(),
            banned
        );
    }

    #[test]
    fn test_channel_with_unnamed_params() {
        let (item, errors) = do_channel(
            quote! {
                protocol = WEBSOCKETS,
                path = "/my/ws/channel",
            },
            quote! {
                async fn handler_xyz(
                    _: RequestContext<()>,
                    _: Query<Q>,
                    _: Path<P>,
                    _: WebsocketConnection,
                ) -> WebsocketChannelResult {
                    Ok(())
                }
            },
        );

        println!("{:?}", errors);

        assert!(errors.is_empty());
        assert_contents(
            "tests/output/channel_with_unnamed_params.rs",
            &prettyplease::unparse(&parse_quote! { #item }),
        );
    }

    #[test]
    fn test_channel_operation_id() {
        let (item, errors) = do_channel(
            quote! {
                protocol = WEBSOCKETS,
                path = "/my/ws/channel",
                operation_id = "vzeroupper"
            },
            quote! {
                async fn handler_xyz(
                    _rqctx: RequestContext<()>,
                    _ws: WebsocketConnection,
                ) -> Result<HttpResponseOk<()>, HttpError> {
                    Ok(())
                }
            },
        );

        assert!(errors.is_empty());
        assert_contents(
            "tests/output/channel_operation_id.rs",
            &prettyplease::unparse(&parse_quote! { #item }),
        );
    }

    #[test]
    fn test_channel_with_versions() {
        let input = quote! {
            async fn my_channel(
                _rqctx: RequestContext<()>,
                _conn: WebsocketConnection,
            ) -> WebsocketChannelResult {
                Ok(())
            }
        };

        let (item, errors) = do_channel(
            quote! {
                protocol = WEBSOCKETS,
                path = "/my/ws/channel",
                versions = .."1.2.3",
            },
            input.clone(),
        );

        assert!(errors.is_empty());

        let file = parse_quote! { #item };
        assert_contents(
            "tests/output/channel_with_versions.rs",
            &prettyplease::unparse(&file),
        );
    }
}
