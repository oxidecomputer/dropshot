// Copyright 2024 Oxide Computer Company

//! Support for the `#[dropshot::api_description]` attribute macro.
//!
//! The server macro tends to follow the same structure as `endpoint.rs`, but is
//! overall quite a bit more complex than function-based macros. The main source
//! of complexity comes from having to parse the entire trait which consists of
//! many endpoints and unmanaged items.
//!
//! * Each endpoint item is parsed and validated separately, and it's unhelpful
//!   to just bail out on the first error -- hence we use a hierarchical error
//!   collection scheme.
//! * There are some overall constraints on the trait itself, such as not having
//!   generics or where clauses.
//! * Syntax errors within endpoint implementations are passed through as-is,
//!   through lazy parsing with `crate::syn_parsing::ItemTraitPartParsed`.
//! * If the trait is valid, a support module is also generated. To generate the
//!   functions inside this module, we need to mutate the `RequestContext` type
//!   in particular.
//!
//! Code that is common to both the server and endpoint macros lives in
//! `endpoint.rs`.

use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned, ToTokens};
use serde::Deserialize;
use serde_tokenstream::{from_tokenstream, from_tokenstream_spanned};
use syn::{parse_quote, parse_quote_spanned, spanned::Spanned, Error};

use crate::{
    channel::ChannelParams,
    doc::{string_to_doc_attrs, ExtractedDoc},
    endpoint::EndpointParams,
    error_store::{ErrorSink, ErrorStore},
    metadata::{
        ApiEndpointKind, ChannelMetadata, EndpointMetadata,
        ValidatedChannelMetadata, ValidatedEndpointMetadata,
    },
    params::RqctxKind,
    syn_parsing::{
        ItemTraitPartParsed, TraitItemFnForSignature, TraitItemPartParsed,
        UnparsedBlock,
    },
    util::{get_crate, MacroKind},
};

pub(crate) fn do_trait(
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> (proc_macro2::TokenStream, Vec<Error>) {
    let mut error_store = ErrorStore::new();
    let errors = error_store.sink();

    // Parse attributes. (Do this before parsing the trait since that's the
    // order they're in, in source code.)
    let api_metadata = match from_tokenstream::<ApiMetadata>(&attr) {
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

    let output = match (api_metadata, item_trait.as_ref()) {
        (Some(api_metadata), Some(item_trait)) => {
            // The happy path.
            let parser = ApiParser::new(api_metadata, &item_trait, errors);
            parser.to_output()
        }
        (None, Some(item_trait)) => {
            // This is a case where we can do something useful. Don't try and
            // validate the input, but we can at least regenerate the same type
            // with endpoint and channel attributes stripped.
            let parser = ApiParser::invalid_no_metadata(&item_trait, &errors);
            parser.to_output()
        }
        (_, None) => {
            // Can't do anything here, just return errors.
            ApiOutput {
                output: quote! {},
                context: "Self::Context".to_string(),
                has_endpoint_param_errors: false,
                has_channel_param_errors: false,
            }
        }
    };

    let mut errors = error_store.into_inner();

    // If there are any errors, we also want to provide a usage message as an error.
    if output.has_endpoint_param_errors {
        let item_trait = item_trait
            .as_ref()
            .expect("has_endpoint_param_errors is true => item_fn is Some");
        errors.insert(
            0,
            Error::new_spanned(
                &item_trait.ident,
                crate::endpoint::usage_str(&output.context),
            ),
        );
    }
    if output.has_channel_param_errors {
        let item_trait = item_trait
            .as_ref()
            .expect("has_channel_param_errors is true => item_fn is Some");
        errors.insert(
            0,
            Error::new_spanned(
                &item_trait.ident,
                crate::channel::usage_str(&output.context),
            ),
        );
    }

    (output.output, errors)
}

#[derive(Deserialize, Debug)]
struct ApiMetadata {
    #[serde(default)]
    context: Option<String>,
    module: Option<String>,
    _dropshot_crate: Option<String>,
}

impl ApiMetadata {
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
            .unwrap_or_else(|| trait_ident.to_string().to_snake_case())
    }
}

struct ApiParser<'ast> {
    dropshot: TokenStream,
    item_trait: ApiItemTrait<'ast>,
    // We want to maintain the order of items in the trait (other than the
    // Context associated type which we're always going to move to the top), so
    // we use a single list to store all of them.
    items: Vec<ApiItem<'ast>>,

    context_item: ContextItem<'ast>,

    // None indicates invalid metadata.
    module_ident: Option<syn::Ident>,
}

const ENDPOINT_IDENT: &str = "endpoint";
const CHANNEL_IDENT: &str = "channel";

impl<'ast> ApiParser<'ast> {
    fn new(
        metadata: ApiMetadata,
        item_trait: &'ast ItemTraitPartParsed,
        errors: ErrorSink<'_, Error>,
    ) -> Self {
        let dropshot = metadata.dropshot_crate();
        let mut items = Vec::with_capacity(item_trait.items.len());

        // First, validate the top-level properties of the trait itself.
        let trait_ident = &item_trait.ident;
        let item_trait = ApiItemTrait::new(item_trait, &errors);

        let context_name = metadata.context_name();

        // Do a first pass to look for the context item. We could forgo this
        // pass and always generate a context ident from the API metadata, but
        // that would lose valuable span information.
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
                    items.push(ApiItem::Fn(ApiFnItem::new(
                        &dropshot,
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
                        items.push(ApiItem::Other(item));
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

    /// Creates an `ApiParser` for invalid metadata.
    ///
    /// In this case, no further checking is done on the items, and they are all
    /// stored as `Other`. The trait will be output as-is, with `#[endpoint]`
    /// and `#[channel]` attributes stripped.
    fn invalid_no_metadata(
        item_trait: &'ast ItemTraitPartParsed,
        errors: &ErrorSink<'_, Error>,
    ) -> Self {
        // Validate top-level properties of the trait itself.
        let item_trait = ApiItemTrait::new(item_trait, errors);

        // Just store all the items as "Other", to indicate that we haven't
        // performed any validation on them.
        let items = item_trait.item.items.iter().map(ApiItem::Other);

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

    fn to_output(&self) -> ApiOutput {
        let context = format!("Self::{}", self.context_item.ident());
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

        let output = quote! {
            #out_trait

            #module
        };

        // Dig through the items to see if any of them have parameter errors.
        let has_endpoint_param_errors =
            self.items.iter().any(|item| match item {
                ApiItem::Fn(ApiFnItem::Invalid {
                    kind: InvalidApiItemKind::Endpoint(summary),
                    ..
                }) => summary.has_param_errors,
                _ => false,
            });
        let has_channel_param_errors =
            self.items.iter().any(|item| match item {
                ApiItem::Fn(ApiFnItem::Invalid {
                    kind: InvalidApiItemKind::Channel(summary),
                    ..
                }) => summary.has_param_errors,
                _ => false,
            });

        ApiOutput {
            output,
            context,
            has_endpoint_param_errors,
            has_channel_param_errors,
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
                let module_gen = SupportModuleGenerator {
                    dropshot: &self.dropshot,
                    module_ident,
                    item_trait,
                    context_item,
                    items: &self.items,
                };
                module_gen.to_token_stream()
            }
            (_, _, Some(module_ident)) => {
                // If the module ident is known but one of the other parts is
                // invalid, generate an empty support module. (We can't generate
                // the API description functions, even ones that immediately
                // panic, because those depend on the trait and context items
                // being valid.)
                let doc = ModuleDocComments::generate_invalid(
                    &self.item_trait.item.ident,
                );
                let outer = doc.outer();
                let vis = &self.item_trait.item.vis;

                quote! {
                    #outer
                    #vis mod #module_ident {}
                }
            }
            _ => {
                // Can't do anything if the module name is missing.
                quote! {}
            }
        }
    }
}

/// The result of calling [`ApiParser::to_output`].
struct ApiOutput {
    /// The actual output.
    output: TokenStream,

    /// The context type (typically `Self::Context`), provided as a string.
    context: String,

    /// Whether there were any endpoint parameter-related errors.
    ///
    /// If there were, then we provide a usage message.
    has_endpoint_param_errors: bool,

    /// Whether there were any channel parameter-related errors.
    ///
    /// If there were, then we provide a usage message.
    has_channel_param_errors: bool,
}

#[derive(Clone, Copy)]
struct ApiItemTrait<'ast> {
    item: &'ast ItemTraitPartParsed,
    is_valid: bool,
}

impl<'ast> ApiItemTrait<'ast> {
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
                    "API trait `{trait_ident}` must not be marked as `unsafe`"
                ),
            ));
        }

        if item.auto_token.is_some() {
            errors.push(Error::new_spanned(
                &item.auto_token,
                format!("API trait `{trait_ident}` must not be an auto trait"),
            ));
        }

        if !item.generics.params.is_empty() {
            errors.push(Error::new_spanned(
                &item.generics,
                format!("API trait `{trait_ident}` must not have generics"),
            ));
        }

        if let Some(where_clause) = &item.generics.where_clause {
            // Empty where clauses are no-ops and therefore permitted.
            if !where_clause.predicates.is_empty() {
                errors.push(Error::new_spanned(
                    where_clause,
                    format!(
                        "API trait `{trait_ident}` must not have a where clause"
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

/// The context item as present within an API trait (or not).
#[derive(Clone, Debug)]
enum ContextItem<'ast> {
    /// The item is valid.
    Valid(&'ast syn::TraitItemType),

    /// The item is invalid but present.
    Invalid(&'ast syn::TraitItemType),

    /// The item is missing.
    Missing {
        /// A conjured-up ident for the missing item.
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
                "API trait `{trait_ident}` does not have associated type \
                `{context_name}`\n\
                 (this type specifies the shared context for endpoints)",
            ),
        ));

        Self::Missing { ident: format_ident!("{context_name}") }
    }

    fn new_invalid_metadata() -> Self {
        Self::Missing {
            ident: format_ident!("{}", ApiMetadata::DEFAULT_CONTEXT_NAME),
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
}

/// Code that creates the support module for an API trait.
///
/// The support module should only be created in cases where the overall
/// structure of the trait is valid. (If it isn't, then that leads to some very
/// bad error messages). This serves as a statically typed guarantee regarding
/// that.
struct SupportModuleGenerator<'ast> {
    dropshot: &'ast TokenStream,
    module_ident: &'ast syn::Ident,

    // Invariant: the corresponding `ApiItemTrait::is_valid` is true.
    item_trait: &'ast ItemTraitPartParsed,

    // Invariant: the corresponding `ContextItem::is_valid` is true.
    context_item: &'ast syn::TraitItemType,

    // These items might or might not be valid individually.
    items: &'ast [ApiItem<'ast>],
}

impl<'ast> SupportModuleGenerator<'ast> {
    fn make_api_description(&self, doc: TokenStream) -> TokenStream {
        let dropshot = &self.dropshot;
        let trait_ident = &self.item_trait.ident;
        let context_ident = &self.context_item.ident;
        // Note we always use "pub" visibility for the items inside, since it
        // can be tricky to compute the exact visibility required here. The item
        // won't actually be exported unless the parent module is public, or it
        // is re-exported.

        let body = self.make_api_factory_body(FactoryKind::Regular);

        quote! {
            #doc
            #[automatically_derived]
            pub fn api_description<ServerImpl: #trait_ident>() -> ::std::result::Result<
                #dropshot::ApiDescription<<ServerImpl as #trait_ident>::#context_ident>,
                #dropshot::ApiDescriptionBuildError,
            > {
                #body
            }
        }
    }

    fn make_stub_api_description(&self, doc: TokenStream) -> TokenStream {
        let dropshot = &self.dropshot;
        // Note we always use "pub" visibility for the items inside. See the
        // comment in `make_api_description`.

        let body = self.make_api_factory_body(FactoryKind::Stub);

        quote! {
            #doc
            #[automatically_derived]
            pub fn stub_api_description() -> ::std::result::Result<
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
        let trait_ident = &self.item_trait.ident;
        let trait_ident_str = trait_ident.to_string();

        if self.has_invalid_fn_items() {
            let err_msg = format!(
                "invalid endpoints encountered while parsing API trait `{}`",
                trait_ident_str
            );
            quote! {
                panic!(#err_msg);
            }
        } else {
            let endpoints = self.items.iter().filter_map(|item| match item {
                ApiItem::Fn(ApiFnItem::Endpoint(e)) => {
                    Some(e.to_api_endpoint(&self.dropshot, kind))
                }

                ApiItem::Fn(ApiFnItem::Channel(c)) => {
                    Some(c.to_api_endpoint(&self.dropshot, kind))
                }

                ApiItem::Fn(ApiFnItem::Invalid { .. })
                | ApiItem::Fn(ApiFnItem::Unmanaged(_))
                | ApiItem::Other(_) => None,
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

    fn make_doc_comments(&self) -> ModuleDocComments {
        if self.has_invalid_fn_items() {
            ModuleDocComments::generate_invalid(&self.item_trait.ident)
        } else {
            ModuleDocComments::generate(
                self.dropshot,
                &self.item_trait.ident,
                &self.context_item.ident,
                self.module_ident,
            )
        }
    }

    fn make_type_checks(&self) -> impl Iterator<Item = TokenStream> + '_ {
        self.items.iter().filter_map(|item| match item {
            ApiItem::Fn(ApiFnItem::Endpoint(_)) => {
                // We don't need to generate type checks the way we do with
                // function-based macros, because we get error messages that are
                // roughly as good through the stub API description generator.
                // (Also, adding type checks would end up duplicating a ton of
                // error messages.)
                None
            }
            ApiItem::Fn(ApiFnItem::Channel(c)) => {
                // Since we use an adapter function, the stub function doesn't
                // quite capture all the type checks desired. We do need to
                // generate a subset of the typechecks.
                Some(c.params.to_trait_type_checks())
            }
            _ => None,
        })
    }

    fn has_invalid_fn_items(&self) -> bool {
        self.items.iter().any(|item| item.is_invalid())
    }
}

impl<'ast> ToTokens for SupportModuleGenerator<'ast> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let vis = &self.item_trait.vis;
        let module_ident = self.module_ident;

        let doc_comments = self.make_doc_comments();

        // Generate two API description functions: one for the real trait, and
        // one for a stub impl meant for OpenAPI generation.
        let api = self.make_api_description(doc_comments.api_description());
        let stub_api =
            self.make_stub_api_description(doc_comments.stub_api_description());

        let type_checks = self.make_type_checks();

        let outer = doc_comments.outer();

        tokens.extend(quote! {
            #outer
            #[automatically_derived]
            #vis mod #module_ident {
                // super::* pulls in definitions from the surrounding scope.
                // This is not ideal because it means that the macro can only be
                // applied to traits defined in modules, not in functions.
                //
                // A much better approach would be to be able to annotate the
                // module and say "don't create a new scope", similar to
                // `#[transparent]` as proposed in
                // https://github.com/rust-lang/rust/issues/79260.
                //
                // There does not appear to be a workaround for this, short of
                // not using a module at all. There are two other options:
                //
                // 1. Put the functions below into the parent scope. This adds a
                //    bunch of items to the scope rather than one, which seems
                //    worse on balance.
                // 2. Make these methods on a type rather than free functions in
                //    a module. This approach works for functions, but not other
                //    items like macros we may wish to define in the future.
                //
                // In RFD 479, we determined that on balance, the current
                // approach has the fewest downsides.
                use super::*;

                #(#type_checks)*

                // We don't need to generate type checks the way we do with
                // function-based macros, because we get error messages that are
                // roughly as good through the stub API description generator.
                // (Also, adding type checks would end up duplicating a ton of
                // error messages.)
                //
                // For that reason, put it above the real API description --
                // that way, the best error messages appear first.
                #stub_api

                #api
            }
        });
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
            "Support module for the Dropshot API trait \
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

    fn generate_invalid(trait_ident: &syn::Ident) -> Self {
        let outer = format!(
"**Invalid**: Support module for the Dropshot API trait `{trait_ident}`.

Errors were encountered while parsing the API.
");

        let api_description = format!(
"**Invalid, panics:** Given an implementation of `{trait_ident}`, generate an API description.

Errors were encountered while parsing the API, so this function panics.
");

        let stub_api_description = format!(
"**Invalid, panics:** Generate a _stub_ API description for `{trait_ident}`, meant for OpenAPI
generation.

Errors were encountered while parsing the API, so this function panics.
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

enum ApiItem<'ast> {
    Fn(ApiFnItem<'ast>),
    Other(&'ast TraitItemPartParsed),
}

impl ApiItem<'_> {
    fn is_invalid(&self) -> bool {
        matches!(self, Self::Fn(ApiFnItem::Invalid { .. }))
    }

    fn to_out_trait_item(&self) -> TraitItemPartParsed {
        match self {
            Self::Fn(f) => TraitItemPartParsed::Fn(f.to_out_trait_item()),
            Self::Other(o) => {
                // Strip recognized attributes, retaining all others.
                o.clone_and_strip_recognized_attrs()
            }
        }
    }
}

enum ApiFnItem<'ast> {
    Endpoint(ApiEndpoint<'ast>),
    Channel(ApiChannel<'ast>),
    // An item managed by the macro that was somehow invalid.
    Invalid { f: &'ast TraitItemFnForSignature, kind: InvalidApiItemKind },
    // A item without an endpoint or channel attribute.
    Unmanaged(&'ast TraitItemFnForSignature),
}

impl<'ast> ApiFnItem<'ast> {
    fn new(
        dropshot: &TokenStream,
        f: &'ast TraitItemFnForSignature,
        trait_ident: &'ast syn::Ident,
        context_ident: &syn::Ident,
        errors: &ErrorSink<'_, Error>,
    ) -> Self {
        // We must have zero or one endpoint or channel attributes.
        let attrs = f
            .attrs
            .iter()
            .filter_map(|attr| {
                if attr.path().is_ident(ENDPOINT_IDENT) {
                    Some(ApiAttr::Endpoint(attr))
                } else if attr.path().is_ident(CHANNEL_IDENT) {
                    Some(ApiAttr::Channel(attr))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        match attrs.as_slice() {
            [] => {
                // This is just a normal method.
                Self::Unmanaged(f)
            }
            [ApiAttr::Endpoint(eattr)] => {
                match ApiEndpoint::new(
                    dropshot,
                    f,
                    eattr,
                    trait_ident,
                    context_ident,
                    errors,
                ) {
                    Ok(endpoint) => Self::Endpoint(endpoint),
                    Err(summary) => Self::Invalid {
                        f,
                        kind: InvalidApiItemKind::Endpoint(summary),
                    },
                }
            }
            [ApiAttr::Channel(cattr)] => {
                match ApiChannel::new(
                    dropshot,
                    f,
                    cattr,
                    trait_ident,
                    context_ident,
                    errors,
                ) {
                    Ok(channel) => Self::Channel(channel),
                    Err(summary) => Self::Invalid {
                        f,
                        kind: InvalidApiItemKind::Channel(summary),
                    },
                }
            }
            [first, rest @ ..] => {
                // We must have exactly one endpoint or channel attribute, so
                // this is an error. Produce errors for all the rest of the
                // attrs.
                let name = &f.sig.ident;

                for attr in rest {
                    let msg = match (first, attr) {
                        (ApiAttr::Endpoint(_), ApiAttr::Endpoint(_)) => {
                            format!("method `{name}` marked as endpoint multiple times")
                        }
                        (ApiAttr::Channel(_), ApiAttr::Channel(_)) => {
                            format!("method `{name}` marked as channel multiple times")
                        }
                        _ => {
                            format!("method `{name}` marked as both endpoint and channel")
                        }
                    };

                    errors.push(Error::new_spanned(attr, msg));
                }

                Self::Invalid { f, kind: InvalidApiItemKind::Unknown }
            }
        }
    }

    fn to_out_trait_item(&self) -> TraitItemFnForSignature {
        match self {
            Self::Endpoint(e) => e.to_out_trait_item(),
            Self::Channel(c) => c.to_out_trait_item(),
            Self::Invalid { f, .. } | Self::Unmanaged(f) => {
                // Strip recognized attributes, retaining all others.
                f.clone_and_strip_recognized_attrs()
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

    match from_tokenstream_spanned::<EndpointMetadata>(
        l.delimiter.span(),
        &l.tokens,
    ) {
        Ok(m) => m.validate(name_str, attr, MacroKind::Trait, errors),
        Err(error) => {
            errors.push(Error::new(
                error.span(),
                format!(
                    "endpoint `{name_str}` has invalid attributes: {error}"
                ),
            ));
            return None;
        }
    }
}

struct ApiEndpoint<'ast> {
    f: &'ast TraitItemFnForSignature,
    attr: &'ast syn::Attribute,
    trait_ident: &'ast syn::Ident,
    metadata: ValidatedEndpointMetadata,
    params: EndpointParams<'ast>,
}

impl<'ast> ApiEndpoint<'ast> {
    /// Parses endpoint metadata to create a new `ApiEndpoint`.
    ///
    /// If the return value is None, at least one error occurred while parsing.
    fn new(
        dropshot: &TokenStream,
        f: &'ast TraitItemFnForSignature,
        attr: &'ast syn::Attribute,
        trait_ident: &'ast syn::Ident,
        context_ident: &syn::Ident,
        errors: &ErrorSink<'_, Error>,
    ) -> Result<Self, ApiItemErrorSummary> {
        let name_str = f.sig.ident.to_string();

        let metadata = parse_endpoint_metadata(&name_str, attr, errors);
        let params = EndpointParams::new(
            dropshot,
            &f.sig,
            RqctxKind::Trait { trait_ident, context_ident },
            errors,
        );

        match (metadata, params) {
            (Some(metadata), Some(params)) => {
                Ok(Self { f, attr, trait_ident, metadata, params })
            }
            // This means that something failed.
            (_, params) => {
                Err(ApiItemErrorSummary { has_param_errors: params.is_none() })
            }
        }
    }

    fn to_out_trait_item(&self) -> TraitItemFnForSignature {
        let mut f = self.f.clone();
        transform_signature(&mut f, &self.params.ret_ty);
        f
    }

    fn to_api_endpoint(
        &self,
        dropshot: &TokenStream,
        kind: FactoryKind,
    ) -> TokenStream {
        match kind {
            FactoryKind::Regular => {
                let name = &self.f.sig.ident;
                let trait_ident = self.trait_ident;
                // Adding the span information to path_to_name leads to fewer
                // call_site errors.
                let path_to_name = quote_spanned! {self.attr.span()=>
                    <ServerImpl as #trait_ident>::#name
                };
                self.to_api_endpoint_impl(
                    dropshot,
                    &ApiEndpointKind::Regular(&path_to_name),
                )
            }
            FactoryKind::Stub => {
                let extractor_types = self.params.extractor_types().collect();
                let ret_ty = self.params.ret_ty;
                self.to_api_endpoint_impl(
                    dropshot,
                    &ApiEndpointKind::Stub {
                        attr: &self.attr,
                        extractor_types,
                        ret_ty,
                    },
                )
            }
        }
    }

    fn to_api_endpoint_impl(
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

fn parse_channel_metadata(
    name_str: &str,
    attr: &syn::Attribute,
    errors: &ErrorSink<'_, Error>,
) -> Option<ValidatedChannelMetadata> {
    // Attempt to parse the metadata -- it must be a list.
    let l = match &attr.meta {
        syn::Meta::List(l) => l,
        _ => {
            errors.push(Error::new_spanned(
                &attr,
                format!(
                    "endpoint `{name_str}` must be of the form \
                     #[channel {{ protocol = WEBSOCKETS, path = \"/path\", ... }}]"
                ),
            ));
            return None;
        }
    };

    // TODO: Switch to from_tokenstream_spanned once
    // https://github.com/oxidecomputer/serde_tokenstream/pull/194 is available.
    match from_tokenstream::<ChannelMetadata>(&l.tokens) {
        Ok(m) => m.validate(name_str, attr, MacroKind::Trait, errors),
        Err(error) => {
            errors.push(Error::new(
                error.span(),
                format!(
                    "endpoint `{name_str}` has invalid attributes: {error}"
                ),
            ));
            return None;
        }
    }
}

struct ApiChannel<'ast> {
    f: &'ast TraitItemFnForSignature,
    attr: &'ast syn::Attribute,
    trait_ident: &'ast syn::Ident,
    metadata: ValidatedChannelMetadata,
    params: ChannelParams<'ast>,
}

impl<'ast> ApiChannel<'ast> {
    /// Parses endpoint metadata to create a new `ServerEndpoint`.
    ///
    /// If the return value is None, at least one error occurred while parsing.
    fn new(
        dropshot: &TokenStream,
        f: &'ast TraitItemFnForSignature,
        attr: &'ast syn::Attribute,
        trait_ident: &'ast syn::Ident,
        context_ident: &syn::Ident,
        errors: &ErrorSink<'_, Error>,
    ) -> Result<Self, ApiItemErrorSummary> {
        let name_str = f.sig.ident.to_string();

        let metadata = parse_channel_metadata(&name_str, attr, errors);
        let params = ChannelParams::new(
            dropshot,
            &f.sig,
            RqctxKind::Trait { trait_ident, context_ident },
            errors,
        );

        match (metadata, params) {
            (Some(metadata), Some(params)) => {
                Ok(Self { f, attr, trait_ident, metadata, params })
            }
            // This means that something failed.
            (_, params) => {
                Err(ApiItemErrorSummary { has_param_errors: params.is_none() })
            }
        }
    }

    fn to_out_trait_item(&self) -> TraitItemFnForSignature {
        let mut f = self.f.clone();
        transform_signature(&mut f, &self.params.ret_ty);
        f
    }

    fn to_api_endpoint(
        &self,
        dropshot: &TokenStream,
        kind: FactoryKind,
    ) -> TokenStream {
        match kind {
            FactoryKind::Regular => {
                // For channels, generate the adapter function here.
                let adapter_fn =
                    self.params.to_trait_adapter_fn(self.trait_ident);
                // In this case, the adapter name needs to have its type
                // parameter specified.
                let adapter_name = &self.params.adapter_name;
                let path_to_name = quote_spanned! {self.attr.span()=>
                    #adapter_name::<ServerImpl>
                };

                let endpoint = self.to_api_endpoint_impl(
                    &dropshot,
                    &ApiEndpointKind::Regular(&path_to_name),
                );

                quote_spanned! {self.attr.span()=>
                    {
                        #adapter_fn
                        #endpoint
                    }
                }
            }
            FactoryKind::Stub => {
                let extractor_types = self.params.extractor_types().collect();
                // The stub receives the adapter function's
                // signature.
                let ret_ty = &self.params.endpoint_result_ty;
                self.to_api_endpoint_impl(
                    dropshot,
                    &ApiEndpointKind::Stub {
                        attr: &self.attr,
                        extractor_types,
                        ret_ty,
                    },
                )
            }
        }
    }

    fn to_api_endpoint_impl(
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

/// Transform the signature of an endpoint function to prepare it for output.
///
/// * Strip recognized attributes from the function.
/// * Add `Send + 'static` bounds to the output type, replacing the `async` with
///   `Future`.
fn transform_signature(f: &mut TraitItemFnForSignature, ret_ty: &syn::Type) {
    f.strip_recognized_attrs();

    // Below code adapted from https://github.com/rust-lang/impl-trait-utils
    // and used under the MIT and Apache 2.0 licenses.
    let output_ty = {
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
}

enum ApiAttr<'ast> {
    Endpoint(&'ast syn::Attribute),
    Channel(&'ast syn::Attribute),
}

impl ToTokens for ApiAttr<'_> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            ApiAttr::Endpoint(attr) => attr.to_tokens(tokens),
            ApiAttr::Channel(attr) => attr.to_tokens(tokens),
        }
    }
}

/// The kind of API item that's invalid, along with potentially the reasons for
/// it being so.
#[derive(Clone, Copy, Debug)]
enum InvalidApiItemKind {
    /// An invalid endpoint.
    Endpoint(ApiItemErrorSummary),

    /// An invalid channel.
    Channel(ApiItemErrorSummary),

    /// We're not sure if it's an endpoint or a channel, or the endpoint/channel
    /// annotations were provided multiple times.
    Unknown,
}

#[derive(Clone, Copy, Debug)]
struct ApiItemErrorSummary {
    // We have no need for a similar "has_metadata_errors" right now, but we
    // could add it later.
    has_param_errors: bool,
}

trait StripRecognizedAttrs {
    fn strip_recognized_attrs(&mut self);

    fn clone_and_strip_recognized_attrs(&self) -> Self
    where
        Self: Clone,
    {
        let mut cloned = self.clone();
        cloned.strip_recognized_attrs();
        cloned
    }
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
        self.attrs.strip_recognized_attrs();
    }
}

impl StripRecognizedAttrs for syn::TraitItem {
    fn strip_recognized_attrs(&mut self) {
        match self {
            syn::TraitItem::Const(c) => {
                c.attrs.strip_recognized_attrs();
            }
            syn::TraitItem::Fn(f) => {
                f.attrs.strip_recognized_attrs();
            }
            syn::TraitItem::Type(t) => {
                t.attrs.strip_recognized_attrs();
            }
            syn::TraitItem::Macro(m) => {
                m.attrs.strip_recognized_attrs();
            }
            _ => {}
        }
    }
}

impl StripRecognizedAttrs for Vec<syn::Attribute> {
    fn strip_recognized_attrs(&mut self) {
        self.retain(|a| {
            !(a.path().is_ident(ENDPOINT_IDENT)
                || a.path().is_ident(CHANNEL_IDENT))
        });
    }
}

#[cfg(test)]
mod tests {
    use expectorate::assert_contents;

    use crate::{test_util::assert_banned_idents, util::DROPSHOT};

    use super::*;

    #[test]
    fn test_api_trait_basic() {
        let (item, errors) = do_trait(
            quote! {},
            quote! {
                trait MyTrait {
                    type Context;

                    #[endpoint { method = GET, path = "/xyz" }]
                    async fn handler_xyz(
                        rqctx: RequestContext<Self::Context>,
                    ) -> Result<HttpResponseOk<()>, HttpError>;

                    #[channel { protocol = WEBSOCKETS, path = "/ws" }]
                    async fn handler_ws(
                        rqctx: RequestContext<Self::Context>,
                        upgraded: WebsocketConnection,
                    ) -> WebsocketChannelResult;
                }
            },
        );

        assert!(errors.is_empty());
        assert_contents(
            "tests/output/api_trait_basic.rs",
            &prettyplease::unparse(&parse_quote! { #item }),
        );
    }

    #[test]
    fn test_api_trait_with_custom_params() {
        // Provide custom parameters and ensure that the original ones are
        // not used.
        let (item, errors) = do_trait(
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

                    #[channel { protocol = WEBSOCKETS, path = "/ws" }]
                    async fn handler_ws(
                        rqctx: RequestContext<Self::Situation>,
                        upgraded: WebsocketConnection,
                    ) -> WebsocketChannelResult;
                }
            },
        );

        assert!(errors.is_empty());

        let file = parse_quote! { #item };
        // Write out the file before checking it, so that we can see what it
        // looks like.
        assert_contents(
            "tests/output/api_trait_with_custom_params.rs",
            &prettyplease::unparse(&file),
        );

        // Check banned identifiers.
        let banned = [ApiMetadata::DEFAULT_CONTEXT_NAME, DROPSHOT, "my_trait"];
        assert_banned_idents(&file, banned);
    }

    // Test output for a server with no endpoints.
    //
    // This currently does not produce an error or warning -- this fact is
    // presented as a snapshot test. If we decide to add a warning or error in
    // the future, this test will change.
    #[test]
    fn test_api_trait_no_endpoints() {
        let (item, errors) = do_trait(
            quote! {},
            quote! {
                pub(crate) trait MyTrait {
                    type Context;
                }
            },
        );

        assert!(errors.is_empty());
        assert_contents(
            "tests/output/api_trait_no_endpoints.rs",
            &prettyplease::unparse(&parse_quote! { #item }),
        );
    }
}
