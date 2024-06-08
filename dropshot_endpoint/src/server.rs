// Copyright 2024 Oxide Computer Company

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
            let server = Server::invalid_no_metadata(&item_trait, &errors);
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
    module: Option<String>,
    _dropshot_crate: Option<String>,
}

impl ServerMetadata {
    /// The default name for the associated context type: `Self::Context`.
    const DEFAULT_CONTEXT_NAME: &'static str = "Context";

    /// Returns the dropshot crate value as a TokenStream.
    fn dropshot_crate(&self) -> TokenStream {
        get_crate(self._dropshot_crate.as_deref())
    }

    fn context_name(&self) -> &str {
        self.context.as_deref().unwrap_or(Self::DEFAULT_CONTEXT_NAME)
    }

    fn module_name(&self, trait_ident: &syn::Ident) -> String {
        self.module
            .clone()
            .unwrap_or_else(|| to_snake_case(&trait_ident.to_string()))
    }
}

fn to_snake_case(s: &str) -> String {
    let mut ret = String::new();
    for (i, ch) in s.char_indices() {
        if i > 0 && ch.is_uppercase() {
            ret.push('_');
        }
        ret.push(ch.to_ascii_lowercase());
    }

    ret
}

struct Server<'ast> {
    dropshot: TokenStream,
    item_trait: ServerItemTrait<'ast>,
    // We want to maintain the order of items in the trait (other than the
    // Context associated type which we're always going to move to the top), so
    // we use a single list to store all of them.
    items: Vec<ServerItem<'ast>>,

    context_item: ContextItem<'ast>,

    // None indicates invalid metadata.
    module_ident: Option<syn::Ident>,
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

        // First, validate the top-level properties of the trait itself.
        let trait_ident = &item_trait.ident;
        let item_trait = ServerItemTrait::new(item_trait, &errors);

        let context_name = metadata.context_name();

        // Do a first pass to look for the context item. We could forgo this
        // pass and always generate a context ident from the server metadata,
        // but that would lose valuable span information.
        let mut context_item = None;
        for item in &item_trait.item.items {
            if let TraitItemPartParsed::Other(syn::TraitItem::Type(ty)) = item {
                if ty.ident == context_name {
                    // This is the context item.
                    context_item = Some(ContextItem::new(ty, &errors));
                    break;
                }
            }
        }

        let context_item = if let Some(context_item) = context_item {
            context_item
        } else {
            ContextItem::new_missing(context_name, trait_ident, &errors)
        };

        let module_ident =
            format_ident!("{}", metadata.module_name(trait_ident));

        for item in &item_trait.item.items {
            match item {
                // Functions need to be parsed to see if they are endpoints or
                // channels.
                TraitItemPartParsed::Fn(f) => {
                    items.push(ServerItem::Fn(ServerFnItem::new(
                        f,
                        trait_ident,
                        context_item.ident(),
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
                            t.ident != context_name
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

        Self {
            dropshot,
            item_trait,
            items,
            context_item,
            module_ident: Some(module_ident),
        }
    }

    /// Creates a `Server` for invalid metadata.
    ///
    /// In this case, no further checking is done on the items, and they are all
    /// stored as `Other`. The trait will be output as-is, with `#[endpoint]`
    /// and `#[channel]` attributes stripped.
    fn invalid_no_metadata(
        item_trait: &'ast ItemTraitPartParsed,
        errors: &ErrorSink<'_, Error>,
    ) -> Self {
        // Validate top-level properties of the trait itself.
        let item_trait = ServerItemTrait::new(item_trait, errors);

        // Just store all the items as "Other", to indicate that we haven't
        // performed any validation on them.
        let items = item_trait.item.items.iter().map(ServerItem::Other);

        Self {
            // The "dropshot" token is not available, so the best we can do is
            // to use the default "dropshot".
            dropshot: get_crate(None),
            item_trait,
            items: items.collect(),
            context_item: ContextItem::new_invalid_metadata(),
            module_ident: None,
        }
    }

    fn to_output(&self) -> TokenStream {
        let context_item =
            self.make_context_trait_item().map(TraitItemPartParsed::Other);
        let other_items =
            self.items.iter().map(|item| item.to_out_trait_item());
        let out_items = context_item.into_iter().chain(other_items);

        // Output the trait whether or not it is valid.
        let item_trait = self.item_trait.item;

        // We need a 'static bound on the trait itself, otherwise we get `T
        // doesn't live long enough` errors.
        let mut supertraits = item_trait.supertraits.clone();
        supertraits.push(parse_quote!('static));

        // Everything else about the trait stays the same -- just the items change.
        let out_trait = ItemTraitPartParsed {
            supertraits,
            items: out_items.collect(),
            ..item_trait.clone()
        };

        // Also generate the support module.
        let module = self.make_module();

        quote! {
            #out_trait

            #module
        }
    }

    fn make_context_trait_item(&self) -> Option<syn::TraitItem> {
        let dropshot = &self.dropshot;
        // In this context, invalid items should be passed through as-is.
        let item = self.context_item.original_item()?;
        let mut bounds = item.bounds.clone();
        // Generate these bounds for the associated type. We could require that
        // users specify them and error out if they don't, but this is much
        // easier to do, and also produces better errors.
        bounds.push(parse_quote!(#dropshot::ServerContext));

        let out_item = syn::TraitItemType { bounds, ..item.clone() };
        Some(syn::TraitItem::Type(out_item))
    }

    /// Generate the support module corresponding to the trait, with ways to
    /// make real servers and stub API descriptions.
    fn make_module(&self) -> TokenStream {
        let item_trait = self.item_trait.valid_item();
        let context_item = self.context_item.valid_item();
        let module_ident = self.module_ident.as_ref();

        match (item_trait, context_item, module_ident) {
            (Some(item_trait), Some(context_item), Some(module_ident)) => {
                // Only generate the full support module if the trait and the
                // context item are valid, and the module ident is available. If
                // we don't check for these, the error messages become quite
                // ugly.
                let trait_ident = &item_trait.ident;
                let vis = &item_trait.vis;

                let doc_comments = ModuleDocComments::generate(
                    &self.dropshot,
                    trait_ident,
                    &context_item.ident,
                    module_ident,
                );

                // Generate two API description functions: one for the real trait, and
                // one for a stub impl meant for OpenAPI generation.
                let api = self.make_api_description(
                    &context_item.ident,
                    doc_comments.api_description(),
                );
                let stub_api = self.make_stub_api_description(
                    doc_comments.stub_api_description(),
                );
                let outer = doc_comments.outer();

                quote! {
                    #outer
                    #[automatically_derived]
                    #vis mod #module_ident {
                        use super::*;

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
                }
            }
            (_, _, Some(module_ident)) => {
                // If the module ident is known but one of the other bits is
                // invalid, then generate an empty support module.
                let doc = invalid_module_doc(&self.item_trait.item.ident);
                let vis = &self.item_trait.item.vis;

                quote! {
                    #doc
                    #vis mod #module_ident {}
                }
            }
            _ => {
                // Can't do anything if the module name is missing.
                quote! {}
            }
        }
    }

    fn make_api_description(
        &self,
        context_ident: &syn::Ident,
        doc: TokenStream,
    ) -> TokenStream {
        let dropshot = &self.dropshot;
        let trait_ident = &self.item_trait.item.ident;
        let vis = &self.item_trait.item.vis;

        let body = self.make_api_factory_body(FactoryKind::Regular);

        quote! {
            #doc
            #[automatically_derived]
            #vis fn api_description<ServerImpl: #trait_ident>() -> ::std::result::Result<
                #dropshot::ApiDescription<<ServerImpl as #trait_ident>::#context_ident>,
                #dropshot::ApiDescriptionBuildError,
            > {
                #body
            }
        }
    }

    fn make_stub_api_description(&self, doc: TokenStream) -> TokenStream {
        let dropshot = &self.dropshot;
        let vis = &self.item_trait.item.vis;

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
        let trait_ident = &self.item_trait.item.ident;
        let trait_ident_str = trait_ident.to_string();

        if self.is_invalid() {
            let err_msg = format!(
                "errors encountered while generating dropshot server `{}`",
                trait_ident_str
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
                            let path_to_name: TokenStream =
                                quote_spanned! {e.attr.span()=> <ServerImpl as #trait_ident>::#name };
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
                                    attr: &e.attr,
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

            quote! {
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
        if !self.item_trait.is_valid {
            return true;
        }

        if !self.context_item.is_valid() {
            return true;
        }

        self.items.iter().any(|item| match item {
            ServerItem::Fn(ServerFnItem::Invalid(_)) => true,
            _ => false,
        })
    }
}

#[derive(Clone, Copy)]
struct ServerItemTrait<'ast> {
    item: &'ast ItemTraitPartParsed,
    is_valid: bool,
}

impl<'ast> ServerItemTrait<'ast> {
    fn new(
        item: &'ast ItemTraitPartParsed,
        errors: &ErrorSink<'_, Error>,
    ) -> Self {
        let trait_ident = &item.ident;
        let errors = errors.new();

        if item.unsafety.is_some() {
            errors.push(Error::new_spanned(
                &item.unsafety,
                format!(
                    "server `{trait_ident}` must not be marked as `unsafe`"
                ),
            ));
        }

        if item.auto_token.is_some() {
            errors.push(Error::new_spanned(
                &item.auto_token,
                format!("server `{trait_ident}` must not be an auto trait"),
            ));
        }

        if !item.generics.params.is_empty() {
            errors.push(Error::new_spanned(
                &item.generics,
                format!("server `{trait_ident}` must not have generics"),
            ));
        }

        if let Some(where_clause) = &item.generics.where_clause {
            // Empty where clauses are no-ops and therefore permitted.
            if !where_clause.predicates.is_empty() {
                errors.push(Error::new_spanned(
                    where_clause,
                    format!(
                        "server `{trait_ident}` must not have a where clause"
                    ),
                ));
            }
        }

        Self { item, is_valid: !errors.has_errors() }
    }

    fn valid_item(&self) -> Option<&'ast ItemTraitPartParsed> {
        self.is_valid.then_some(self.item)
    }
}

/// The context item as present within a server trait (or not).
#[derive(Clone, Debug)]
enum ContextItem<'ast> {
    /// The item is valid.
    Valid(&'ast syn::TraitItemType),

    /// The item is invalid but present.
    Invalid(&'ast syn::TraitItemType),

    /// The item is missing.
    Missing {
        /// This is a conjured-up ident for the missing item.
        ident: syn::Ident,
    },
}

impl<'ast> ContextItem<'ast> {
    fn new(
        ty: &'ast syn::TraitItemType,
        errors: &ErrorSink<'_, Error>,
    ) -> Self {
        let errors = errors.new();

        // The context type must not have generics.
        if !ty.generics.params.is_empty() {
            errors.push(Error::new_spanned(
                &ty.generics,
                format!("context type `{}` must not have generics", ty.ident),
            ));
        }

        // Don't return the type if there were errors.
        if errors.has_errors() {
            Self::Invalid(ty)
        } else {
            Self::Valid(ty)
        }
    }

    fn new_missing(
        context_name: &str,
        trait_ident: &syn::Ident,
        errors: &ErrorSink<'_, Error>,
    ) -> Self {
        errors.push(Error::new_spanned(
            &trait_ident,
            format!(
                "server `{trait_ident}` does not have associated type \
                `{context_name}`\n\
                 (this type specifies the shared context for endpoints)",
            ),
        ));

        Self::Missing { ident: format_ident!("{context_name}") }
    }

    fn new_invalid_metadata() -> Self {
        Self::Missing {
            ident: format_ident!("{}", ServerMetadata::DEFAULT_CONTEXT_NAME),
        }
    }

    fn ident(&self) -> &syn::Ident {
        match self {
            Self::Valid(ty) => &ty.ident,
            Self::Invalid(ty) => &ty.ident,
            Self::Missing { ident } => ident,
        }
    }

    /// Return the original item, or None if it is missing.
    fn original_item(&self) -> Option<&'ast syn::TraitItemType> {
        match self {
            Self::Valid(ty) | Self::Invalid(ty) => Some(ty),
            Self::Missing { .. } => None,
        }
    }

    /// Return a valid item, or None if the item is invalid or missing.
    fn valid_item(&self) -> Option<&'ast syn::TraitItemType> {
        match self {
            Self::Valid(ty) => Some(ty),
            Self::Invalid(_) | Self::Missing { .. } => None,
        }
    }

    fn is_valid(&self) -> bool {
        matches!(self, Self::Valid(_))
    }
}

/// Generated documentation comments for the support module.
struct ModuleDocComments {
    /// Doc comment for the module type.
    outer: String,

    /// Doc comment for the API description factory.
    api_description: String,

    /// Doc comment for the stub API description function.
    stub_api_description: String,
}

impl ModuleDocComments {
    fn generate(
        dropshot: &TokenStream,
        trait_ident: &syn::Ident,
        context_ident: &syn::Ident,
        module_ident: &syn::Ident,
    ) -> ModuleDocComments {
        let outer = format!(
            "Support module for the Dropshot server trait \
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
    let description = {module_ident}::api_description::<{trait_ident}Impl>().unwrap();

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
    let stub = {module_ident}::stub_api_description().unwrap();

    // Generate OpenAPI spec from `stub`.
    let spec = stub.openapi(\"{trait_ident}\", \"0.1.0\");
    spec.write(&mut std::io::stdout()).unwrap();
}}
```

[`{trait_ident}`]: {trait_ident}
[`api_description`]: {module_ident}::api_description
[`ApiDescription`]: {dropshot}::ApiDescription
[`StubContext`]: {dropshot}::StubContext
");

        ModuleDocComments { outer, api_description, stub_api_description }
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

fn invalid_module_doc(trait_ident: &syn::Ident) -> TokenStream {
    let outer = format!(
"Support module for the **invalid** Dropshot server trait `{trait_ident}`.

Errors were encountered while generating the server, so this module is left empty.
",
    );

    string_to_doc_attrs(&outer)
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
        trait_ident: &syn::Ident,
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
                if let Some(endpoint) = ServerEndpoint::new(
                    f,
                    eattr,
                    trait_ident,
                    context_ident,
                    errors,
                ) {
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
    attr: &'ast syn::Attribute,
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
        trait_ident: &syn::Ident,
        context_ident: &syn::Ident,
        errors: &ErrorSink<'_, Error>,
    ) -> Option<Self> {
        let name_str = f.sig.ident.to_string();

        let metadata = parse_endpoint_metadata(&name_str, attr, errors);
        let params = EndpointParams::new(
            &f.sig,
            RqctxKind::Trait { trait_ident, context_ident },
            errors,
        );

        match (metadata, params) {
            (Some(metadata), Some(params)) => {
                Some(Self { f, attr, metadata, params })
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

        quote_spanned! {self.attr.span()=>
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
        // Provide custom parameters and ensure that the original ones are
        // not used.
        let (item, errors) = do_server(
            quote! {
                context = Situation,
                module = my_support_module,
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
            "tests/output/server_with_custom_params.rs",
            &prettyplease::unparse(&file),
        );

        // Check banned identifiers.
        let banned =
            [ServerMetadata::DEFAULT_CONTEXT_NAME, DROPSHOT, "my_trait"];
        assert_banned_idents(&file, banned);
    }

    // Test output for a server with no endpoints.
    #[test]
    fn test_server_no_endpoints() {
        let (item, errors) = do_server(
            quote! {},
            quote! {
                pub trait MyTrait {
                    type Context;
                }
            },
        );

        assert!(errors.is_empty());
        assert_contents(
            "tests/output/server_no_endpoints.rs",
            &prettyplease::unparse(&parse_quote! { #item }),
        );
    }
}
