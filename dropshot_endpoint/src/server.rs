// Copyright 2023 Oxide Computer Company

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned, ToTokens};
use serde::Deserialize;
use serde_tokenstream::from_tokenstream;
use syn::{parse_quote, Error};

use crate::{
    doc::ExtractedDoc,
    endpoint::{
        ApiEndpointKind, EndpointMetadata, EndpointParams, RqctxKind,
        ValidatedEndpointMetadata,
    },
    error_store::{ErrorSink, ErrorStore},
    syn_parsing::{
        ItemTraitPartParsed, TraitItemFnForSignature, TraitItemPartParsed,
        UnparsedBlock,
    },
    util::{get_crate, MacroKind},
};

pub(crate) fn do_server(
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> (proc_macro2::TokenStream, Vec<Error>) {
    let mut error_store = ErrorStore::new();
    let errors = error_store.sink();

    // Parse attributes. (Do this before parsing the trait since that's the
    // order they're in, in source code.)
    let server_metadata = match from_tokenstream::<ServerMetadata>(&attr) {
        Ok(m) => Some(m),
        Err(e) => {
            errors.push(e);
            None
        }
    };

    // Attempt to parse the trait.
    let item_trait = match syn::parse2::<ItemTraitPartParsed>(item) {
        Ok(item_trait) => Some(item_trait),
        Err(e) => {
            errors.push(e);
            None
        }
    };

    let output = match (server_metadata, item_trait) {
        (Some(server_metadata), Some(item_trait)) => {
            // The happy path.
            let server = Server::new(server_metadata, &item_trait, errors);
            server.to_output()
        }
        (None, Some(item_trait)) => {
            // This is a case where we can do something useful. Don't try and
            // validate the input, but we can at least regenerate the same type
            // with endpoint and channel attributes stripped.
            let server = Server::invalid_no_metadata(&item_trait);
            server.to_output()
        }
        (_, None) => {
            // Can't do anything here, just return errors.
            quote! {}
        }
    };

    let errors = error_store.into_inner();

    // XXX: If there are any parameter errors, also provide a usage message.

    (output, errors)
}

#[derive(Deserialize, Debug)]
struct ServerMetadata {
    #[serde(default)]
    context: Option<String>,
    _dropshot_crate: Option<String>,
}

impl ServerMetadata {
    /// The default name for the associated context type: `Self::Context`.
    const DEFAULT_CONTEXT_TY: &'static str = "Context";

    /// Returns the dropshot crate value as a TokenStream.
    fn dropshot_crate(&self) -> TokenStream {
        get_crate(self._dropshot_crate.as_deref())
    }

    fn context_ty(&self) -> &str {
        self.context.as_deref().unwrap_or(Self::DEFAULT_CONTEXT_TY)
    }
}

struct Server<'ast> {
    dropshot: TokenStream,
    item_trait: &'ast ItemTraitPartParsed,
    // We want to maintain the order of items in the trait (other than the
    // Context associated type which we're always going to move to the top), so
    // we use a single list to store all of them.
    items: Vec<ServerItem<'ast>>,
    // This is None if there is no context type, which is an error.
    context_item: Option<&'ast syn::TraitItemType>,
}

const ENDPOINT_IDENT: &str = "endpoint";
const CHANNEL_IDENT: &str = "channel";

impl<'ast> Server<'ast> {
    fn new(
        metadata: ServerMetadata,
        item_trait: &'ast ItemTraitPartParsed,
        errors: ErrorSink<'_, Error>,
    ) -> Self {
        let dropshot = metadata.dropshot_crate();
        let mut items = Vec::with_capacity(item_trait.items.len());

        let context_ty = metadata.context_ty();
        let context_ident = format_ident!("{}", context_ty);
        let mut context_item = None;

        for item in &item_trait.items {
            match item {
                TraitItemPartParsed::Fn(f) => {
                    // XXX cannot have multiple endpoint or channel attributes,
                    // check here.
                    let endpoint_attr = f
                        .attrs
                        .iter()
                        .find(|a| a.path().is_ident(ENDPOINT_IDENT));
                    let channel_attr = f
                        .attrs
                        .iter()
                        .find(|a| a.path().is_ident(CHANNEL_IDENT));

                    let item = match (endpoint_attr, channel_attr) {
                        (Some(_), Some(cattr)) => {
                            let name = &f.sig.ident;
                            errors.push(Error::new_spanned(
                                cattr,
                                format!("method `{name}` marked as both endpoint and channel"),
                            ));
                            ServerItem::Invalid(f)
                        }
                        (Some(eattr), None) => {
                            if let Some(endpoint) = ServerEndpoint::new(
                                f,
                                eattr,
                                &context_ident,
                                &errors,
                            ) {
                                ServerItem::Endpoint(endpoint)
                            } else {
                                ServerItem::Invalid(f)
                            }
                        }
                        (None, Some(cattr)) => {
                            if let Some(channel) = ServerChannel::new(f, cattr)
                            {
                                ServerItem::Channel(channel)
                            } else {
                                ServerItem::Invalid(f)
                            }
                        }
                        (None, None) => {
                            // This is just a normal method.
                            ServerItem::Other(item)
                        }
                    };
                    items.push(item);
                }

                // Everything else is permissible -- just ensure that they
                // aren't marked as `endpoint` or `channel`.
                TraitItemPartParsed::Other(other) => {
                    let should_push = match other {
                        syn::TraitItem::Const(c) => {
                            check_endpoint_or_channel_on_non_fn(
                                "const",
                                &c.ident.to_string(),
                                &c.attrs,
                                &errors,
                            );
                            true
                        }
                        syn::TraitItem::Fn(_) => {
                            unreachable!(
                                "function items should have been handled above"
                            )
                        }
                        syn::TraitItem::Type(t) => {
                            check_endpoint_or_channel_on_non_fn(
                                "type",
                                &t.ident.to_string(),
                                &t.attrs,
                                &errors,
                            );

                            // We're looking for the context type. Look for a type with
                            // the right name.
                            if t.ident == context_ty {
                                // This is the context type.
                                context_item = Some(t);
                                false
                            } else {
                                // This is something else.
                                items.push(ServerItem::Other(item));
                                true
                            }
                        }
                        syn::TraitItem::Macro(m) => {
                            check_endpoint_or_channel_on_non_fn(
                                "macro",
                                &m.mac.path.to_token_stream().to_string(),
                                &m.attrs,
                                &errors,
                            );
                            true
                        }
                        _ => true,
                    };

                    if should_push {
                        items.push(ServerItem::Other(item));
                    }
                }
            }
        }

        if context_item.is_none() {
            errors.push(Error::new_spanned(
                &item_trait.ident,
                format!(
                    "no context type found in trait \
                     (there should be an associated type named `{}`)",
                    context_ty
                ),
            ));
        }

        Self { dropshot, item_trait, items, context_item }
    }

    /// Creates a `Server` for invalid metadata.
    ///
    /// In this case, no further checking is done on the items, and they are all
    /// stored as `Other`. The trait will be output as-is, with `#[endpoint]`
    /// and `#[channel]` attributes stripped.
    fn invalid_no_metadata(item_trait: &'ast ItemTraitPartParsed) -> Self {
        // Just store all the items as "Other", to indicate that we haven't
        // performed any validation on them.
        let items = item_trait.items.iter().map(ServerItem::Other);

        Self {
            // The "dropshot" token is not available, so the best we can do is
            // to use the default "dropshot".
            dropshot: get_crate(None),
            item_trait,
            items: items.collect(),
            context_item: None,
        }
    }

    fn to_output(&self) -> TokenStream {
        let context_item =
            self.make_context_trait_item().map(TraitItemPartParsed::Other);
        let other_items =
            self.items.iter().map(|item| item.to_out_trait_item());
        let out_items = context_item.into_iter().chain(other_items);

        // We need a 'static bound on the trait itself, otherwise we get `T
        // doesn't live long enough` errors.
        let mut supertraits = self.item_trait.supertraits.clone();
        supertraits.push(parse_quote!('static));

        // Everything else about the trait stays the same -- just the items change.

        let out_trait = ItemTraitPartParsed {
            supertraits,
            items: out_items.collect(),
            ..self.item_trait.clone()
        };

        // Generate two API description functions: one for the real trait, and
        // one for a stub impl meant for OpenAPI generation. Restrict the first
        // to the case where the context type is present, since the error
        // messages are quite ugly otherwise.
        let api_description =
            self.context_item.is_some().then(|| self.make_api_factory());
        let stub_api_description = self.make_stub_api();

        quote! {
            #out_trait

            // We don't need to generate type checks the way we do with
            // function-based macros, because we get error messages that are
            // exactly as good through the stub API description generator.
            //
            // For that reason, put it above the real API description as well --
            // that way, the best error messages appear first.
            #stub_api_description

            #api_description
        }
    }

    fn make_context_trait_item(&self) -> Option<syn::TraitItem> {
        let dropshot = &self.dropshot;
        let item = self.context_item?;
        let mut bounds = item.bounds.clone();
        // Generate these bounds for the associated type. We could require that
        // users specify them and error out if they don't, but this is much
        // easier to do, and also produces better errors.
        bounds.push(parse_quote!(#dropshot::ServerContext));
        bounds.push(parse_quote!('static));

        let out_item = syn::TraitItemType { bounds, ..item.clone() };
        Some(syn::TraitItem::Type(out_item))
    }

    fn make_api_factory(&self) -> TokenStream {
        let dropshot = &self.dropshot;

        let trait_name = &self.item_trait.ident;
        let trait_name_str = trait_name.to_string();
        let vis = &self.item_trait.vis;
        let fn_ident = format_ident!("{trait_name_str}_api_description");

        let body = self.make_api_factory_body(ApiFactoryKind::Regular);

        // XXX: Switch to enum TraitFactory {}
        quote! {
            #[automatically_derived]
            #vis fn #fn_ident<ServerImpl: #trait_name>() -> ::std::result::Result<
                #dropshot::ApiDescription<<ServerImpl as #trait_name>::Context>,
                #dropshot::ApiDescriptionBuildError,
            > {
                #body
            }
        }
    }

    fn make_stub_api(&self) -> TokenStream {
        let trait_name = &self.item_trait.ident;
        let trait_name_str = trait_name.to_string();
        let vis = &self.item_trait.vis;

        let fn_ident = format_ident!("{trait_name_str}_stub_api_description");

        let body = self.make_api_factory_body(ApiFactoryKind::Stub);

        // XXX: need a way to get the dropshot crate name here.
        quote! {
            #[automatically_derived]
            #vis fn #fn_ident() -> ::std::result::Result<
                dropshot::ApiDescription<dropshot::StubContext>,
                dropshot::ApiDescriptionBuildError,
            > {
                #body
            }
        }
    }

    /// Generates the body for the API factory function, as well as the stub
    /// generator function.
    ///
    /// This code is shared across the real and stub API description functions.
    fn make_api_factory_body(&self, kind: ApiFactoryKind) -> TokenStream {
        let trait_name = &self.item_trait.ident;
        let trait_name_str = trait_name.to_string();

        if self.is_invalid() {
            let err_msg = format!(
                "errors encountered while generating dropshot server `{}`",
                trait_name_str
            );
            quote! {
                panic!(#err_msg);
            }
        } else {
            let endpoints = self.items.iter().filter_map(|item| match item {
                ServerItem::Endpoint(e) => {
                    let name = &e.f.sig.ident;
                    let endpoint = match kind {
                        ApiFactoryKind::Regular => {
                            // Adding the span information to path_to_name leads
                            // to fewer call_site errors.
                            let path_to_name =
                                quote_spanned! {name.span()=> <ServerImpl as #trait_name>::#name };
                            e.to_api_endpoint(
                                &self.dropshot,
                                &ApiEndpointKind::Regular(&path_to_name),
                            )
                        }
                        ApiFactoryKind::Stub => {
                            let extractor_types =
                                e.params.extractor_types().collect();
                            let ret_ty = e.params.ret_ty;
                            e.to_api_endpoint(
                                &self.dropshot,
                                &ApiEndpointKind::Stub {
                                    name,
                                    extractor_types,
                                    ret_ty,
                                },
                            )
                        }
                    };

                    Some(endpoint)
                }

                ServerItem::Channel(_) => {
                    todo!("still need to implement channels")
                }

                ServerItem::Invalid(_) | ServerItem::Other(_) => None,
            });

            quote_spanned! {self.item_trait.ident.span()=>
                let mut dropshot_api = dropshot::ApiDescription::new();
                let mut dropshot_errors: Vec<String> = Vec::new();

                #(#endpoints)*

                if !dropshot_errors.is_empty() {
                    Err(dropshot::ApiDescriptionBuildError::new(dropshot_errors))
                } else {
                    Ok(dropshot_api)
                }
            }
        }
    }

    /// Returns true if errors were detected while generating the dropshot
    /// server, and it is invalid as a result.
    fn is_invalid(&self) -> bool {
        if self.context_item.is_none() {
            return true;
        }

        self.items.iter().any(|item| match item {
            ServerItem::Invalid(_) => true,
            _ => false,
        })
    }
}

#[derive(Clone, Copy, Debug)]
enum ApiFactoryKind {
    Regular,
    Stub,
}

fn check_endpoint_or_channel_on_non_fn(
    kind: &str,
    name: &str,
    attrs: &[syn::Attribute],
    errors: &ErrorSink<'_, Error>,
) {
    if let Some(attr) = attrs.iter().find(|a| a.path().is_ident(ENDPOINT_IDENT))
    {
        errors.push(Error::new_spanned(
            attr,
            format!("{kind} `{name}` marked as endpoint is not a method"),
        ));
    }

    if let Some(attr) = attrs.iter().find(|a| a.path().is_ident(CHANNEL_IDENT))
    {
        errors.push(Error::new_spanned(
            attr,
            format!("{kind} `{name}` marked as channel is not a method"),
        ));
    }
}

enum ServerItem<'ast> {
    Endpoint(ServerEndpoint<'ast>),
    Channel(ServerChannel<'ast>),
    // For endpoints that we couldn't parse successfully, we continue to
    // generate the underlying method because rust-analyzer works better if it
    // exists.
    Invalid(&'ast TraitItemFnForSignature),
    Other(&'ast TraitItemPartParsed),
}

impl<'a> ServerItem<'a> {
    fn to_out_trait_item(&self) -> TraitItemPartParsed {
        match self {
            Self::Endpoint(e) => e.to_out_trait_item(),
            Self::Channel(c) => c.to_out_trait_item(),
            Self::Invalid(f) => {
                // Strip recognized attributes, retaining all others.
                let mut f = (*f).clone();
                f.strip_recognized_attrs();
                TraitItemPartParsed::Fn(f)
            }
            Self::Other(o) => {
                let mut o = (*o).clone();
                o.strip_recognized_attrs();
                o
            }
        }
    }
}

fn parse_endpoint_metadata(
    name_str: &str,
    attr: &syn::Attribute,
    errors: &ErrorSink<'_, Error>,
) -> Option<ValidatedEndpointMetadata> {
    // Attempt to parse the metadata -- it must be a list.
    let l = match &attr.meta {
        syn::Meta::List(l) => l,
        _ => {
            errors.push(Error::new_spanned(
                &attr,
                format!(
                    "endpoint `{name_str}` must be of the form \
                     #[endpoint {{ method = GET, path = \"/path\", ... }}]"
                ),
            ));
            return None;
        }
    };

    match from_tokenstream::<EndpointMetadata>(&l.tokens) {
        Ok(m) => m.validate(name_str, attr, MacroKind::Trait, errors),
        Err(error) => {
            errors.push(error);
            return None;
        }
    }
}

struct ServerEndpoint<'ast> {
    f: &'ast TraitItemFnForSignature,
    metadata: ValidatedEndpointMetadata,
    params: EndpointParams<'ast>,
}

impl<'ast> ServerEndpoint<'ast> {
    /// Parses endpoint metadata to create a new `ServerEndpoint`.
    ///
    /// If the return value is None, at least one error occurred while parsing.
    fn new(
        f: &'ast TraitItemFnForSignature,
        attr: &'ast syn::Attribute,
        context_ident: &syn::Ident,
        errors: &ErrorSink<'_, Error>,
    ) -> Option<Self> {
        let name_str = f.sig.ident.to_string();

        let metadata = parse_endpoint_metadata(&name_str, attr, errors);
        let params = EndpointParams::new(
            &f.sig,
            RqctxKind::Trait { context_ident },
            errors,
        );

        match (metadata, params) {
            (Some(metadata), Some(params)) => {
                Some(Self { f, metadata, params })
            }
            // This means that something failed.
            _ => None,
        }
    }

    fn to_out_trait_item(&self) -> TraitItemPartParsed {
        // Retain all attributes other than the endpoint attribute.
        let mut f = self.f.clone();
        f.strip_recognized_attrs();

        // Below code adapted from https://github.com/rust-lang/impl-trait-utils
        // and used under the MIT and Apache 2.0 licenses.
        let output_ty = {
            let ret_ty = self.params.ret_ty;
            let bounds = parse_quote! {
                ::core::future::Future<Output = #ret_ty> + Send + 'static
            };
            syn::Type::ImplTrait(syn::TypeImplTrait {
                impl_token: Default::default(),
                bounds,
            })
        };

        // If there's a block, then surround it with `async move`, to match the
        // fact that we're going to remove `async` from the signature.
        let block = f.block.as_ref().map(|block| {
            let block = block.clone();
            let tokens = quote! { async move #block };
            UnparsedBlock { brace_token: block.brace_token, tokens }
        });

        f.sig.asyncness = None;
        f.sig.output =
            syn::ReturnType::Type(Default::default(), Box::new(output_ty));
        f.block = block;

        TraitItemPartParsed::Fn(f)
    }

    fn to_api_endpoint(
        &self,
        dropshot: &TokenStream,
        kind: &ApiEndpointKind<'_>,
    ) -> TokenStream {
        let name = &self.f.sig.ident;
        let name_str = name.to_string();

        let doc = ExtractedDoc::from_attrs(&self.f.attrs);

        let endpoint_fn =
            self.metadata.to_api_endpoint_fn(dropshot, &name_str, kind, &doc);

        // Note that we use name_str (string) rather than name (ident) here
        // because we deliberately want to lose the span information. If we
        // don't do that, then rust-analyzer will get confused and believe that
        // the name is both a method and a variable.
        //
        // Note that there isn't any possible variable name collision here,
        // since all names are prefixed with "endpoint_".
        let endpoint_name = format_ident!("endpoint_{}", name_str);

        quote_spanned! {name.span()=>
            {
                let #endpoint_name = #endpoint_fn;
                if let Err(error) = dropshot_api.register(#endpoint_name) {
                    dropshot_errors.push(error);
                }
            }
        }
    }
}

struct ServerChannel<'ast> {
    orig: &'ast TraitItemFnForSignature,
}

impl<'ast> ServerChannel<'ast> {
    fn new(
        orig: &'ast TraitItemFnForSignature,
        _cattr: &'ast syn::Attribute,
    ) -> Option<Self> {
        // TODO: implement channels
        Some(Self { orig })
    }

    fn to_out_trait_item(&self) -> TraitItemPartParsed {
        // Retain all attributes other than the channel attribute.
        let mut f = self.orig.clone();
        f.strip_recognized_attrs();
        TraitItemPartParsed::Fn(f)
    }
}

trait StripRecognizedAttrs {
    fn strip_recognized_attrs(&mut self);
}

impl StripRecognizedAttrs for TraitItemPartParsed {
    fn strip_recognized_attrs(&mut self) {
        match self {
            TraitItemPartParsed::Fn(f) => f.strip_recognized_attrs(),
            TraitItemPartParsed::Other(o) => o.strip_recognized_attrs(),
        }
    }
}

impl StripRecognizedAttrs for TraitItemFnForSignature {
    fn strip_recognized_attrs(&mut self) {
        strip_attrs_impl(&mut self.attrs);
    }
}

impl StripRecognizedAttrs for syn::TraitItem {
    fn strip_recognized_attrs(&mut self) {
        match self {
            syn::TraitItem::Const(c) => {
                strip_attrs_impl(&mut c.attrs);
            }
            syn::TraitItem::Fn(f) => {
                strip_attrs_impl(&mut f.attrs);
            }
            syn::TraitItem::Type(t) => {
                strip_attrs_impl(&mut t.attrs);
            }
            syn::TraitItem::Macro(m) => {
                strip_attrs_impl(&mut m.attrs);
            }
            _ => {}
        }
    }
}

fn strip_attrs_impl(attrs: &mut Vec<syn::Attribute>) {
    attrs.retain(|a| {
        !(a.path().is_ident(ENDPOINT_IDENT) || a.path().is_ident(CHANNEL_IDENT))
    });
}

#[cfg(test)]
mod tests {
    use expectorate::assert_contents;

    use super::*;

    #[test]
    fn test_server_basic() {
        let (item, errors) = do_server(
            quote! {},
            quote! {
                pub trait MyTrait {
                    type Context;

                    #[endpoint { method = GET, path = "/xyz" }]
                    async fn handler_xyz(
                        rqctx: RequestContext<Self::Context>,
                    ) -> Result<HttpResponseOk<()>, HttpError>;
                }
            },
        )
        .unwrap();

        assert!(errors.is_empty());
        assert_contents(
            "tests/output/server_basic.rs",
            &prettyplease::unparse(&parse_quote! { #item }),
        );
    }
}
