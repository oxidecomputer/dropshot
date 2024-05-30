// Copyright 2023 Oxide Computer Company

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned, ToTokens};
use serde::Deserialize;
use serde_tokenstream::from_tokenstream;
use syn::{parse_quote, parse_quote_spanned, spanned::Spanned, Error};

use crate::{
    doc::{string_to_doc_attrs, ExtractedDoc},
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
    factory: Option<String>,
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

    fn factory_ty(&self, trait_ident: &syn::Ident) -> String {
        self.factory.clone().unwrap_or_else(|| format!("{trait_ident}Factory"))
    }
}

struct Server<'ast> {
    dropshot: TokenStream,
    item_trait: &'ast ItemTraitPartParsed,
    // We want to maintain the order of items in the trait (other than the
    // Context associated type which we're always going to move to the top), so
    // we use a single list to store all of them.
    items: Vec<ServerItem<'ast>>,

    context_ident: syn::Ident,
    // This is None if there is no context type, which is an error.
    context_item: Option<&'ast syn::TraitItemType>,

    // None indicates invalid metadata.
    factory_ident: Option<syn::Ident>,
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

        // Do a first pass to look for the context type. We could forgo this
        // pass and always generate a context ident from context_ty, but that
        // would lose valuable span information.
        let mut context_item = None;
        for item in &item_trait.items {
            if let TraitItemPartParsed::Other(syn::TraitItem::Type(t)) = item {
                if t.ident == context_ty {
                    // This is the context type.
                    context_item = Some(t);
                    break;
                }
            }
        }

        let context_ident = context_item.map_or_else(
            || {
                // In this case, conjure up a context type. We can't provide span
                // info.
                format_ident!("{}", context_ty)
            },
            |t| t.ident.clone(),
        );

        let factory_ident =
            format_ident!("{}", metadata.factory_ty(&item_trait.ident));

        for item in &item_trait.items {
            match item {
                // Functions need to be parsed to see if they are endpoints or
                // channels.
                TraitItemPartParsed::Fn(f) => {
                    items.push(ServerItem::Fn(ServerFnItem::new(
                        f,
                        &context_ident,
                        &errors,
                    )));
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

                            // We'll handle the context type separately.
                            t.ident != context_ty
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

        Self {
            dropshot,
            item_trait,
            items,
            context_ident,
            context_item,
            factory_ident: Some(factory_ident),
        }
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
            context_ident: format_ident!("Context"),
            context_item: None,
            factory_ident: None,
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

        // Also generate the factory.
        let factory = self.make_factory();

        quote! {
            #out_trait

            #factory
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

        let out_item = syn::TraitItemType { bounds, ..item.clone() };
        Some(syn::TraitItem::Type(out_item))
    }

    /// Generate a factory corresponding to the trait, with ways to make real
    /// servers and stub API descriptions.
    fn make_factory(&self) -> Option<TokenStream> {
        // Only generate a factory if the context and factory types are both
        // known, otherwise the error messages get quite ugly.
        self.context_item?;
        let factory_ident = self.factory_ident.as_ref()?;
        let vis = &self.item_trait.vis;

        let doc_comments = FactoryDocComments::generate(
            &self.dropshot,
            &self.item_trait.ident,
            &self.context_ident,
            factory_ident,
        );

        // Generate two API description functions: one for the real trait, and
        // one for a stub impl meant for OpenAPI generation.
        let api = self.make_api_description(doc_comments.api_description());
        let stub_api =
            self.make_stub_api_description(doc_comments.stub_api_description());
        let outer = doc_comments.outer();

        let ret = quote! {
            #outer
            #[derive(Copy, Clone, Debug)]
            #[automatically_derived]
            #vis enum #factory_ident {}

            impl #factory_ident {
                // We don't need to generate type checks the way we do with
                // function-based macros, because we get error messages that are
                // roughly as good through the stub API description generator.
                // (Also, adding type checks would end up duplicating a ton of error
                // messages.)
                //
                // For that reason, put it above the real API description -- that
                // way, the best error messages appear first.
                #stub_api

                #api
            }
        };
        Some(ret)
    }

    fn make_api_description(&self, doc: TokenStream) -> TokenStream {
        let dropshot = &self.dropshot;
        let trait_name = &self.item_trait.ident;
        let vis = &self.item_trait.vis;

        let body = self.make_api_factory_body(FactoryKind::Regular);
        let context_ident = &self.context_ident;

        quote! {
            #doc
            #[automatically_derived]
            #vis fn api_description<ServerImpl: #trait_name>() -> ::std::result::Result<
                #dropshot::ApiDescription<<ServerImpl as #trait_name>::#context_ident>,
                #dropshot::ApiDescriptionBuildError,
            > {
                #body
            }
        }
    }

    fn make_stub_api_description(&self, doc: TokenStream) -> TokenStream {
        let dropshot = &self.dropshot;
        let vis = &self.item_trait.vis;

        let body = self.make_api_factory_body(FactoryKind::Stub);

        quote! {
            #doc
            #[automatically_derived]
            #vis fn stub_api_description() -> ::std::result::Result<
                #dropshot::ApiDescription<#dropshot::StubContext>,
                #dropshot::ApiDescriptionBuildError,
            > {
                #body
            }
        }
    }

    /// Generates the body for the API factory function, as well as the stub
    /// generator function.
    ///
    /// This code is shared across the real and stub API description functions.
    fn make_api_factory_body(&self, kind: FactoryKind) -> TokenStream {
        let dropshot = &self.dropshot;
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
                ServerItem::Fn(ServerFnItem::Endpoint(e)) => {
                    let name = &e.f.sig.ident;
                    let endpoint = match kind {
                        FactoryKind::Regular => {
                            // Adding the span information to path_to_name leads
                            // to fewer call_site errors.
                            let path_to_name =
                                quote_spanned! {name.span()=> <ServerImpl as #trait_name>::#name };
                            e.to_api_endpoint(
                                &self.dropshot,
                                &ApiEndpointKind::Regular(&path_to_name),
                            )
                        }
                        FactoryKind::Stub => {
                            let extractor_types =
                                e.params.extractor_types().collect();
                            let ret_ty = e.params.ret_ty;
                            e.to_api_endpoint(
                                dropshot,
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

                ServerItem::Fn(ServerFnItem::Channel(_)) => {
                    todo!("still need to implement channels")
                }

                ServerItem::Fn(ServerFnItem::Invalid(_)) | ServerItem::Fn(ServerFnItem::NonEndpoint(_)) | ServerItem::Other(_) => None,
            });

            quote_spanned! {self.item_trait.ident.span()=>
                let mut dropshot_api = #dropshot::ApiDescription::new();
                let mut dropshot_errors: Vec<String> = Vec::new();

                #(#endpoints)*

                if !dropshot_errors.is_empty() {
                    Err(#dropshot::ApiDescriptionBuildError::new(dropshot_errors))
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
            ServerItem::Fn(ServerFnItem::Invalid(_)) => true,
            _ => false,
        })
    }
}

/// Generated documentation comments for an API factory.
struct FactoryDocComments {
    /// Doc comment for the factory type.
    outer: String,

    /// Doc comment for the API description function.
    api_description: String,

    /// Doc comment for the stub API description function.
    stub_api_description: String,
}

impl FactoryDocComments {
    fn generate(
        dropshot: &TokenStream,
        trait_ident: &syn::Ident,
        context_ident: &syn::Ident,
        factory_ident: &syn::Ident,
    ) -> FactoryDocComments {
        let outer = format!(
            "API description factory for the Dropshot server trait \
            [`{trait_ident}`]({trait_ident}).",
        );

        let api_description = format!(
"Given an implementation of [`{trait_ident}`], generate an API description.

This function accepts a single type argument `ServerImpl`, turning it into a
Dropshot [`ApiDescription`]`<ServerImpl::`[`{context_ident}`]`>`. 
The returned `ApiDescription` can then be turned into a Dropshot server that
accepts a concrete `{context_ident}`.

## Example

```rust,ignore
/// A type used to define the concrete implementation for `{trait_ident}`.
/// 
/// This type is never constructed -- it is just a place to define your
/// implementation of `{trait_ident}`.
enum {trait_ident}Impl {{}}

impl {trait_ident} for {trait_ident}Impl {{
    type {context_ident} = /* context type */;

    // ... trait methods
}}

#[tokio::main]
async fn main() {{
    // Generate the description for `{trait_ident}Impl`.
    let description = {factory_ident}::api_description::<{trait_ident}Impl>().unwrap();

    // Create a value of the concrete context type.
    let context = /* some value of type `{trait_ident}Impl::{context_ident}` */;

    // Create a Dropshot server from the description.
    let config = dropshot::ConfigDropshot::default();
    let log = /* ... */;
    let server = dropshot::HttpServerStarter::new(
        &config,
        description,
        context,
        &log,
    ).unwrap();

    // Run the server.
    server.start().await
}}
```

[`ApiDescription`]: {dropshot}::ApiDescription
[`{trait_ident}`]: {trait_ident}
[`{context_ident}`]: {trait_ident}::{context_ident}
",
        );

        let stub_api_description = format!(
"Generate a _stub_ API description for [`{trait_ident}`], meant for OpenAPI
generation.

Unlike [`api_description`], this function does not require an implementation
of [`{trait_ident}`] to be available, instead generating handlers that panic.
The return value is of type [`ApiDescription`]`<`[`StubContext`]`>`.

The main use of this function is in cases where [`{trait_ident}`] is defined
in a separate crate from its implementation. The OpenAPI spec can then be
generated directly from the stub API description.

## Example

A function that prints the OpenAPI spec to standard output:

```rust,ignore
fn print_openapi_spec() {{
    let stub = {factory_ident}::stub_api_description().unwrap();

    // Generate OpenAPI spec from `stub`.
    let spec = stub.openapi(\"{trait_ident}\", \"0.1.0\");
    spec.write(&mut std::io::stdout()).unwrap();
}}
```

[`{trait_ident}`]: {trait_ident}
[`api_description`]: {factory_ident}::api_description
[`ApiDescription`]: {dropshot}::ApiDescription
[`StubContext`]: {dropshot}::StubContext
");

        FactoryDocComments { outer, api_description, stub_api_description }
    }

    fn outer(&self) -> TokenStream {
        string_to_doc_attrs(&self.outer)
    }

    fn api_description(&self) -> TokenStream {
        string_to_doc_attrs(&self.api_description)
    }

    fn stub_api_description(&self) -> TokenStream {
        string_to_doc_attrs(&self.stub_api_description)
    }
}

#[derive(Clone, Copy, Debug)]
enum FactoryKind {
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
    Fn(ServerFnItem<'ast>),
    Other(&'ast TraitItemPartParsed),
}

impl ServerItem<'_> {
    fn to_out_trait_item(&self) -> TraitItemPartParsed {
        match self {
            Self::Fn(f) => TraitItemPartParsed::Fn(f.to_out_trait_item()),
            Self::Other(&ref o) => {
                let mut o = o.clone();
                o.strip_recognized_attrs();
                o
            }
        }
    }
}

enum ServerFnItem<'ast> {
    Endpoint(ServerEndpoint<'ast>),
    Channel(ServerChannel<'ast>),
    // An endpoint item that was somehow invalid.
    Invalid(&'ast TraitItemFnForSignature),
    // A non-endpoint item not managed by the macro.
    NonEndpoint(&'ast TraitItemFnForSignature),
}

impl<'ast> ServerFnItem<'ast> {
    fn new(
        f: &'ast TraitItemFnForSignature,
        context_ident: &syn::Ident,
        errors: &ErrorSink<'_, Error>,
    ) -> Self {
        // We must have zero or one endpoint or channel attributes.
        let attrs = f
            .attrs
            .iter()
            .filter_map(|attr| {
                if attr.path().is_ident(ENDPOINT_IDENT) {
                    Some(ServerAttr::Endpoint(attr))
                } else if attr.path().is_ident(CHANNEL_IDENT) {
                    Some(ServerAttr::Channel(attr))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        match attrs.as_slice() {
            [] => {
                // This is just a normal method.
                Self::NonEndpoint(f)
            }
            [ServerAttr::Endpoint(eattr)] => {
                if let Some(endpoint) =
                    ServerEndpoint::new(f, eattr, context_ident, errors)
                {
                    Self::Endpoint(endpoint)
                } else {
                    Self::Invalid(f)
                }
            }
            [ServerAttr::Channel(cattr)] => {
                if let Some(channel) = ServerChannel::new(f, cattr) {
                    Self::Channel(channel)
                } else {
                    Self::Invalid(f)
                }
            }
            [first, rest @ ..] => {
                // We must have exactly one endpoint or channel attribute, so
                // this is an error. Produce errors for all the rest of the
                // attrs.
                let name = &f.sig.ident;

                for attr in rest {
                    let msg = match (first, attr) {
                        (ServerAttr::Endpoint(_), ServerAttr::Endpoint(_)) => {
                            format!("method `{name}` marked as endpoint multiple times")
                        }
                        (ServerAttr::Channel(_), ServerAttr::Channel(_)) => {
                            format!("method `{name}` marked as channel multiple times")
                        }
                        _ => {
                            format!("method `{name}` marked as both endpoint and channel")
                        }
                    };

                    errors.push(Error::new_spanned(attr, msg));
                }

                Self::Invalid(f)
            }
        }
    }

    fn to_out_trait_item(&self) -> TraitItemFnForSignature {
        match self {
            Self::Endpoint(e) => e.to_out_trait_item(),
            Self::Channel(c) => c.to_out_trait_item(),
            Self::Invalid(f) => {
                // Strip recognized attributes, retaining all others.
                let mut f = (*f).clone();
                f.strip_recognized_attrs();
                f
            }
            Self::NonEndpoint(&ref f) => {
                let mut f = f.clone();
                f.strip_recognized_attrs();
                f
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

    fn to_out_trait_item(&self) -> TraitItemFnForSignature {
        // Retain all attributes other than the endpoint attribute.
        let mut f = self.f.clone();
        f.strip_recognized_attrs();

        // Below code adapted from https://github.com/rust-lang/impl-trait-utils
        // and used under the MIT and Apache 2.0 licenses.
        let output_ty = {
            let ret_ty = self.params.ret_ty;
            let bounds = parse_quote_spanned! {ret_ty.span()=>
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
            let tokens = quote_spanned! {block.span()=> async move #block };
            UnparsedBlock { brace_token: block.brace_token, tokens }
        });

        f.sig.asyncness = None;
        f.sig.output =
            syn::ReturnType::Type(Default::default(), Box::new(output_ty));
        f.block = block;

        f
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

    fn to_out_trait_item(&self) -> TraitItemFnForSignature {
        // Retain all attributes other than the channel attribute.
        let mut f = self.orig.clone();
        f.strip_recognized_attrs();
        f
    }
}

enum ServerAttr<'ast> {
    Endpoint(&'ast syn::Attribute),
    Channel(&'ast syn::Attribute),
}

impl ToTokens for ServerAttr<'_> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            ServerAttr::Endpoint(attr) => attr.to_tokens(tokens),
            ServerAttr::Channel(attr) => attr.to_tokens(tokens),
        }
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

    use crate::{test_util::assert_banned_idents, util::DROPSHOT};

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
        );

        assert!(errors.is_empty());
        assert_contents(
            "tests/output/server_basic.rs",
            &prettyplease::unparse(&parse_quote! { #item }),
        );
    }

    #[test]
    fn test_server_with_custom_params() {
        // Provide custom parameters and ensure that they are used.
        let (item, errors) = do_server(
            quote! {
                context = Situation,
                _dropshot_crate = "topspin",
            },
            quote! {
                pub trait MyTrait {
                    type Situation;

                    #[endpoint { method = GET, path = "/xyz" }]
                    async fn handler_xyz(
                        rqctx: RequestContext<Self::Situation>,
                    ) -> Result<HttpResponseOk<()>, HttpError>;
                }
            },
        );

        assert!(errors.is_empty());

        let file = parse_quote! { #item };
        // Write out the file before checking it, so that we can see what it
        // looks like.
        assert_contents(
            "tests/output/server_with_custom_context.rs",
            &prettyplease::unparse(&file),
        );

        // Check banned identifiers.
        let banned = [ServerMetadata::DEFAULT_CONTEXT_TY, DROPSHOT];
        assert_banned_idents(&file, banned);
    }
}
