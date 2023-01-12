// Copyright 2023 Oxide Computer Company

//! This package defines macro attributes associated with HTTP handlers. These
//! attributes are used both to define an HTTP API and to generate an OpenAPI
//! Spec (OAS) v3 document that describes the API.

// Clippy's style advice is definitely valuable, but not worth the trouble for
// automated enforcement.
#![allow(clippy::style)]

use quote::format_ident;
use quote::quote;
use quote::{quote_spanned, ToTokens};
use serde::Deserialize;
use serde_tokenstream::from_tokenstream;
use serde_tokenstream::Error;
use std::ops::DerefMut;
use syn::spanned::Spanned;

use syn_parsing::ItemFnForSignature;

mod syn_parsing;

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
enum MethodType {
    DELETE,
    GET,
    PATCH,
    POST,
    PUT,
    OPTIONS,
}

impl MethodType {
    fn as_str(&self) -> &'static str {
        match self {
            MethodType::DELETE => "DELETE",
            MethodType::GET => "GET",
            MethodType::PATCH => "PATCH",
            MethodType::POST => "POST",
            MethodType::PUT => "PUT",
            MethodType::OPTIONS => "OPTIONS",
        }
    }
}

#[derive(Deserialize, Debug)]
struct EndpointMetadata {
    method: MethodType,
    path: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    unpublished: bool,
    #[serde(default)]
    deprecated: bool,
    content_type: Option<String>,
    _dropshot_crate: Option<String>,
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

const DROPSHOT: &str = "dropshot";
const USAGE: &str = "Endpoint handlers must have the following signature:
    async fn(
        rqctx: std::sync::Arc<dropshot::RequestContext<MyContext>>,
        [query_params: Query<Q>,]
        [path_params: Path<P>,]
        [body_param: TypedBody<J>,]
        [body_param: UntypedBody<J>,]
        [raw_request: RawRequest,]
    ) -> Result<HttpResponse*, HttpError>";

/// This attribute transforms a handler function into a Dropshot endpoint
/// suitable to be used as a parameter to
/// [`ApiDescription::register()`](../dropshot/struct.ApiDescription.html#method.register).
/// It encodes information relevant to the operation of an API endpoint beyond
/// what is expressed by the parameter and return types of a handler function.
///
/// ```ignore
/// #[endpoint {
///     // Required fields
///     method = { DELETE | GET | OPTIONS | PATCH | POST | PUT },
///     path = "/path/name/with/{named}/{variables}",
///
///     // Optional tags for the operation's description
///     tags = [ "all", "your", "OpenAPI", "tags" ],
///     // Specifies the media type used to encode the request body
///     content_type = { "application/json" | "application/x-www-form-urlencoded" }
///     // A value of `true` marks the operation as deprecated
///     deprecated = { true | false },
///     // A value of `true` causes the operation to be omitted from the API description
///     unpublished = { true | false },
/// }]
/// ```
///
/// See the dropshot documentation for
/// [how to specify an endpoint](../dropshot/index.html#api-handler-functions)
/// or
/// [a description of the attribute parameters](../dropshot/index.html#endpoint----attribute-parameters)
#[proc_macro_attribute]
pub fn endpoint(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    do_output(do_endpoint(attr.into(), item.into()))
}

fn do_endpoint(
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> Result<(proc_macro2::TokenStream, Vec<Error>), Error> {
    let metadata = from_tokenstream(&attr)?;
    // factored this way for now so #[channel] can use it too
    do_endpoint_inner(metadata, attr, item)
}

/// As with [`endpoint`], this attribute turns a handler function into a
/// Dropshot endpoint, but first wraps the handler function in such a way
/// that is spawned asynchronously and given the upgraded connection of
/// the given `protocol` (i.e. `WEBSOCKETS`).
///
/// The first argument still must be an `Arc<RequestContext<_>>`.
///
/// The second argument passed to the handler function must be a
/// [`dropshot::WebsocketConnection`].
///
/// The function must return a [`dropshot::WebsocketChannelResult`] (which is
/// a general-purpose `Result<(), Box<dyn Error + Send + Sync + 'static>>`).
/// Returned error values will be written to the RequestContext's log.
///
/// ```ignore
/// #[dropshot::channel { protocol = WEBSOCKETS, path = "/my/ws/channel/{id}" }]
/// ```
#[proc_macro_attribute]
pub fn channel(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    do_output(do_channel(attr.into(), item.into()))
}

fn do_channel(
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> Result<(proc_macro2::TokenStream, Vec<Error>), Error> {
    let ChannelMetadata {
        protocol,
        path,
        tags,
        unpublished,
        deprecated,
        _dropshot_crate,
    } = from_tokenstream(&attr)?;
    match protocol {
        ChannelProtocol::WEBSOCKETS => {
            // Here we construct a wrapper function and mutate the arguments a bit
            // for the outer layer: we replace WebsocketConnection, which is not
            // an extractor, with WebsocketUpgrade, which is.  We also move it
            // to the end.
            let ItemFnForSignature { attrs, vis, mut sig, _block: body } =
                syn::parse2(item)?;

            let inner_args = sig.inputs.clone();
            let inner_output = sig.output.clone();

            let arg_names: Vec<_> = inner_args
                .iter()
                .map(|arg: &syn::FnArg| match arg {
                    syn::FnArg::Receiver(r) => r.self_token.to_token_stream(),
                    syn::FnArg::Typed(syn::PatType { pat, .. }) => {
                        pat.to_token_stream()
                    }
                })
                .collect();
            let found = sig.inputs.iter_mut().nth(1).and_then(|arg| {
                if let syn::FnArg::Typed(syn::PatType { pat, ty, .. }) = arg {
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
            if found.is_none() {
                return Err(Error::new_spanned(
                    &attr,
                    "An argument of type dropshot::WebsocketConnection must be provided immediately following Arc<RequestContext<T>>.",
                ));
            }

            // Historically, we required that the `WebsocketConnection` argument
            // be first after the `RequestContext`.  However, we also require
            // that any exclusive extractor (which includes the
            // `WebsocketUpgrade` argument that we put in its place) appears
            // last.  We replaced the type above, but now we need to put it in
            // the right spot.
            sig.inputs = {
                let mut input_pairs =
                    sig.inputs.clone().into_pairs().collect::<Vec<_>>();
                let second_pair = input_pairs.remove(1);
                input_pairs.push(second_pair);
                input_pairs.into_iter().collect()
            };

            sig.output =
                syn::parse2(quote!(-> dropshot::WebsocketEndpointResult))?;

            let (conn_name, conn_type) = found.unwrap();

            let new_item = quote! {
                #(#attrs)*
                #vis #sig {
                    async fn __dropshot_websocket_handler(#inner_args) #inner_output #body
                    __dropshot_websocket_upgrade.handle(move | #conn_name: #conn_type | async move {
                        __dropshot_websocket_handler(#(#arg_names),*).await
                    })
                }
            };

            let metadata = EndpointMetadata {
                method: MethodType::GET,
                path,
                tags,
                unpublished,
                deprecated,
                content_type: Some("application/json".to_string()),
                _dropshot_crate,
            };
            do_endpoint_inner(metadata, attr, new_item)
        }
    }
}

fn do_output(
    res: Result<(proc_macro2::TokenStream, Vec<Error>), Error>,
) -> proc_macro::TokenStream {
    match res {
        Err(err) => err.to_compile_error().into(),
        Ok((endpoint, errors)) => {
            let compiler_errors =
                errors.iter().map(|err| err.to_compile_error());

            let output = quote! {
                #endpoint
                #( #compiler_errors )*
            };

            output.into()
        }
    }
}

fn do_endpoint_inner(
    metadata: EndpointMetadata,
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> Result<(proc_macro2::TokenStream, Vec<Error>), Error> {
    let ast: ItemFnForSignature = syn::parse2(item.clone())?;
    let method = metadata.method.as_str();
    let path = metadata.path;
    let content_type =
        metadata.content_type.unwrap_or_else(|| "application/json".to_string());
    if !matches!(
        content_type.as_str(),
        "application/json" | "application/x-www-form-urlencoded"
    ) {
        return Err(Error::new_spanned(
            &attr,
            "invalid content type for endpoint",
        ));
    }

    let mut errors = Vec::new();

    if ast.sig.constness.is_some() {
        errors.push(Error::new_spanned(
            &ast.sig.constness,
            "endpoint handlers may not be const functions",
        ));
    }

    if ast.sig.asyncness.is_none() {
        errors.push(Error::new_spanned(
            &ast.sig.fn_token,
            "endpoint handler functions must be async",
        ));
    }

    if ast.sig.unsafety.is_some() {
        errors.push(Error::new_spanned(
            &ast.sig.unsafety,
            "endpoint handlers may not be unsafe",
        ));
    }

    if ast.sig.abi.is_some() {
        errors.push(Error::new_spanned(
            &ast.sig.abi,
            "endpoint handler may not use an alternate ABI",
        ));
    }

    if !ast.sig.generics.params.is_empty() {
        errors.push(Error::new_spanned(
            &ast.sig.generics,
            "generics are not permitted for endpoint handlers",
        ));
    }

    if ast.sig.variadic.is_some() {
        errors
            .push(Error::new_spanned(&ast.sig.variadic, "no language C here"));
    }

    let name = &ast.sig.ident;
    let name_str = name.to_string();
    let method_ident = format_ident!("{}", method);
    let visibility = &ast.vis;

    let (summary_text, description_text) = extract_doc_from_attrs(&ast.attrs);
    let comment_text = {
        let mut buf = String::new();
        buf.push_str("API Endpoint: ");
        buf.push_str(&name_str);
        if let Some(s) = &summary_text {
            buf.push_str("\n");
            buf.push_str(&s);
        }
        if let Some(s) = &description_text {
            buf.push_str("\n");
            buf.push_str(&s);
        }
        buf
    };
    let description_doc_comment = quote! {
        #[doc = #comment_text]
    };

    let summary = summary_text.map(|summary| {
        quote! { .summary(#summary) }
    });
    let description = description_text.map(|description| {
        quote! { .description(#description) }
    });

    let tags = metadata
        .tags
        .iter()
        .map(|tag| {
            quote! {
                .tag(#tag)
            }
        })
        .collect::<Vec<_>>();

    let visible = if metadata.unpublished {
        quote! {
            .visible(false)
        }
    } else {
        quote! {}
    };

    let deprecated = if metadata.deprecated {
        quote! {
            .deprecated(true)
        }
    } else {
        quote! {}
    };

    let dropshot = get_crate(metadata._dropshot_crate);

    let first_arg = match ast.sig.inputs.first() {
        Some(syn::FnArg::Typed(syn::PatType {
            attrs: _,
            pat: _,
            colon_token: _,
            ty,
        })) => quote! {
                <#ty as #dropshot::RequestContextArgument>::Context
        },
        Some(first_arg @ syn::FnArg::Receiver(_)) => {
            errors.push(Error::new(
                first_arg.span(),
                "Expected a non-receiver argument",
            ));
            quote! { () }
        }
        None => {
            errors.push(Error::new(
                ast.sig.paren_token.span,
                "Endpoint requires arguments",
            ));
            quote! { () }
        }
    };

    // When the user attaches this proc macro to a function with the wrong type
    // signature, the resulting errors can be deeply inscrutable. To attempt to
    // make failures easier to understand, we inject code that asserts the types
    // of the various parameters. We do this by calling dummy functions that
    // require a type that satisfies SharedExtractor or ExclusiveExtractor.
    let mut arg_types = Vec::new();
    let mut arg_is_receiver = false;
    let param_checks = ast
        .sig
        .inputs
        .iter()
        .enumerate()
        .map(|(index, arg)| {
            match arg {
                syn::FnArg::Receiver(_) => {
                    // The compiler failure here is already comprehensible.
                    arg_is_receiver = true;
                    quote! {}
                }
                syn::FnArg::Typed(pat) => {
                    let span = pat.ty.span();
                    let ty = pat.ty.as_ref().into_token_stream();
                    arg_types.push(ty.clone());
                    if index == 0 {
                        // The first parameter must be an Arc<RequestContext<T>>
                        // and fortunately we already have a trait that we can
                        // use to validate this type.
                        quote_spanned! { span=>
                            const _: fn() = || {
                                struct NeedRequestContext(<#ty as #dropshot::RequestContextArgument>::Context);
                            };
                        }
                    } else if index < ast.sig.inputs.len() - 1 {
                        // Subsequent parameters aside from the last one must
                        // impl SharedExtractor.
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
                    } else {
                        // The final parameter must impl ExclusiveExtractor.
                        // (It's okay if it's another SharedExtractor.  Those
                        // impl ExclusiveExtractor, too.)
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
                    }
                }
            }
        })
        .collect::<Vec<_>>();

    // We want to construct a function that will call the user's endpoint, so
    // we can check the future it returns for bounds that otherwise produce
    // inscrutable error messages (like returning a non-`Send` future). We
    // produce a wrapper function that takes all the same argument types,
    // which requires building up a list of argument names: we can't use the
    // original definitions argument names since they could have multiple args
    // named `_`, so we use "arg0", "arg1", etc.
    let arg_names = (0..arg_types.len())
        .map(|i| {
            let argname = format_ident!("arg{}", i);
            quote! { #argname }
        })
        .collect::<Vec<_>>();
    let impl_checks = if !arg_is_receiver {
        quote! {
            const _: fn() = || {
                fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
                fn check_future_bounds(#( #arg_names: #arg_types ),*) {
                    future_endpoint_must_be_send(#name(#(#arg_names),*));
                }
            };
        }
    } else {
        // If we have a `self` arg, our `future_is_send` check will introduce
        // even more confusing error messages, so omit it entirely.
        quote! {}
    };

    let ret_check = match &ast.sig.output {
        syn::ReturnType::Default => {
            errors.push(Error::new_spanned(
                &ast.sig,
                "Endpoint must return a Result",
            ));
            quote! {}
        }
        syn::ReturnType::Type(_, ret_ty) => {
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
        }
    };

    // For reasons that are not well understood unused constants that use the
    // (default) call_site() Span do not trigger the dead_code lint. Because
    // defining but not using an endpoint is likely a programming error, we
    // want to be sure to have the compiler flag this. We force this by using
    // the span from the name of the function to which this macro was applied.
    let span = ast.sig.ident.span();
    let const_struct = quote_spanned! {span=>
        #visibility const #name: #name = #name {};
    };

    let construct = if errors.is_empty() {
        quote! {
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
        }
    } else {
        quote! {
            unreachable!()
        }
    };

    // The final TokenStream returned will have a few components that reference
    // `#name`, the name of the function to which this macro was applied...
    let stream = quote! {
        // ... type validation for parameter and return types
        #(#param_checks)*
        #ret_check

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
        impl From<#name>
            for #dropshot::ApiEndpoint< #first_arg >
        {
            fn from(_: #name) -> Self {
                #item

                // The checks on the implementation require #name to be in
                // scope, which is provided by #item, hence we place these
                // checks here instead of above with the others.
                #impl_checks

                #construct
            }
        }
    };

    // Prepend the usage message if any errors were detected.
    if !errors.is_empty() {
        errors.insert(0, Error::new_spanned(&ast.sig, USAGE));
    }

    if path.contains(":.*}") && !metadata.unpublished {
        errors.push(Error::new_spanned(
            &attr,
            "paths that contain a wildcard match must include 'unpublished = \
             true'",
        ));
    }

    Ok((stream, errors))
}

fn get_crate(var: Option<String>) -> proc_macro2::TokenStream {
    if let Some(s) = var {
        if let Ok(ts) = syn::parse_str(s.as_str()) {
            return ts;
        }
    }
    syn::Ident::new(DROPSHOT, proc_macro2::Span::call_site()).to_token_stream()
}

#[allow(dead_code)]
fn to_compile_errors(errors: Vec<syn::Error>) -> proc_macro2::TokenStream {
    let compile_errors = errors.iter().map(syn::Error::to_compile_error);
    quote!(#(#compile_errors)*)
}

fn extract_doc_from_attrs(
    attrs: &[syn::Attribute],
) -> (Option<String>, Option<String>) {
    let doc = syn::Ident::new("doc", proc_macro2::Span::call_site());

    let mut lines = attrs.iter().flat_map(|attr| {
        if let Ok(meta) = attr.parse_meta() {
            if let syn::Meta::NameValue(nv) = meta {
                if nv.path.is_ident(&doc) {
                    if let syn::Lit::Str(s) = nv.lit {
                        return normalize_comment_string(s.value());
                    }
                }
            }
        }
        Vec::new()
    });

    // Skip initial blank lines; they make for excessively terse summaries.
    let summary = loop {
        match lines.next() {
            Some(s) if s.is_empty() => (),
            next => break next,
        }
    };
    // Skip initial blank description lines.
    let first = loop {
        match lines.next() {
            Some(s) if s.is_empty() => (),
            next => break next,
        }
    };

    match (summary, first) {
        (None, _) => (None, None),
        (summary, None) => (summary, None),
        (Some(summary), Some(first)) => (
            Some(summary),
            Some(
                lines
                    .fold(first, |acc, comment| {
                        if acc.ends_with('-')
                            || acc.ends_with('\n')
                            || acc.is_empty()
                        {
                            // Continuation lines and newlines.
                            format!("{}{}", acc, comment)
                        } else if comment.is_empty() {
                            // Handle fully blank comments as newlines we keep.
                            format!("{}\n", acc)
                        } else {
                            // Default to space-separating comment fragments.
                            format!("{} {}", acc, comment)
                        }
                    })
                    .trim_end()
                    .to_string(),
            ),
        ),
    }
}
fn normalize_comment_string(s: String) -> Vec<String> {
    s.split('\n')
        .enumerate()
        .map(|(idx, s)| {
            // Rust-style comments are intrinsically single-line. We don't want
            // to trim away formatting such as an initial '*'.
            if idx == 0 {
                s.trim_start().trim_end()
            } else {
                let trimmed = s.trim_start().trim_end();
                trimmed.strip_prefix("* ").unwrap_or_else(|| {
                    trimmed.strip_prefix('*').unwrap_or(trimmed)
                })
            }
        })
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use schema::Schema;

    use super::*;

    #[test]
    fn test_endpoint_basic() {
        let (item, errors) = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c"
            },
            quote! {
                pub async fn handler_xyz(
                    _rqctx: Arc<RequestContext<()>>,
                ) -> Result<HttpResponseOk<()>, HttpError> {
                    Ok(())
                }
            },
        )
        .unwrap();
        let expected = quote! {
            const _: fn() = || {
                struct NeedRequestContext(<Arc<RequestContext<()> > as dropshot::RequestContextArgument>::Context) ;
            };
            const _: fn() = || {
                trait ResultTrait {
                    type T;
                    type E;
                }
                impl<TT, EE> ResultTrait for Result<TT, EE>
                where
                    TT: dropshot::HttpResponse,
                {
                    type T = TT;
                    type E = EE;
                }
                struct NeedHttpResponse(
                    <Result<HttpResponseOk<()>, HttpError> as ResultTrait>::T,
                );
                trait TypeEq {
                    type This: ?Sized;
                }
                impl<T: ?Sized> TypeEq for T {
                    type This = Self;
                }
                fn validate_result_error_type<T>()
                where
                    T: ?Sized + TypeEq<This = dropshot::HttpError>,
                {
                }
                validate_result_error_type::<
                    <Result<HttpResponseOk<()>, HttpError> as ResultTrait>::E,
                >();
            };

            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub struct handler_xyz {}

            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub const handler_xyz: handler_xyz = handler_xyz {};

            impl From<handler_xyz>
                for dropshot::ApiEndpoint<
                    <Arc<RequestContext<()>
                > as dropshot::RequestContextArgument>::Context>
            {
                fn from(_: handler_xyz) -> Self {
                    pub async fn handler_xyz(
                        _rqctx: Arc<RequestContext<()>>,
                    ) -> Result<HttpResponseOk<()>, HttpError> {
                        Ok(())
                    }

                    const _: fn() = || {
                        fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
                        fn check_future_bounds(arg0: Arc< RequestContext<()> >) {
                            future_endpoint_must_be_send(handler_xyz(arg0));
                        }
                    };

                    dropshot::ApiEndpoint::new(
                        "handler_xyz".to_string(),
                        handler_xyz,
                        dropshot::Method::GET,
                        "application/json",
                        "/a/b/c",
                    )
                }
            }
        };

        assert!(errors.is_empty());
        assert_eq!(expected.to_string(), item.to_string());
    }

    #[test]
    fn test_endpoint_context_fully_qualified_names() {
        let (item, errors) = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c"
            },
            quote! {
                pub async fn handler_xyz(_rqctx: std::sync::Arc<dropshot::RequestContext<()>>) ->
                std::Result<dropshot::HttpResponseOk<()>, dropshot::HttpError>
                {
                    Ok(())
                }
            },
        ).unwrap();
        let expected = quote! {
            const _: fn() = || {
                struct NeedRequestContext(<std::sync::Arc<dropshot::RequestContext<()> > as dropshot::RequestContextArgument>::Context) ;
            };
            const _: fn() = || {
                trait ResultTrait {
                    type T;
                    type E;
                }
                impl<TT, EE> ResultTrait for Result<TT, EE>
                where
                    TT: dropshot::HttpResponse,
                {
                    type T = TT;
                    type E = EE;
                }
                struct NeedHttpResponse(
                    <std::Result<dropshot::HttpResponseOk<()>, dropshot::HttpError> as ResultTrait>::T,
                );
                trait TypeEq {
                    type This: ?Sized;
                }
                impl<T: ?Sized> TypeEq for T {
                    type This = Self;
                }
                fn validate_result_error_type<T>()
                where
                    T: ?Sized + TypeEq<This = dropshot::HttpError>,
                {
                }
                validate_result_error_type::<
                    <std::Result<dropshot::HttpResponseOk<()>, dropshot::HttpError> as ResultTrait>::E,
                >();
            };

            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub struct handler_xyz {}

            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub const handler_xyz: handler_xyz = handler_xyz {};

            impl From<handler_xyz> for dropshot::ApiEndpoint< <std::sync::Arc<dropshot::RequestContext<()> > as dropshot::RequestContextArgument>::Context> {
                fn from(_: handler_xyz) -> Self {
                    pub async fn handler_xyz(_rqctx: std::sync::Arc<dropshot::RequestContext<()>>) ->
                        std::Result<dropshot::HttpResponseOk<()>, dropshot::HttpError>
                    {
                        Ok(())
                    }

                    const _: fn() = || {
                        fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
                        fn check_future_bounds(arg0: std::sync::Arc< dropshot::RequestContext<()> >) {
                            future_endpoint_must_be_send(handler_xyz(arg0));
                        }
                    };

                    dropshot::ApiEndpoint::new(
                        "handler_xyz".to_string(),
                        handler_xyz,
                        dropshot::Method::GET,
                        "application/json",
                        "/a/b/c",
                    )
                }
            }
        };

        assert!(errors.is_empty());
        assert_eq!(expected.to_string(), item.to_string());
    }

    #[test]
    fn test_endpoint_with_query() {
        let (item, errors) = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c"
            },
            quote! {
                async fn handler_xyz(
                    _rqctx: Arc<RequestContext<std::i32>>,
                    q: Query<Q>,
                ) -> Result<HttpResponseOk<()>, HttpError>
                {
                    Ok(())
                }
            },
        )
        .unwrap();
        let expected = quote! {
            const _: fn() = || {
                struct NeedRequestContext(<Arc<RequestContext<std::i32> > as dropshot::RequestContextArgument>::Context) ;
            };
            const _: fn() = || {
                fn need_exclusive_extractor<T>()
                where
                    T: ?Sized + dropshot::ExclusiveExtractor,
                {
                }
                need_exclusive_extractor::<Query<Q> >();
            };
            const _: fn() = || {
                trait ResultTrait {
                    type T;
                    type E;
                }
                impl<TT, EE> ResultTrait for Result<TT, EE>
                where
                    TT: dropshot::HttpResponse,
                {
                    type T = TT;
                    type E = EE;
                }
                struct NeedHttpResponse(
                    <Result<HttpResponseOk<()>, HttpError> as ResultTrait>::T,
                );
                trait TypeEq {
                    type This: ?Sized;
                }
                impl<T: ?Sized> TypeEq for T {
                    type This = Self;
                }
                fn validate_result_error_type<T>()
                where
                    T: ?Sized + TypeEq<This = dropshot::HttpError>,
                {
                }
                validate_result_error_type::<
                    <Result<HttpResponseOk<()>, HttpError> as ResultTrait>::E,
                >();
            };

            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            struct handler_xyz {}

            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            const handler_xyz: handler_xyz = handler_xyz {};

            impl From<handler_xyz>
                for dropshot::ApiEndpoint<
                    <Arc<RequestContext<std::i32> > as dropshot::RequestContextArgument>::Context
                >
            {
                fn from(_: handler_xyz) -> Self {
                    async fn handler_xyz(
                        _rqctx: Arc<RequestContext<std::i32>>,
                        q: Query<Q>,
                    ) ->
                        Result<HttpResponseOk<()>, HttpError>
                    {
                        Ok(())
                    }

                    const _: fn() = || {
                        fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
                        fn check_future_bounds(arg0: Arc< RequestContext<std::i32> >, arg1: Query<Q>) {
                            future_endpoint_must_be_send(handler_xyz(arg0, arg1));
                        }
                    };

                    dropshot::ApiEndpoint::new(
                        "handler_xyz".to_string(),
                        handler_xyz,
                        dropshot::Method::GET,
                        "application/json",
                        "/a/b/c",
                    )
                }
            }
        };

        assert!(errors.is_empty());
        assert_eq!(expected.to_string(), item.to_string());
    }

    #[test]
    fn test_endpoint_pub_crate() {
        let (item, errors) = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c"
            },
            quote! {
                pub(crate) async fn handler_xyz(
                    _rqctx: Arc<RequestContext<()>>,
                    q: Query<Q>,
                ) -> Result<HttpResponseOk<()>, HttpError>
                {
                    Ok(())
                }
            },
        )
        .unwrap();
        let expected = quote! {
            const _: fn() = || {
                struct NeedRequestContext(<Arc<RequestContext<()> > as dropshot::RequestContextArgument>::Context) ;
            };
            const _: fn() = || {
                fn need_exclusive_extractor<T>()
                where
                    T: ?Sized + dropshot::ExclusiveExtractor,
                {
                }
                need_exclusive_extractor::<Query<Q> >();
            };
            const _: fn() = || {
                trait ResultTrait {
                    type T;
                    type E;
                }
                impl<TT, EE> ResultTrait for Result<TT, EE>
                where
                    TT: dropshot::HttpResponse,
                {
                    type T = TT;
                    type E = EE;
                }
                struct NeedHttpResponse(
                    <Result<HttpResponseOk<()>, HttpError> as ResultTrait>::T,
                );
                trait TypeEq {
                    type This: ?Sized;
                }
                impl<T: ?Sized> TypeEq for T {
                    type This = Self;
                }
                fn validate_result_error_type<T>()
                where
                    T: ?Sized + TypeEq<This = dropshot::HttpError>,
                {
                }
                validate_result_error_type::<
                    <Result<HttpResponseOk<()>, HttpError> as ResultTrait>::E,
                >();
            };

            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub(crate) struct handler_xyz {}

            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub(crate) const handler_xyz: handler_xyz = handler_xyz {};

            impl From<handler_xyz>
                for dropshot::ApiEndpoint<
                    <Arc<RequestContext<()> > as dropshot::RequestContextArgument>::Context
                >
            {
                fn from(_: handler_xyz) -> Self {
                    pub(crate) async fn handler_xyz(
                        _rqctx: Arc<RequestContext<()>>,
                        q: Query<Q>,
                    ) ->
                        Result<HttpResponseOk<()>, HttpError>
                    {
                        Ok(())
                    }

                    const _: fn() = || {
                        fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
                        fn check_future_bounds(arg0: Arc< RequestContext<()> >, arg1: Query<Q>) {
                            future_endpoint_must_be_send(handler_xyz(arg0, arg1));
                        }
                    };

                    dropshot::ApiEndpoint::new(
                        "handler_xyz".to_string(),
                        handler_xyz,
                        dropshot::Method::GET,
                        "application/json",
                        "/a/b/c",
                    )
                }
            }
        };

        assert!(errors.is_empty());
        assert_eq!(expected.to_string(), item.to_string());
    }

    #[test]
    fn test_endpoint_with_tags() {
        let (item, errors) = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c",
                tags = ["stuff", "things"],
            },
            quote! {
                async fn handler_xyz(
                    _rqctx: Arc<RequestContext<()>>,
                ) -> Result<HttpResponseOk<()>, HttpError> {
                    Ok(())
                }
            },
        )
        .unwrap();
        let expected = quote! {
            const _: fn() = || {
                struct NeedRequestContext(<Arc<RequestContext<()> > as dropshot::RequestContextArgument>::Context) ;
            };
            const _: fn() = || {
                trait ResultTrait {
                    type T;
                    type E;
                }
                impl<TT, EE> ResultTrait for Result<TT, EE>
                where
                    TT: dropshot::HttpResponse,
                {
                    type T = TT;
                    type E = EE;
                }
                struct NeedHttpResponse(
                    <Result<HttpResponseOk<()>, HttpError> as ResultTrait>::T,
                );
                trait TypeEq {
                    type This: ?Sized;
                }
                impl<T: ?Sized> TypeEq for T {
                    type This = Self;
                }
                fn validate_result_error_type<T>()
                where
                    T: ?Sized + TypeEq<This = dropshot::HttpError>,
                {
                }
                validate_result_error_type::<
                    <Result<HttpResponseOk<()>, HttpError> as ResultTrait>::E,
                >();
            };

            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            struct handler_xyz {}

            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            const handler_xyz: handler_xyz = handler_xyz {};

            impl From<handler_xyz>
                for dropshot::ApiEndpoint<
                    <Arc<RequestContext<()>
                > as dropshot::RequestContextArgument>::Context>
            {
                fn from(_: handler_xyz) -> Self {
                    async fn handler_xyz(
                        _rqctx: Arc<RequestContext<()>>,
                    ) -> Result<HttpResponseOk<()>, HttpError> {
                        Ok(())
                    }

                    const _: fn() = || {
                        fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
                        fn check_future_bounds(arg0: Arc< RequestContext<()> >) {
                            future_endpoint_must_be_send(handler_xyz(arg0));
                        }
                    };

                    dropshot::ApiEndpoint::new(
                        "handler_xyz".to_string(),
                        handler_xyz,
                        dropshot::Method::GET,
                        "application/json",
                        "/a/b/c",
                    )
                    .tag("stuff")
                    .tag("things")
                }
            }
        };

        assert!(errors.is_empty());
        assert_eq!(expected.to_string(), item.to_string());
    }

    #[test]
    fn test_endpoint_with_doc() {
        let (item, errors) = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c"
            },
            quote! {
                /** handle "xyz" requests */
                async fn handler_xyz(
                    _rqctx: Arc<RequestContext<()>>,
                ) -> Result<HttpResponseOk<()>, HttpError> {
                    Ok(())
                }
            },
        )
        .unwrap();
        let expected = quote! {
            const _: fn() = || {
                struct NeedRequestContext(<Arc<RequestContext<()> > as dropshot::RequestContextArgument>::Context) ;
            };
            const _: fn() = || {
                trait ResultTrait {
                    type T;
                    type E;
                }
                impl<TT, EE> ResultTrait for Result<TT, EE>
                where
                    TT: dropshot::HttpResponse,
                {
                    type T = TT;
                    type E = EE;
                }
                struct NeedHttpResponse(
                    <Result<HttpResponseOk<()>, HttpError> as ResultTrait>::T,
                );
                trait TypeEq {
                    type This: ?Sized;
                }
                impl<T: ?Sized> TypeEq for T {
                    type This = Self;
                }
                fn validate_result_error_type<T>()
                where
                    T: ?Sized + TypeEq<This = dropshot::HttpError>,
                {
                }
                validate_result_error_type::<
                    <Result<HttpResponseOk<()>, HttpError> as ResultTrait>::E,
                >();
            };

            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handler_xyz\nhandle \"xyz\" requests"]
            struct handler_xyz {}

            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handler_xyz\nhandle \"xyz\" requests"]
            const handler_xyz: handler_xyz = handler_xyz {};

            impl From<handler_xyz>
                for dropshot::ApiEndpoint<
                    <Arc<RequestContext<()>
                > as dropshot::RequestContextArgument>::Context>
            {
                fn from(_: handler_xyz) -> Self {
                    #[doc = r#" handle "xyz" requests "#]
                    async fn handler_xyz(
                        _rqctx: Arc<RequestContext<()>>,
                    ) -> Result<HttpResponseOk<()>, HttpError> {
                        Ok(())
                    }

                    const _: fn() = || {
                        fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
                        fn check_future_bounds(arg0: Arc< RequestContext<()> >) {
                            future_endpoint_must_be_send(handler_xyz(arg0));
                        }
                    };

                    dropshot::ApiEndpoint::new(
                        "handler_xyz".to_string(),
                        handler_xyz,
                        dropshot::Method::GET,
                        "application/json",
                        "/a/b/c",
                    )
                    .summary("handle \"xyz\" requests")
                }
            }
        };

        assert!(errors.is_empty());
        assert_eq!(expected.to_string(), item.to_string());
    }

    #[test]
    fn test_endpoint_invalid_item() {
        let ret = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c"
            },
            quote! {
                const POTATO = "potato";
            },
        );

        let msg = format!("{}", ret.err().unwrap());
        assert_eq!("expected `fn`", msg);
    }

    #[test]
    fn test_endpoint_bad_string() {
        let ret = do_endpoint(
            quote! {
                method = GET,
                path = /a/b/c
            },
            quote! {
                const POTATO = "potato";
            },
        );

        let msg = format!("{}", ret.err().unwrap());
        assert_eq!("expected a string, but found `/`", msg);
    }

    #[test]
    fn test_endpoint_bad_metadata() {
        let ret = do_endpoint(
            quote! {
                methud = GET,
                path = "/a/b/c"
            },
            quote! {
                const POTATO = "potato";
            },
        );

        let msg = format!("{}", ret.err().unwrap());
        assert_eq!("extraneous member `methud`", msg);
    }

    #[test]
    fn test_endpoint_not_async() {
        let (_, errors) = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c",
            },
            quote! {
                fn handler_xyz(_rqctx: Arc<RequestContext>) {}
            },
        )
        .unwrap();

        assert!(!errors.is_empty());
        assert_eq!(
            errors.get(1).map(ToString::to_string),
            Some("endpoint handler functions must be async".to_string())
        );
    }

    #[test]
    fn test_endpoint_bad_context_receiver() {
        let (_, errors) = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c",
            },
            quote! {
                async fn handler_xyz(&self) {}
            },
        )
        .unwrap();

        assert!(!errors.is_empty());
        assert_eq!(
            errors.get(1).map(ToString::to_string),
            Some("Expected a non-receiver argument".to_string())
        );
    }

    #[test]
    fn test_endpoint_no_arguments() {
        let (_, errors) = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c",
            },
            quote! {
                async fn handler_xyz() {}
            },
        )
        .unwrap();

        assert!(!errors.is_empty());
        assert_eq!(
            errors.get(1).map(ToString::to_string),
            Some("Endpoint requires arguments".to_string())
        );
    }

    #[test]
    fn test_endpoint_content_type() {
        let (item, errors) = do_endpoint(
            quote! {
                method = POST,
                path = "/a/b/c",
                content_type = "application/x-www-form-urlencoded"
            },
            quote! {
                pub async fn handler_xyz(
                    _rqctx: Arc<RequestContext<()>>,
                ) -> Result<HttpResponseOk<()>, HttpError> {
                    Ok(())
                }
            },
        )
        .unwrap();

        let expected = quote! {
            const _: fn() = || {
                struct NeedRequestContext(<Arc<RequestContext<()> > as dropshot::RequestContextArgument>::Context) ;
            };
            const _: fn() = || {
                trait ResultTrait {
                    type T;
                    type E;
                }
                impl<TT, EE> ResultTrait for Result<TT, EE>
                where
                    TT: dropshot::HttpResponse,
                {
                    type T = TT;
                    type E = EE;
                }
                struct NeedHttpResponse(
                    <Result<HttpResponseOk<()>, HttpError> as ResultTrait>::T,
                );
                trait TypeEq {
                    type This: ?Sized;
                }
                impl<T: ?Sized> TypeEq for T {
                    type This = Self;
                }
                fn validate_result_error_type<T>()
                where
                    T: ?Sized + TypeEq<This = dropshot::HttpError>,
                {
                }
                validate_result_error_type::<
                    <Result<HttpResponseOk<()>, HttpError> as ResultTrait>::E,
                >();
            };

            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub struct handler_xyz {}

            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub const handler_xyz: handler_xyz = handler_xyz {};

            impl From<handler_xyz>
                for dropshot::ApiEndpoint<
                    <Arc<RequestContext<()>
                > as dropshot::RequestContextArgument>::Context>
            {
                fn from(_: handler_xyz) -> Self {
                    pub async fn handler_xyz(
                        _rqctx: Arc<RequestContext<()>>,
                    ) -> Result<HttpResponseOk<()>, HttpError> {
                        Ok(())
                    }

                    const _: fn() = || {
                        fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
                        fn check_future_bounds(arg0: Arc< RequestContext<()> >) {
                            future_endpoint_must_be_send(handler_xyz(arg0));
                        }
                    };

                    dropshot::ApiEndpoint::new(
                        "handler_xyz".to_string(),
                        handler_xyz,
                        dropshot::Method::POST,
                        "application/x-www-form-urlencoded",
                        "/a/b/c",
                    )
                }
            }
        };

        assert!(errors.is_empty());
        assert_eq!(expected.to_string(), item.to_string());
    }

    #[test]
    fn test_extract_summary_description() {
        /// Javadoc summary
        /// Maybe there's another name for these...
        /// ... but Java is the first place I saw these types of comments.
        #[derive(Schema)]
        struct JavadocComments;
        assert_eq!(
            extract_doc_from_attrs(&JavadocComments::schema().attrs),
            (
                Some("Javadoc summary".to_string()),
                Some(
                    "Maybe there's another name for these... ... but Java \
                    is the first place I saw these types of comments."
                        .to_string()
                )
            )
        );

        /// Javadoc summary
        ///
        /// Skip that blank.
        #[derive(Schema)]
        struct JavadocCommentsWithABlank;
        assert_eq!(
            extract_doc_from_attrs(&JavadocCommentsWithABlank::schema().attrs),
            (
                Some("Javadoc summary".to_string()),
                Some("Skip that blank.".to_string())
            )
        );

        /// Terse Javadoc summary
        #[derive(Schema)]
        struct JavadocCommentsTerse;
        assert_eq!(
            extract_doc_from_attrs(&JavadocCommentsTerse::schema().attrs),
            (Some("Terse Javadoc summary".to_string()), None)
        );

        /// Rustdoc summary
        /// Did other folks do this or what this an invention I can right-
        /// fully ascribe to Rust?
        #[derive(Schema)]
        struct RustdocComments;
        assert_eq!(
            extract_doc_from_attrs(&RustdocComments::schema().attrs),
            (
                Some("Rustdoc summary".to_string()),
                Some(
                    "Did other folks do this or what this an invention \
                    I can right-fully ascribe to Rust?"
                        .to_string()
                )
            )
        );

        /// Rustdoc summary
        ///
        /// Skip that blank.
        #[derive(Schema)]
        struct RustdocCommentsWithABlank;
        assert_eq!(
            extract_doc_from_attrs(&RustdocCommentsWithABlank::schema().attrs),
            (
                Some("Rustdoc summary".to_string()),
                Some("Skip that blank.".to_string())
            )
        );

        /// Just a Rustdoc summary
        #[derive(Schema)]
        struct JustTheRustdocSummary;
        assert_eq!(
            extract_doc_from_attrs(&JustTheRustdocSummary::schema().attrs),
            (Some("Just a Rustdoc summary".to_string()), None)
        );

        /// Just a Javadoc summary
        #[derive(Schema)]
        struct JustTheJavadocSummary;
        assert_eq!(
            extract_doc_from_attrs(&JustTheJavadocSummary::schema().attrs),
            (Some("Just a Javadoc summary".to_string()), None)
        );

        /// Summary
        /// Text
        /// More
        ///
        /// Even
        /// More
        #[derive(Schema)]
        struct SummaryDescriptionBreak;
        assert_eq!(
            extract_doc_from_attrs(&SummaryDescriptionBreak::schema().attrs),
            (
                Some("Summary".to_string()),
                Some("Text More\nEven More".to_string())
            )
        );
    }
}
