// Copyright 2023 Oxide Computer Company

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned, ToTokens};
use serde_tokenstream::from_tokenstream;
use syn::{spanned::Spanned, *};

use crate::{extract_doc_from_attrs, get_crate, EndpointMetadata};

pub(crate) fn do_server(
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> std::result::Result<(proc_macro2::TokenStream, Vec<Error>), Error> {
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
    attrs: &[Attribute],
    errors: &mut Vec<Error>,
) {
    if let Some(attr) = attrs.iter().find(|a| a.path().is_ident("endpoint")) {
        errors.push(Error::new_spanned(
            attr,
            "only functions can be marked as endpoints",
        ));
    }

    if let Some(attr) = attrs.iter().find(|a| a.path().is_ident("channel")) {
        errors.push(Error::new_spanned(
            attr,
            "only functions can be marked as channels",
        ));
    }
}

struct Server<'a> {
    item_trait: &'a ItemTrait,
    // We want to maintain the order of items in the trait, so we use a single
    // list to store all of them.
    items: Vec<ServerItem<'a>>,
}

impl<'a> Server<'a> {
    fn new(item_trait: &'a ItemTrait, errors: &mut Vec<Error>) -> Self {
        let mut items: Vec<_> = Vec::with_capacity(item_trait.items.len());

        for item in &item_trait.items {
            match item {
                TraitItem::Fn(f) => {
                    // XXX cannot have multiple endpoints or channel attributes,
                    // check here.
                    let endpoint_attr =
                        f.attrs.iter().find(|a| a.path().is_ident("endpoint"));
                    let channel_attr =
                        f.attrs.iter().find(|a| a.path().is_ident("channel"));

                    match (endpoint_attr, channel_attr) {
                        (Some(_), Some(cattr)) => {
                            errors.push(Error::new_spanned(
                                cattr,
                                "methods cannot be both endpoints and channels",
                            ));
                        }
                        (Some(eattr), None) => {
                            if let Some(endpoint) =
                                ServerEndpoint::new(f, eattr, errors)
                            {
                                items.push(ServerItem::Endpoint(endpoint));
                            }
                        }
                        (None, Some(_cattr)) => {
                            todo!("implement support for channels")
                        }
                        (None, None) => {
                            // This is just a normal method, so it goes in
                            // other_items.
                            items.push(ServerItem::Other(item));
                        }
                    }
                }

                // Everything else is permissible (for now?) -- just ensure that
                // they aren't marked as `endpoint` or `channel`.
                TraitItem::Const(c) => {
                    check_endpoint_or_channel_on_non_fn(&c.attrs, errors);
                    items.push(ServerItem::Other(item));
                }
                TraitItem::Type(t) => {
                    check_endpoint_or_channel_on_non_fn(&t.attrs, errors);
                    items.push(ServerItem::Other(item));
                }
                TraitItem::Macro(m) => {
                    check_endpoint_or_channel_on_non_fn(&m.attrs, errors);
                    // Note that we can't expand macros within proc macros, so
                    // that can't be supported.
                    items.push(ServerItem::Other(item));
                }
                TraitItem::Verbatim(_) | _ => {
                    items.push(ServerItem::Other(item));
                }
            }
        }

        Self { item_trait, items }
    }

    fn to_output(&self) -> TokenStream {
        let out_items = self.items.iter().map(|item| item.to_out_trait_item());

        // Everything else about the trait stays the same -- just the items change.
        let mut out_attrs = self.item_trait.attrs.clone();
        out_attrs.push(parse_quote!(#[::async_trait::async_trait]));

        let out_trait = ItemTrait {
            attrs: out_attrs,
            items: out_items.collect(),
            ..self.item_trait.clone()
        };

        let api_description = self.to_api_description();

        quote! {
            #out_trait

            #api_description
        }
    }

    fn to_api_description(&self) -> TokenStream {
        let trait_name = &self.item_trait.ident;
        let endpoints = self.items.iter().filter_map(|item| match item {
            ServerItem::Endpoint(e) => Some(e.to_api_endpoint()),
            ServerItem::Other(_) => None,
        });

        // XXX: need a way to get the dropshot crate name here.
        quote! {
            fn to_api_description<T: #trait_name>(input: T) -> dropshot::ApiDescription<()> {
                let __dropshot_input: ::std::sync::Arc<dyn #trait_name> =
                    ::std::sync::Arc::new(input);
                let mut __dropshot_api = dropshot::ApiDescription::new();
                let mut __dropshot_errors = Vec::new();

                #(#endpoints)*

                if !__dropshot_errors.is_empty() {
                    let mut error_str = String::from("failed to register endpoints: \n");
                    for error in __dropshot_errors {
                        error_str.push_str("  - ");
                        error_str.push_str(&format!("{}\n", error));
                    }
                    panic!("{}", error_str);
                }

                __dropshot_api
            }
        }
    }
}

enum ServerItem<'a> {
    Endpoint(ServerEndpoint<'a>),
    Other(&'a TraitItem),
}

impl<'a> ServerItem<'a> {
    fn to_out_trait_item(&self) -> TraitItem {
        match self {
            Self::Endpoint(e) => e.to_out_trait_item(),
            Self::Other(o) => (*o).clone(),
        }
    }
}

struct ServerEndpoint<'a> {
    f: &'a TraitItemFn,
    attr: &'a Attribute,
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

        let mut had_errors = false;

        // An endpoint must be async.
        if f.sig.asyncness.is_none() {
            errors.push(Error::new_spanned(
                &f.sig.fn_token,
                "endpoint methods must be async",
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
                    "endpoint methods must return a Result",
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
                Some(Self { f, attr, metadata, args, ret_ty })
            }
            // This means that something failed.
            _ => None,
        }
    }

    fn to_out_trait_item(&self) -> TraitItem {
        // Retain all attributes other than the endpoint attribute.
        let attrs = self
            .f
            .attrs
            .iter()
            .filter(|a| !a.path().is_ident("endpoint"))
            .cloned()
            .collect();

        let f = TraitItemFn {
            attrs,
            sig: self.f.sig.clone(),
            default: self.f.default.clone(),
            semi_token: self.f.semi_token,
        };
        TraitItem::Fn(f)
    }

    fn to_api_endpoint(&self) -> TokenStream {
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
        // TODO: check content type

        let (summary_text, description_text) =
            extract_doc_from_attrs(&self.f.attrs);
        // TODO: description doc comment?

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

        // The first parameter must be a RequestContext<T>.
        let rqctx_span = self.args.rqctx.ty.span();
        let rqctx_ty = &self.args.rqctx.ty;
        let rqctx_check = quote_spanned! { rqctx_span=>
            const _: fn() = || {
                struct NeedRequestContext(<#rqctx_ty as #dropshot::RequestContextArgument>::Context);
            };
        };

        // Subsequent parameters must impl SharedExtractor.
        let shared_extractor_checks =
            self.args.shared_extractors.iter().map(|pat| {
                let ty = &pat.ty;
                quote! {
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
                quote! {
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

        // XXX: param checks and ret check

        let arg_names = self.args.arg_names();
        let args = self.args.to_out_pat_types();

        // Generate a new function that wraps `__dropshot_input` and calls the
        // right method on it. There's some levels of `move` happening here:
        //
        // 1. An owned `__dropshot_input` is moved into the closure
        // 2. Another clone of `__dropshot_input` is moved into the async block.
        //
        // The net result is that the async block is 'static as required by
        // Dropshot, and the closure is `Fn` (not `FnOnce`).
        let handler_func = quote! {
            let #name = {
                let __dropshot_input = __dropshot_input.clone();
                move |#(#args),*| {
                    let __dropshot_input = __dropshot_input.clone();
                    async move {
                        __dropshot_input.#name(#(#arg_names),*).await
                    }
                }
            };
        };

        quote! {
            {
                let #name = {
                    #rqctx_check

                    #(#shared_extractor_checks)*

                    #exclusive_extractor_check

                    #handler_func

                    #dropshot::ApiEndpoint::new(
                        #name_str.to_string(),
                        #name,
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
                if let Err(error) = __dropshot_api.register(#name) {
                    __dropshot_errors.push(error);
                }
            }
        }
    }
}

struct EndpointArgs<'a> {
    rqctx: &'a PatType,
    shared_extractors: Vec<&'a PatType>,
    exclusive_extractor: Option<&'a PatType>,
}

impl<'a> EndpointArgs<'a> {
    fn new(f: &'a TraitItemFn, errors: &mut Vec<Error>) -> Option<Self> {
        let mut had_errors = false;

        let mut inputs = f.sig.inputs.iter();

        // The first arg must be a receiver.
        match inputs.next() {
            Some(FnArg::Receiver(r)) => {
                // The only allowed receiver is &self.
                // XXX consider lifetime parameters?
                if !(r.reference.is_some() && r.mutability.is_none()) {
                    errors.push(Error::new_spanned(
                        &f.sig,
                        "endpoint methods must take `&self`",
                    ));

                    // Continue parsing the rest of the arguments, though -- we
                    // may be able to produce more errors.
                    had_errors = true;
                }
            }
            _ => {
                errors.push(Error::new_spanned(
                    &f.sig,
                    "endpoint methods must take `&self`",
                ));

                // Bail right away for static methods.
                return None;
            }
        };

        // The second arg must be a RequestContext.
        let rqctx = match inputs.next() {
            Some(FnArg::Typed(pat)) => &*pat,
            _ => {
                errors.push(Error::new_spanned(
                    &f.sig,
                    "endpoint methods must take a RequestContext",
                ));
                return None;
            }
        };

        // Subsequent parameters other than the last one must impl
        // SharedExtractor.
        let mut shared_extractors = Vec::new();
        while let Some(FnArg::Typed(pat)) = inputs.next() {
            shared_extractors.push(&*pat);
        }

        // Pop the last one off the iterator -- it must impl ExclusiveExtractor.
        // (A SharedExtractor can impl ExclusiveExtractor too.)
        let exclusive_extractor = shared_extractors.pop();

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

    fn to_out_pat_types(&self) -> impl Iterator<Item = PatType> + '_ {
        let names = self.arg_names();

        names.into_iter().zip(self.iter_args()).map(|(name, arg)| PatType {
            pat: Box::new(Pat::Ident(PatIdent {
                attrs: Vec::new(),
                ident: name,
                by_ref: None,
                mutability: None,
                subpat: None,
            })),
            ..arg.clone()
        })
    }

    fn iter_args(&self) -> impl Iterator<Item = &PatType> + '_ {
        std::iter::once(self.rqctx)
            .chain(self.shared_extractors.iter().copied())
            .chain(self.exclusive_extractor)
    }
}
