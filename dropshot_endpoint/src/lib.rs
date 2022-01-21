// Copyright 2021 Oxide Computer Company

//! This package defines macro attributes associated with HTTP handlers. These
//! attributes are used both to define an HTTP API and to generate an OpenAPI
//! Spec (OAS) v3 document that describes the API.

/*
 * Clippy's style advice is definitely valuable, but not worth the trouble for
 * automated enforcement.
 */
#![allow(clippy::style)]

use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use quote::{quote_spanned, ToTokens};
use serde::Deserialize;
use serde_tokenstream::from_tokenstream;
use serde_tokenstream::Error;
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
}

impl MethodType {
    fn as_str(&self) -> &'static str {
        match self {
            MethodType::DELETE => "DELETE",
            MethodType::GET => "GET",
            MethodType::PATCH => "PATCH",
            MethodType::POST => "POST",
            MethodType::PUT => "PUT",
        }
    }
}

#[derive(Deserialize, Debug)]
struct Metadata {
    method: MethodType,
    path: String,
    tags: Option<Vec<String>>,
    unpublished: Option<bool>,
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
///     method = { DELETE | GET | PATCH | POST | PUT },
///     path = "/path/name/with/{named}/{variables}",
///
///     // Optional tags for the API description
///     tags = [ "all", "your", "OpenAPI", "tags" ],
///     // A value of `true` causes the API to be omitted from the API description
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
    match do_endpoint(attr.into(), item.into()) {
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

fn do_endpoint(
    attr: TokenStream,
    item: TokenStream,
) -> Result<(TokenStream, Vec<Error>), Error> {
    let metadata = from_tokenstream::<Metadata>(&attr)?;

    let method = metadata.method.as_str();
    let path = metadata.path;

    let ast: ItemFnForSignature = syn::parse2(item.clone())?;

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
        .map(|v| {
            v.iter()
                .map(|tag| {
                    quote! {
                        .tag(#tag)
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let visible = if let Some(true) = metadata.unpublished {
        quote! {
            .visible(false)
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
    // of the various parameters. We do this by calling a dummy function that
    // requires a type that satisfies the trait Extractor.
    let param_checks = ast
        .sig
        .inputs
        .iter()
        .enumerate()
        .map(|(index, arg)| {
            match arg {
                syn::FnArg::Receiver(_) => {
                    // The compiler failure here is already comprehensible.
                    quote! {}
                }
                syn::FnArg::Typed(pat) => {
                    let span = pat.ty.span();
                    let ty = pat.ty.as_ref().into_token_stream();
                    if index == 0 {
                        // The first parameter must be an Arc<RequestContext<T>>
                        // and fortunately we already have a trait that we can
                        // use to validate this type.
                        quote_spanned! { span=>
                            const _: fn() = || {
                                struct NeedRequestContext(<#ty as #dropshot::RequestContextArgument>::Context);
                            };
                        }
                    } else {
                        // Subsequent parameters must implement Extractor.
                        quote_spanned! { span=>
                            const _: fn() = || {
                                fn need_extractor<T>()
                                where
                                    T: ?Sized + #dropshot::Extractor,
                                {
                                }
                                need_extractor::<#ty>();
                            };
                        }
                    }
                }
            }
        })
        .collect::<Vec<_>>();

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
                #path,
            )
            #summary
            #description
            #(#tags)*
            #visible
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

                #construct
            }
        }
    };

    // Prepend the usage message if any errors were detected.
    if !errors.is_empty() {
        errors.insert(0, Error::new_spanned(&ast.sig, USAGE));
    }

    if path.contains(":.*}") && metadata.unpublished != Some(true) {
        errors.push(Error::new_spanned(
            &attr,
            "paths that contain a wildcard match must include 'unpublished = \
             true'",
        ));
    }

    Ok((stream, errors))
}

fn get_crate(var: Option<String>) -> TokenStream {
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

    let summary = lines.next();
    let first = lines.next();

    match (summary, first) {
        (None, _) => (None, None),
        (summary, None) => (summary, None),
        (Some(summary), Some(first)) => (
            Some(summary),
            Some(lines.fold(first, |acc, comment| {
                if acc.ends_with('-') || acc.is_empty() {
                    format!("{}{}", acc, comment)
                } else {
                    format!("{} {}", acc, comment)
                }
            })),
        ),
    }
}
fn normalize_comment_string(s: String) -> Vec<String> {
    let mut ret = s
        .split('\n')
        .enumerate()
        .map(|(idx, s)| {
            if idx == 0 {
                s.trim_start()
            } else {
                let trimmed = s.trim_start();
                trimmed.strip_prefix("* ").unwrap_or_else(|| {
                    trimmed.strip_prefix('*').unwrap_or(trimmed)
                })
            }
        })
        .map(|s| s.trim_end())
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    loop {
        match ret.first() {
            Some(s) if s.is_empty() => {
                let _ = ret.remove(0);
            }
            _ => break,
        }
    }

    while ret.last().map(|s| s.is_empty()) == Some(true) {
        ret.pop();
    }

    ret
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
                    dropshot::ApiEndpoint::new(
                        "handler_xyz".to_string(),
                        handler_xyz,
                        dropshot::Method::GET,
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
                    dropshot::ApiEndpoint::new(
                        "handler_xyz".to_string(),
                        handler_xyz,
                        dropshot::Method::GET,
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
                fn need_extractor<T>()
                where
                    T: ?Sized + dropshot::Extractor,
                {
                }
                need_extractor::<Query<Q> >();
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
                    dropshot::ApiEndpoint::new(
                        "handler_xyz".to_string(),
                        handler_xyz,
                        dropshot::Method::GET,
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
                fn need_extractor<T>()
                where
                    T: ?Sized + dropshot::Extractor,
                {
                }
                need_extractor::<Query<Q> >();
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
                    dropshot::ApiEndpoint::new(
                        "handler_xyz".to_string(),
                        handler_xyz,
                        dropshot::Method::GET,
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
                    dropshot::ApiEndpoint::new(
                        "handler_xyz".to_string(),
                        handler_xyz,
                        dropshot::Method::GET,
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
            #[doc = "API Endpoint: handle \"xyz\" requests"]
            struct handler_xyz {}

            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handle \"xyz\" requests"]
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
                    dropshot::ApiEndpoint::new(
                        "handler_xyz".to_string(),
                        handler_xyz,
                        dropshot::Method::GET,
                        "/a/b/c",
                    )
                    .description("handle \"xyz\" requests")
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
    fn test_extract_summary_description() {
        /**
         * Javadoc summary
         * Maybe there's another name for these...
         * ... but Java is the first place I saw these types of comments.
         */
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

        /**
         * Javadoc summary
         *
         * Skip that blank.
         */
        #[derive(Schema)]
        struct JavadocCommentsWithABlank;
        assert_eq!(
            extract_doc_from_attrs(&JavadocCommentsWithABlank::schema().attrs),
            (
                Some("Javadoc summary".to_string()),
                Some("Skip that blank.".to_string())
            )
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

        /// Just a summary
        #[derive(Schema)]
        struct JustTheSummary;
        assert_eq!(
            extract_doc_from_attrs(&JustTheSummary::schema().attrs),
            (Some("Just a summary".to_string()), None)
        );
    }
}
