// Copyright 2023 Oxide Computer Company

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned, ToTokens};
use serde_tokenstream::from_tokenstream;
use syn::{punctuated::Punctuated, spanned::Spanned, *};

use crate::{extract_doc_from_attrs, get_crate, EndpointMetadata};

pub(crate) fn do_server(
    _attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> std::result::Result<(proc_macro2::TokenStream, Vec<Error>), Error> {
    // XXX: parse attributes for global metadata.

    // This has to be a trait.
    let item_trait: ItemTrait = parse2(item.clone())?;

    let mut errors = Vec::new();
    let server = Server::new(&item_trait, &mut errors);
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

struct Server<'a> {
    item_trait: &'a ItemTrait,
    supertraits: Punctuated<TypeParamBound, Token![+]>,
    // We want to maintain the order of items in the trait, so we use a single
    // list to store all of them.
    items: Vec<ServerItem<'a>>,
}

impl<'a> Server<'a> {
    fn new(item_trait: &'a ItemTrait, errors: &mut Vec<Error>) -> Self {
        // The trait itself should specify a `Send + Sync + 'static` bound.
        // Automatically insert the bounds if they're missing.
        //
        // We choose to insert these traits rather than producing an error,
        // because if we don't insert these traits, the API description
        // functions will produce tons of errors in a horrible way.
        let supertraits = {
            let mut has_send = false;
            let mut has_sync = false;
            let mut has_sized = false;
            let mut has_static = false;

            for bound in &item_trait.supertraits {
                match bound {
                    TypeParamBound::Lifetime(lifetime) => {
                        if lifetime.ident == "static" {
                            has_static = true;
                        }
                    }
                    TypeParamBound::Trait(trait_bound) => {
                        let path = &trait_bound.path;
                        if path.is_ident("Send") {
                            has_send = true;
                        } else if path.is_ident("Sync") {
                            has_sync = true;
                        } else if path.is_ident("Sized") {
                            has_sized = true;
                        }
                    }
                    TypeParamBound::Verbatim(_) => {}
                    _ => {}
                }
            }

            let mut supertraits = item_trait.supertraits.clone();

            if !has_send {
                supertraits.push(parse_quote!(Send));
            }
            if !has_sync {
                supertraits.push(parse_quote!(Sync));
            }
            if !has_sized {
                supertraits.push(parse_quote!(Sized));
            }
            if !has_static {
                supertraits.push(parse_quote!('static));
            }

            supertraits
        };

        let items: Vec<_> = item_trait
            .items
            .iter()
            .map(|item| ServerItem::new(item, errors))
            .collect();

        Self { item_trait, supertraits, items }
    }

    fn to_output(&self) -> TokenStream {
        let out_items = self.items.iter().map(|item| item.to_out_trait_item());

        let top_level_checks = self.items.iter().filter_map(|item| {
            // Don't generate any top-level checks if the trait is invalid.
            if self.is_invalid() {
                None
            } else {
                item.to_top_level_checks()
            }
        });

        // Everything else about the trait stays the same -- just the items change.
        let mut out_attrs = self.item_trait.attrs.clone();
        out_attrs.push(parse_quote!(#[::dropshot::async_trait]));

        let out_trait = ItemTrait {
            attrs: out_attrs,
            supertraits: self.supertraits.clone(),
            items: out_items.collect(),
            ..self.item_trait.clone()
        };

        // Generate two API description functions: one for the real trait, and
        // one for a stub impl meant for OpenAPI generation.
        let api_description = self.to_api_description_fn();
        let stub_api_description = self.to_stub_api_description_fn();

        quote! {
            #out_trait

            #(#top_level_checks)*

            #api_description

            #stub_api_description
        }
    }

    fn to_api_description_fn(&self) -> TokenStream {
        let trait_name = &self.item_trait.ident;
        let trait_name_str = trait_name.to_string();
        let vis = &self.item_trait.vis;
        let fn_ident = format_ident!("{trait_name_str}_to_api_description");

        let body =
            self.to_api_description_fn_body(parse_quote!(Context), |data| {
                // For the real impl, generate a closure that wraps
                // `dropshot_context` and calls the right method on it. There's some
                // levels of `move` happening here:
                //
                // 1. An owned `dropshot_context` is moved into the closure
                // 2. Another clone of `dropshot_context` is moved into the async
                //    block.
                //
                // The net result is that the async block is 'static as required by
                // Dropshot, and the closure is `Fn` (not `FnOnce`).

                let handler_func_name = data.handler_func_name();
                let name = data.name;
                let args = data.args;
                let arg_names = data.arg_names;

                quote! {
                    let #handler_func_name = {
                        move |#(#args),*| {
                            <Context as #trait_name>::#name(#(#arg_names),*)
                        }
                    };
                }
            });

        // XXX: need a way to get the dropshot crate name here.
        quote! {
            #[automatically_derived]
            #vis fn #fn_ident<Context: #trait_name>() -> ::std::result::Result<
                dropshot::ApiDescription<Context>,
                dropshot::ApiDescriptionBuildError,
            > {
                #body
            }
        }
    }

    fn to_stub_api_description_fn(&self) -> TokenStream {
        let trait_name = &self.item_trait.ident;
        let trait_name_str = trait_name.to_string();
        let vis = &self.item_trait.vis;

        let fn_ident =
            format_ident!("{trait_name_str}_to_stub_api_description");

        let body = self.to_api_description_fn_body(parse_quote!(()), |data| {
            // For the stub impl, generate a free function (not a closure). This
            // allows the return type to be specified as `impl Future`, which is
            // required for type inference.

            let handler_func_name = data.handler_func_name();
            let name_str = data.name.to_string();
            let args = &data.args;
            let ret_ty = data.ret_ty;

            quote! {
                fn #handler_func_name(#(#args),*) -> impl ::std::future::Future<Output = #ret_ty> {
                    async {
                        unimplemented!("stub implementation for `{}`", #name_str);
                    }
                }
            }
        });

        // XXX: need a way to get the dropshot crate name here.
        quote! {
            #[automatically_derived]
            #vis fn #fn_ident() -> ::std::result::Result<
                dropshot::ApiDescription<()>,
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
    fn to_api_description_fn_body<F>(
        &self,
        rqctx_transform: Type,
        mut handler_func: F,
    ) -> TokenStream
    where
        F: FnMut(HandlerFuncData<'_>) -> TokenStream,
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
                ServerItem::Endpoint(e) => Some(e.to_api_endpoint(
                    rqctx_transform.clone(),
                    &mut handler_func,
                )),
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
    fn new(item: &'a TraitItem, errors: &mut Vec<Error>) -> Self {
        match item {
            TraitItem::Fn(f) => {
                // XXX cannot have multiple endpoint or channel attributes,
                // check here.
                let endpoint_attr =
                    f.attrs.iter().find(|a| a.path().is_ident("endpoint"));
                let channel_attr =
                    f.attrs.iter().find(|a| a.path().is_ident("channel"));

                match (endpoint_attr, channel_attr) {
                    (Some(_), Some(cattr)) => {
                        errors.push(Error::new_spanned(
                            cattr,
                            "methods must not be both endpoints and channels",
                        ));
                        Self::Invalid(f)
                    }
                    (Some(eattr), None) => {
                        if let Some(endpoint) =
                            ServerEndpoint::new(f, eattr, errors)
                        {
                            Self::Endpoint(endpoint)
                        } else {
                            Self::Invalid(f)
                        }
                    }
                    (None, Some(_cattr)) => {
                        todo!("implement support for channels")
                    }
                    (None, None) => {
                        // This is just a normal method.
                        Self::Other(item)
                    }
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
                Self::Other(item)
            }
            TraitItem::Type(t) => {
                check_endpoint_or_channel_on_non_fn(
                    "type",
                    &t.ident.to_string(),
                    &t.attrs,
                    errors,
                );
                Self::Other(item)
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
                Self::Other(item)
            }
            _ => Self::Other(item),
        }
    }

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

struct ServerEndpoint<'a> {
    f: &'a TraitItemFn,
    metadata: EndpointMetadata,
    args: EndpointArgs<'a>,
    ret_ty: &'a Type,
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
}

impl<'a> ServerEndpoint<'a> {
    /// Parses endpoint metadata to create a new `ServerEndpoint`.
    ///
    /// If the return value is None, at least one error occurred while parsing.
    fn new(
        f: &'a TraitItemFn,
        attr: &'a Attribute,
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

        let args = EndpointArgs::new(f, errors);

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

        // It's okay to allow a body, for a possible stub implementation if
        // desired.

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
        TraitItem::Fn(f)
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

    fn to_api_endpoint<F>(
        &self,
        rqctx_transform: Type,
        mut handler_func: F,
    ) -> TokenStream
    where
        F: FnMut(HandlerFuncData<'_>) -> TokenStream,
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

        let arg_names = self.args.arg_names();
        let args: Vec<_> =
            self.args.to_transformed_args(rqctx_transform).collect();

        // Note that we use name_str (string) rather than name (ident) here
        // because we deliberately want to lose the span information. If we
        // don't do that, then rust-analyzer will get confused and believe that
        // the name is both a method and a variable.
        //
        // Note that there isn't any possible variable name collision here,
        // since all names are either generated by us (e.g. `extractor0`) or
        // have a prefix (e.g. `handler_`).
        let handler_func_data = HandlerFuncData {
            name,
            args: &args,
            arg_names: &arg_names,
            ret_ty: self.ret_ty,
        };
        let handler_func_name = handler_func_data.handler_func_name();
        let handler_func = handler_func(handler_func_data);

        // Same here -- we use name_str deliberately, just like in
        // `handler_func_name`.
        let api_endpoint_name = format_ident!("endpoint_{}", name_str);

        quote! {
            {
                let #api_endpoint_name = {
                    #handler_func

                    #dropshot::ApiEndpoint::new(
                        #name_str.to_string(),
                        #handler_func_name,
                        #dropshot::Method::#method_ident,
                        #content_type,
                        #path,
                    )
                    #summary
                    #description
                    #(#tags)*
                    #visible
                    #deprecated
                };
                if let Err(error) = dropshot_api.register(#api_endpoint_name) {
                    dropshot_errors.push(error);
                }
            }
        }
    }
}

/// Information passed into `handler_func` instances.
///
/// This is used to generate the handler function for each endpoint.
struct HandlerFuncData<'a> {
    name: &'a Ident,
    args: &'a [PatType],
    arg_names: &'a [Ident],
    ret_ty: &'a Type,
}

impl<'a> HandlerFuncData<'a> {
    fn handler_func_name(&self) -> Ident {
        // Note that we call to_string here because we deliberately want to lose
        // the span information. If we don't do that, then rust-analyzer will
        // get confused and believe that the name is both a method and a
        // variable.
        //
        // Note that there isn't any possible variable name collision here,
        // since all names are either generated by us (e.g. `extractor0`) or
        // have a prefix (e.g. `handler_`).
        format_ident!("handler_{}", self.name.to_string())
    }
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
    fn new(f: &'a TraitItemFn, errors: &mut Vec<Error>) -> Option<Self> {
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
        let Some(rqctx) = RqctxTy::new(rqctx) else {
            errors.push(Error::new_spanned(
                &rqctx.ty,
                format!(
                    "endpoint method `{fname}` must take RequestContext<Self>"
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

    fn arg_names(&self) -> Vec<Ident> {
        let mut names = Vec::new();
        names.push(format_ident!("rqctx"));

        for i in 0..self.shared_extractors.len() {
            names.push(format_ident!("extractor{}", i));
        }

        if self.exclusive_extractor.is_some() {
            names.push(format_ident!("last_extractor"));
        }

        names
    }

    fn to_transformed_args(
        &self,
        rqctx_transform: Type,
    ) -> impl Iterator<Item = PatType> + '_ {
        let names = self.arg_names();

        names
            .into_iter()
            .zip(self.to_transformed_arg_types(rqctx_transform))
            .map(|(name, ty)| PatType {
                attrs: Vec::new(),
                pat: Box::new(Pat::Ident(PatIdent {
                    attrs: Vec::new(),
                    ident: name,
                    by_ref: None,
                    mutability: None,
                    subpat: None,
                })),
                colon_token: Default::default(),
                ty,
            })
    }

    fn to_transformed_arg_types(
        &self,
        rqctx_transform: Type,
    ) -> impl Iterator<Item = Box<Type>> + '_ {
        std::iter::once(self.rqctx.transform_ty(rqctx_transform))
            .chain(self.shared_extractors.iter().map(|pat| pat.ty.clone()))
            .chain(self.exclusive_extractor.map(|pat| pat.ty.clone()))
    }
}

struct RqctxTy<'a> {
    pat_ty: &'a PatType,
}

impl<'a> RqctxTy<'a> {
    /// Creates a new `RqctxTy` from a `PatType`, returning None if the type is
    /// invalid.
    fn new(pat_ty: &'a PatType) -> Option<Self> {
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

        // The type must be `Self`.
        p2.path.is_ident("Self").then_some(Self { pat_ty })
    }

    /// Generate a new Type where the inner type to the `RequestContext` is
    /// `type_arg`.
    fn transform_ty(&self, type_arg: Type) -> Box<Type> {
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

        *t = type_arg;
        out_ty
    }
}
