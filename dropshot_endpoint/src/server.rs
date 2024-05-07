// Copyright 2023 Oxide Computer Company

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned, ToTokens};
use serde::Deserialize;
use serde_tokenstream::from_tokenstream;
use syn::{spanned::Spanned, *};

use crate::{extract_doc_from_attrs, get_crate, EndpointMetadata};

pub(crate) fn do_server(
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> std::result::Result<(proc_macro2::TokenStream, Vec<Error>), Error> {
    // Parse attributes.
    let server_metadata: ServerMetadata = from_tokenstream(&attr)?;

    // This has to be a trait.
    let item_trait: ItemTrait = parse2(item.clone())?;

    let mut errors = Vec::new();
    let server = Server::new(server_metadata, &item_trait, &mut errors);
    let output = server.to_output();

    // Now generate the output.
    let output = if errors.is_empty() {
        quote! {
            #output
        }
    } else {
        quote! {
            #output
        }
    };

    Ok((output, errors))
}

fn check_endpoint_or_channel_on_non_fn(
    kind: &str,
    name: &str,
    attrs: &[Attribute],
    errors: &mut Vec<Error>,
) {
    if let Some(attr) = attrs.iter().find(|a| a.path().is_ident("endpoint")) {
        errors.push(Error::new_spanned(
            attr,
            format!("{kind} `{name}` marked as endpoint is not a method"),
        ));
    }

    if let Some(attr) = attrs.iter().find(|a| a.path().is_ident("channel")) {
        errors.push(Error::new_spanned(
            attr,
            format!("{kind} `{name}` marked as channel is not a method"),
        ));
    }
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

    fn context_ty(&self) -> &str {
        self.context.as_deref().unwrap_or(Self::DEFAULT_CONTEXT_TY)
    }
}

struct Server<'a> {
    item_trait: &'a ItemTrait,
    // We want to maintain the order of items in the trait (other than the
    // Context associated type which we're always going to move to the top), so
    // we use a single list to store all of them.
    items: Vec<ServerItem<'a>>,
    // This is None if there is no context type, which is an error.
    context_item: Option<&'a TraitItemType>,
}

impl<'a> Server<'a> {
    fn new(
        server_metadata: ServerMetadata,
        item_trait: &'a ItemTrait,
        errors: &mut Vec<Error>,
    ) -> Self {
        let mut items = Vec::with_capacity(item_trait.items.len());

        let context_ty = server_metadata.context_ty();
        let mut context_item = None;

        for item in &item_trait.items {
            match item {
                TraitItem::Fn(f) => {
                    // XXX cannot have multiple endpoint or channel attributes,
                    // check here.
                    let endpoint_attr =
                        f.attrs.iter().find(|a| a.path().is_ident("endpoint"));
                    let channel_attr =
                        f.attrs.iter().find(|a| a.path().is_ident("channel"));

                    let item = match (endpoint_attr, channel_attr) {
                        (Some(_), Some(cattr)) => {
                            errors.push(Error::new_spanned(
                                cattr,
                                "methods must not be both endpoints and channels",
                            ));
                            ServerItem::Invalid(f)
                        }
                        (Some(eattr), None) => {
                            if let Some(endpoint) = ServerEndpoint::new(
                                f, eattr, context_ty, errors,
                            ) {
                                ServerItem::Endpoint(endpoint)
                            } else {
                                ServerItem::Invalid(f)
                            }
                        }
                        (None, Some(_cattr)) => {
                            todo!("implement support for channels")
                        }
                        (None, None) => {
                            // This is just a normal method.
                            ServerItem::Other(item)
                        }
                    };
                    items.push(item);
                }

                TraitItem::Type(t) => {
                    check_endpoint_or_channel_on_non_fn(
                        "type",
                        &t.ident.to_string(),
                        &t.attrs,
                        errors,
                    );

                    // We're looking for the context type. Look for a type with
                    // the right name.
                    if t.ident == context_ty {
                        // This is the context type.
                        context_item = Some(t);
                    } else {
                        // This is something else.
                        items.push(ServerItem::Other(item));
                    }
                }

                // Everything else is permissible (for now?) -- just ensure that
                // they aren't marked as `endpoint` or `channel`.
                TraitItem::Const(c) => {
                    check_endpoint_or_channel_on_non_fn(
                        "const",
                        &c.ident.to_string(),
                        &c.attrs,
                        errors,
                    );
                }

                TraitItem::Macro(m) => {
                    check_endpoint_or_channel_on_non_fn(
                        "macro",
                        &m.mac.path.to_token_stream().to_string(),
                        &m.attrs,
                        errors,
                    );
                    // Note that we can't expand macros within proc macros, so
                    // that can't be supported.
                    items.push(ServerItem::Other(item));
                }
                _ => {
                    items.push(ServerItem::Other(item));
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

        Self { item_trait, items, context_item }
    }

    fn to_output(&self) -> TokenStream {
        let context_item = self.make_context_trait_item();
        let other_items =
            self.items.iter().map(|item| item.to_out_trait_item());
        let out_items = context_item.into_iter().chain(other_items);

        let top_level_checks = self.items.iter().filter_map(|item| {
            // Don't generate any top-level checks if the trait is invalid.
            if self.is_invalid() {
                None
            } else {
                item.to_top_level_checks()
            }
        });

        // We need a 'static bound on the trait itself, otherwise we get `T
        // doesn't live long enough` errors.
        let mut supertraits = self.item_trait.supertraits.clone();
        supertraits.push(parse_quote!('static));

        // Everything else about the trait stays the same -- just the items change.

        let out_trait = ItemTrait {
            supertraits,
            items: out_items.collect(),
            ..self.item_trait.clone()
        };

        // Generate two API description functions: one for the real trait, and
        // one for a stub impl meant for OpenAPI generation. Restrict the first
        // to the case where the context type is present, since the error
        // messages are quite ugly otherwise.
        let api_description =
            self.context_item.is_some().then(|| self.make_api_description_fn());
        let stub_api_description = self.make_stub_api_description_fn();

        quote! {
            #out_trait

            #(#top_level_checks)*

            #api_description

            #stub_api_description
        }
    }

    fn make_context_trait_item(&self) -> Option<TraitItem> {
        // XXX obtain dropshot type

        let item = self.context_item?;
        let mut bounds = item.bounds.clone();
        // Generate these bounds for the associated type. We could require that
        // users specify them and error out if they don't, but this is much
        // easier to do, and also produces better errors.
        bounds.push(parse_quote!(dropshot::ServerContext));
        bounds.push(parse_quote!('static));

        let out_item = TraitItemType { bounds, ..item.clone() };
        Some(TraitItem::Type(out_item))
    }

    fn make_api_description_fn(&self) -> TokenStream {
        let trait_name = &self.item_trait.ident;
        let trait_name_str = trait_name.to_string();
        let vis = &self.item_trait.vis;
        let fn_ident = format_ident!("{trait_name_str}_api_description");

        let body = self.make_api_description_fn_body(|data| {
            // For the real impl, just forward to the method on the trait.
            // It is guaranteed to be static, so this should Just Work.
            let dropshot = data.dropshot;
            let name = data.name;
            let method_ident = data.method;
            let content_type = data.content_type;
            let path = data.path;

            quote! {
                #dropshot::ApiEndpoint::new(
                    stringify!(#name).to_string(),
                    <ServerImpl as #trait_name>::#name,
                    #dropshot::Method::#method_ident,
                    #content_type,
                    #path,
                )
            }
        });

        // XXX: need a way to get the dropshot crate name here.
        quote! {
            #[automatically_derived]
            #vis fn #fn_ident<ServerImpl: #trait_name>() -> ::std::result::Result<
                dropshot::ApiDescription<<ServerImpl as #trait_name>::Context>,
                dropshot::ApiDescriptionBuildError,
            > {
                #body
            }
        }
    }

    fn make_stub_api_description_fn(&self) -> TokenStream {
        let trait_name = &self.item_trait.ident;
        let trait_name_str = trait_name.to_string();
        let vis = &self.item_trait.vis;

        let fn_ident = format_ident!("{trait_name_str}_stub_api_description");

        let body = self.make_api_description_fn_body(|data| {
            // For the stub impl, call `ApiEndpoint::new_stub`. Pass in the
            // request and response types as a function pointer type parameter.
            let dropshot = data.dropshot;
            let name = data.name;
            let method_ident = data.method;
            let content_type = data.content_type;
            let path = data.path;
            let arg_types = data.args.extractor_types();
            let ret_ty = data.ret_ty;

            quote! {
                #dropshot::ApiEndpoint::new_stub::<fn ((#(#arg_types,)*)) -> #ret_ty, _, _>(
                    stringify!(#name).to_string(),
                    #dropshot::Method::#method_ident,
                    #content_type,
                    #path,
                )
            }
        });

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

    /// Generates the API description function body.
    ///
    /// This code is shared across the real and stub API description functions.
    /// If the body represents an invalid function, `Err` is returned, otherwise
    /// `Ok`.
    fn make_api_description_fn_body<F>(
        &self,
        mut endpoint_func: F,
    ) -> TokenStream
    where
        F: FnMut(ApiEndpointFuncData<'_>) -> TokenStream,
    {
        let trait_name = &self.item_trait.ident;
        let trait_name_str = trait_name.to_string();

        if self.is_invalid() {
            quote! {
                panic!(
                    "errors encountered while generating dropshot server `{}`",
                    #trait_name_str,
                );
            }
        } else {
            let endpoints = self.items.iter().filter_map(|item| match item {
                ServerItem::Endpoint(e) => {
                    Some(e.to_api_endpoint(&mut endpoint_func))
                }
                ServerItem::Invalid(_) | ServerItem::Other(_) => None,
            });

            quote! {
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

enum ServerItem<'a> {
    Endpoint(ServerEndpoint<'a>),
    // For endpoints that we couldn't parse successfully, we continue to
    // generate the underlying method because rust-analyzer works better if it
    // exists.
    Invalid(&'a TraitItemFn),
    Other(&'a TraitItem),
}

impl<'a> ServerItem<'a> {
    fn to_out_trait_item(&self) -> TraitItem {
        match self {
            Self::Endpoint(e) => e.to_out_trait_item(),
            Self::Invalid(e) => {
                // Retain all attributes other than the endpoint attribute.
                let f = strip_recognized_attrs(e);
                TraitItem::Fn(f)
            }
            Self::Other(o) => (*o).clone(),
        }
    }

    fn to_top_level_checks(&self) -> Option<TokenStream> {
        match self {
            Self::Endpoint(e) => Some(e.to_top_level_checks()),
            _ => None,
        }
    }
}

fn parse_metadata(
    attr: &Attribute,
    errors: &mut Vec<Error>,
) -> Option<EndpointMetadata> {
    // Attempt to parse the metadata -- it must be a list.
    let l = match &attr.meta {
        Meta::List(l) => l,
        _ => {
            errors.push(Error::new_spanned(
                &attr,
                "endpoint attribute must be of the form #[endpoint { ... }]",
            ));
            return None;
        }
    };

    match from_tokenstream(&l.tokens) {
        Ok(m) => Some(m),
        Err(error) => {
            errors.push(error);
            return None;
        }
    }

    // XXX: ban _dropshot_crate, should only be specified in global metadata
}

struct ServerEndpoint<'a> {
    f: &'a TraitItemFn,
    metadata: EndpointMetadata,
    args: EndpointArgs<'a>,
    ret_ty: &'a Type,
}

impl<'a> ServerEndpoint<'a> {
    /// Parses endpoint metadata to create a new `ServerEndpoint`.
    ///
    /// If the return value is None, at least one error occurred while parsing.
    fn new(
        f: &'a TraitItemFn,
        attr: &'a Attribute,
        context_ty: &str,
        errors: &mut Vec<Error>,
    ) -> Option<Self> {
        let metadata = parse_metadata(attr, errors);
        let fname = f.sig.ident.to_string();

        let mut had_errors = false;

        // An endpoint must be async.
        if f.sig.asyncness.is_none() {
            errors.push(Error::new_spanned(
                &f.sig.fn_token,
                format!("endpoint method `{fname}` must be async"),
            ));
            had_errors = true;
        }

        let args = EndpointArgs::new(f, context_ty, errors);

        // The return value must exist.
        let ret_ty = match &f.sig.output {
            ReturnType::Type(_, ty) => Some(ty),
            ReturnType::Default => {
                errors.push(Error::new_spanned(
                    &f.sig,
                    format!("endpoint method `{fname}` must return a Result"),
                ));
                had_errors = true;
                None
            }
        };

        if had_errors {
            return None;
        }

        match (metadata, args, ret_ty) {
            (Some(metadata), Some(args), Some(ret_ty)) => {
                Some(Self { f, metadata, args, ret_ty })
            }
            // This means that something failed.
            _ => None,
        }
    }

    fn to_out_trait_item(&self) -> TraitItem {
        // Retain all attributes other than the endpoint attribute.
        let f = strip_recognized_attrs(self.f);

        // Below code adapted from https://github.com/rust-lang/impl-trait-utils
        // and used under the MIT and Apache 2.0 licenses.
        let output_ty = {
            let ret_ty = &self.ret_ty;
            let bounds = parse_quote! { ::core::future::Future<Output = #ret_ty> + Send + 'static };
            Type::ImplTrait(TypeImplTrait {
                impl_token: Default::default(),
                bounds,
            })
        };
        TraitItem::Fn(TraitItemFn {
            sig: Signature {
                asyncness: None,
                output: ReturnType::Type(
                    Default::default(),
                    Box::new(output_ty),
                ),
                ..f.sig
            },
            ..f
        })
    }

    fn to_top_level_checks(&self) -> TokenStream {
        let dropshot = get_crate(self.metadata._dropshot_crate.clone());

        // The first parameter must be a RequestContext<T>.
        let rqctx_ty = self.args.rqctx.transform_ty(parse_quote!(()));
        let rqctx_span = rqctx_ty.span();
        let rqctx_check = quote_spanned! { rqctx_span=>
            const _: fn() = || {
                struct NeedRequestContext(<#rqctx_ty as #dropshot::RequestContextArgument>::Context);
            };
        };

        // XXX: most of this code is copied from lib.rs -- we should move it to
        // a shared location.

        // Subsequent parameters must impl SharedExtractor.
        let shared_extractor_checks =
            self.args.shared_extractors.iter().map(|pat| {
                let ty = &pat.ty;
                let span = ty.span();
                quote_spanned! { span=>
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

        // The final parameter must impl ExclusiveExtractor. (It's okay if it's
        // another SharedExtractor.  Those impl ExclusiveExtractor, too.)
        let exclusive_extractor_check =
            self.args.exclusive_extractor.map(|pat| {
                let ty = &pat.ty;
                let span = ty.span();
                quote_spanned! { span=>
                    const _: fn() = || {
                        fn need_exclusive_extractor<T>()
                        where
                            T: ?Sized + #dropshot::ExclusiveExtractor,
                        {
                        }
                        need_exclusive_extractor::<#ty>();
                    };
                }
            });

        let ret_check = {
            let ret_ty = self.ret_ty;
            let span = ret_ty.span();
            quote_spanned! { span=>
                const _: fn() = || {
                    // Pick apart the Result type.
                    trait ResultTrait {
                        type T;
                        type E;
                    }

                    // Verify that the affirmative result implements the
                    // HttpResponse trait.
                    impl<TT, EE> ResultTrait for Result<TT, EE>
                    where
                        TT: #dropshot::HttpResponse,
                    {
                        type T = TT;
                        type E = EE;
                    }

                    // This is not strictly necessary as we'll try to use
                    // #ret_ty as ResultTrait below. This does, however,
                    // produce a cleaner error message as type definition
                    // errors are detected prior to function type validation.
                    struct NeedHttpResponse(
                        <#ret_ty as ResultTrait>::T,
                    );

                    // Verify that the error result is of type HttpError.
                    trait TypeEq {
                        type This: ?Sized;
                    }

                    impl<T: ?Sized> TypeEq for T {
                        type This = Self;
                    }

                    fn validate_result_error_type<T>()
                    where
                        T: ?Sized + TypeEq<This = #dropshot::HttpError>,
                    {
                    }

                    validate_result_error_type::<
                        <#ret_ty as ResultTrait>::E,
                    >();
                };
            }
        };

        quote! {
            #rqctx_check
            #(#shared_extractor_checks)*
            #exclusive_extractor_check
            #ret_check
        }
    }

    fn to_api_endpoint<F>(&self, mut endpoint_func: F) -> TokenStream
    where
        F: FnMut(ApiEndpointFuncData<'_>) -> TokenStream,
    {
        let dropshot = get_crate(self.metadata._dropshot_crate.clone());

        let name = &self.f.sig.ident;
        let name_str = name.to_string();

        let path = &self.metadata.path;

        let method = &self.metadata.method;
        let method_ident = format_ident!("{}", method.as_str());

        let content_type = self
            .metadata
            .content_type
            .clone()
            .unwrap_or_else(|| "application/json".to_string());
        // XXX: check content type

        let (summary_text, description_text) =
            extract_doc_from_attrs(&self.f.attrs);
        // XXX: description doc comment?

        let summary = summary_text.map(|summary| {
            quote! { .summary(#summary) }
        });
        let description = description_text.map(|description| {
            quote! { .description(#description) }
        });

        let tags = self
            .metadata
            .tags
            .iter()
            .map(|tag| {
                quote! { .tag(#tag) }
            })
            .collect::<Vec<_>>();

        let visible = self.metadata.unpublished.then(|| {
            quote! { .visible(false) }
        });

        let deprecated = self.metadata.deprecated.then(|| {
            quote! { .deprecated(true) }
        });

        let func_data = ApiEndpointFuncData {
            dropshot: &dropshot,
            name,
            method: &method_ident,
            content_type: &content_type,
            path: &path,
            args: &self.args,
            ret_ty: self.ret_ty,
        };
        let endpoint_func = endpoint_func(func_data);

        // Note that we use name_str (string) rather than name (ident) here
        // because we deliberately want to lose the span information. If we
        // don't do that, then rust-analyzer will get confused and believe that
        // the name is both a method and a variable.
        //
        // Note that there isn't any possible variable name collision here,
        // since all names are prefixed with "endpoint_".
        let endpoint_name = format_ident!("endpoint_{}", name_str);

        quote! {
            {
                let #endpoint_name = {
                    #endpoint_func
                    #summary
                    #description
                    #(#tags)*
                    #visible
                    #deprecated
                };
                if let Err(error) = dropshot_api.register(#endpoint_name) {
                    dropshot_errors.push(error);
                }
            }
        }
    }
}

/// Information passed into `handler_func` instances.
///
/// This is used to generate the handler function for each endpoint.
struct ApiEndpointFuncData<'a> {
    dropshot: &'a TokenStream,
    name: &'a Ident,
    method: &'a Ident,
    content_type: &'a str,
    path: &'a str,
    args: &'a EndpointArgs<'a>,
    ret_ty: &'a Type,
}

fn strip_recognized_attrs(f: &TraitItemFn) -> TraitItemFn {
    let attrs = f
        .attrs
        .iter()
        .filter(|a| {
            // Strip endpoint and channel attributes, since those are the ones
            // recognized by us.
            !(a.path().is_ident("endpoint") || a.path().is_ident("channel"))
        })
        .cloned()
        .collect();

    TraitItemFn { attrs, ..f.clone() }
}

struct EndpointArgs<'a> {
    rqctx: RqctxTy<'a>,
    shared_extractors: Vec<&'a PatType>,
    exclusive_extractor: Option<&'a PatType>,
}

impl<'a> EndpointArgs<'a> {
    fn new(
        f: &'a TraitItemFn,
        context_ty: &str,
        errors: &mut Vec<Error>,
    ) -> Option<Self> {
        let mut had_errors = false;
        let fname = f.sig.ident.to_string();

        if let Some(FnArg::Receiver(r)) = f.sig.inputs.first() {
            errors.push(Error::new_spanned(
                r,
                format!("endpoint method `{fname}` must be static"),
            ));

            // Bail immediately for non-static methods.
            return None;
        }

        let mut inputs = f.sig.inputs.iter();

        // The first arg must be a RequestContext.
        let rqctx = match inputs.next() {
            Some(FnArg::Typed(pat)) => pat,
            _ => {
                errors.push(Error::new_spanned(
                    &f.sig,
                    format!(
                        "endpoint method `{fname}` must take a RequestContext"
                    ),
                ));
                return None;
            }
        };

        // Specifically, it must have exactly one type parameter, which must be
        // exactly `Self`.
        let Some(rqctx) = RqctxTy::new(rqctx, context_ty) else {
            errors.push(Error::new_spanned(
                &rqctx.ty,
                format!(
                    "endpoint method `{fname}` must take \
                     RequestContext<Self::{context_ty}>"
                ),
            ));
            return None;
        };

        // Subsequent parameters other than the last one must impl
        // SharedExtractor.
        let mut shared_extractors = Vec::new();
        while let Some(FnArg::Typed(pat)) = inputs.next() {
            shared_extractors.push(pat);
        }

        // Pop the last one off the iterator -- it must impl ExclusiveExtractor.
        // (A SharedExtractor can impl ExclusiveExtractor too.)
        let exclusive_extractor = shared_extractors.pop();

        // Endpoint methods can't have type or const parameters.
        if f.sig.generics.params.iter().any(|p| match p {
            GenericParam::Type(_) | GenericParam::Const(_) => true,
            _ => false,
        }) {
            errors.push(Error::new_spanned(
                &f.sig.generics.params,
                format!(
                    "endpoint method `{fname}` must not have generic parameters"
                ),
            ));
            had_errors = true;
        }

        // Ban where clauses in endpoint methods.
        //
        // * Anything with `Self` will lead to compile errors, unless it's
        //   `Self: Send/Sync/Sized/'static` which is redundant anyway.
        // * We could support other where clauses (e.g. `where usize: Copy`),
        //   but it's easier to just ban them altogether for now.
        if let Some(c) = &f.sig.generics.where_clause {
            if c.predicates.iter().any(|p| match p {
                WherePredicate::Lifetime(_) => false,
                WherePredicate::Type(_) => true,
                _ => true,
            }) {
                errors.push(Error::new_spanned(
                    c,
                    format!(
                        "endpoint method `{fname}` must not have a where clause"
                    ),
                ));
                had_errors = true;
            }
        }

        (!had_errors).then(|| Self {
            rqctx,
            shared_extractors,
            exclusive_extractor,
        })
    }

    fn extractor_types(&self) -> impl Iterator<Item = &Type> {
        self.shared_extractors
            .iter()
            .map(|pat| &*pat.ty)
            .chain(self.exclusive_extractor.map(|pat| &*pat.ty))
    }
}

struct RqctxTy<'a> {
    pat_ty: &'a PatType,
}

impl<'a> RqctxTy<'a> {
    /// Creates a new `RqctxTy` from a `PatType`, returning None if the type is
    /// invalid.
    fn new(pat_ty: &'a PatType, context_ty: &str) -> Option<Self> {
        let Type::Path(p) = &*pat_ty.ty else { return None };

        // Inspect the last path segment.
        let Some(last_segment) = p.path.segments.last() else { return None };

        // It must have exactly one angle-bracketed argument.
        let PathArguments::AngleBracketed(a) = &last_segment.arguments else {
            return None;
        };
        if a.args.len() != 1 {
            return None;
        }

        // The argument must be a type.
        let GenericArgument::Type(Type::Path(p2)) = a.args.first().unwrap()
        else {
            return None;
        };

        // The type must be `Self::Context`.
        if p2.path.segments.len() != 2 {
            return None;
        }
        if p2.path.segments[0].ident != "Self"
            || p2.path.segments[1].ident != context_ty
        {
            return None;
        }

        Some(Self { pat_ty })
    }

    /// Generate a new Type where the inner type to the `RequestContext` is
    /// `type_arg`.
    fn transform_ty(&self, ty_arg: Type) -> Box<Type> {
        let mut out_ty = self.pat_ty.ty.clone();
        let Type::Path(p) = &mut *out_ty else {
            unreachable!("validated at construction")
        };

        let last_segment =
            p.path.segments.last_mut().expect("validated at construction");

        let PathArguments::AngleBracketed(a) = &mut last_segment.arguments
        else {
            unreachable!("validated at construction")
        };

        let arg = a.args.first_mut().expect("validated at construction");
        let GenericArgument::Type(t) = arg else {
            unreachable!("validated at construction")
        };

        *t = ty_arg;
        out_ty
    }
}
