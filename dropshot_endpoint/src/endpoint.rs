// Copyright 2023 Oxide Computer Company

//! Support for HTTP `#[endpoint]` macros.

use quote::format_ident;
use quote::quote;
use quote::{quote_spanned, ToTokens};
use serde::Deserialize;
use serde_tokenstream::from_tokenstream;
use serde_tokenstream::Error;
use syn::spanned::Spanned;

use crate::syn_parsing::ItemFnForSignature;
use crate::util::{extract_doc_from_attrs, get_crate};

/// Endpoint usage message, produced if there were syntax errors.
const USAGE: &str = "Endpoint handlers must have the following signature:
    async fn(
        rqctx: dropshot::RequestContext<MyContext>,
        [query_params: Query<Q>,]
        [path_params: Path<P>,]
        [body_param: TypedBody<J>,]
        [body_param: UntypedBody,]
        [body_param: StreamingBody,]
        [raw_request: RawRequest,]
    ) -> Result<HttpResponse*, HttpError>";

pub(crate) fn do_endpoint(
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> Result<(proc_macro2::TokenStream, Vec<Error>), Error> {
    let metadata = from_tokenstream(&attr)?;
    // factored this way for now so #[channel] can use it too
    do_endpoint_inner(metadata, attr, item)
}

pub(crate) fn do_endpoint_inner(
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
        "application/json"
            | "application/x-www-form-urlencoded"
            | "multipart/form-data"
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
            quote! { .tag(#tag) }
        })
        .collect::<Vec<_>>();

    let visible = metadata.unpublished.then(|| {
        quote! { .visible(false) }
    });

    let deprecated = metadata.deprecated.then(|| {
        quote! { .deprecated(true) }
    });

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
                ast.sig.paren_token.span.join(),
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
                        // The first parameter must be a RequestContext<T>
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

    // If we have a `self` arg, this check would introduce even more confusing
    // error messages, so we only include it if there is no receiver.
    let impl_checks = (!arg_is_receiver).then(||
        quote! {
            const _: fn() = || {
                fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
                fn check_future_bounds(#( #arg_names: #arg_types ),*) {
                    future_endpoint_must_be_send(#name(#(#arg_names),*));
                }
            };
        }
    );

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
                #[allow(clippy::unused_async)]
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

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub(crate) enum MethodType {
    DELETE,
    GET,
    HEAD,
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
            MethodType::HEAD => "HEAD",
            MethodType::PATCH => "PATCH",
            MethodType::POST => "POST",
            MethodType::PUT => "PUT",
            MethodType::OPTIONS => "OPTIONS",
        }
    }
}

#[derive(Deserialize, Debug)]
pub(crate) struct EndpointMetadata {
    pub(crate) method: MethodType,
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) unpublished: bool,
    #[serde(default)]
    pub(crate) deprecated: bool,
    pub(crate) content_type: Option<String>,
    pub(crate) _dropshot_crate: Option<String>,
}

#[cfg(test)]
mod tests {
    use expectorate::assert_contents;
    use syn::parse_quote;

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
                    _rqctx: RequestContext<()>,
                ) -> Result<HttpResponseOk<()>, HttpError> {
                    Ok(())
                }
            },
        )
        .unwrap();

        assert!(errors.is_empty());
        assert_contents(
            "tests/output/endpoint_basic.rs",
            &prettyplease::unparse(&parse_quote! { #item }),
        );
    }

    #[test]
    fn test_endpoint_context_fully_qualified_names() {
        let (item, errors) = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c"
            },
            quote! {
                pub async fn handler_xyz(_rqctx: dropshot::RequestContext<()>) ->
                std::Result<dropshot::HttpResponseOk<()>, dropshot::HttpError>
                {
                    Ok(())
                }
            },
        ).unwrap();

        assert!(errors.is_empty());
        assert_contents(
            "tests/output/endpoint_context_fully_qualified_names.rs",
            &prettyplease::unparse(&parse_quote! { #item }),
        );
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
                    _rqctx: RequestContext<std::i32>,
                    q: Query<Q>,
                ) -> Result<HttpResponseOk<()>, HttpError>
                {
                    Ok(())
                }
            },
        )
        .unwrap();

        assert!(errors.is_empty());
        assert_contents(
            "tests/output/endpoint_with_query.rs",
            &prettyplease::unparse(&parse_quote! { #item }),
        );
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
                    _rqctx: RequestContext<()>,
                    q: Query<Q>,
                ) -> Result<HttpResponseOk<()>, HttpError>
                {
                    Ok(())
                }
            },
        )
        .unwrap();

        assert!(errors.is_empty());
        assert_contents(
            "tests/output/endpoint_pub_crate.rs",
            &prettyplease::unparse(&parse_quote! { #item }),
        );
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
                    _rqctx: RequestContext<()>,
                ) -> Result<HttpResponseOk<()>, HttpError> {
                    Ok(())
                }
            },
        )
        .unwrap();

        assert!(errors.is_empty());
        assert_contents(
            "tests/output/endpoint_with_tags.rs",
            &prettyplease::unparse(&parse_quote! { #item }),
        );
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
                    _rqctx: RequestContext<()>,
                ) -> Result<HttpResponseOk<()>, HttpError> {
                    Ok(())
                }
            },
        )
        .unwrap();

        assert!(errors.is_empty());
        assert_contents(
            "tests/output/endpoint_with_doc.rs",
            &prettyplease::unparse(&parse_quote! { #item }),
        );
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
                fn handler_xyz(_rqctx: RequestContext) {}
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
                    _rqctx: RequestContext<()>,
                ) -> Result<HttpResponseOk<()>, HttpError> {
                    Ok(())
                }
            },
        )
        .unwrap();

        let expected = quote! {
            const _: fn() = || {
                struct NeedRequestContext(<RequestContext<()> as dropshot::RequestContextArgument>::Context) ;
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
                    <RequestContext<()>
                as dropshot::RequestContextArgument>::Context>
            {
                fn from(_: handler_xyz) -> Self {
                    #[allow(clippy::unused_async)]
                    pub async fn handler_xyz(
                        _rqctx: RequestContext<()>,
                    ) -> Result<HttpResponseOk<()>, HttpError> {
                        Ok(())
                    }

                    const _: fn() = || {
                        fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
                        fn check_future_bounds(arg0: RequestContext<()>) {
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
}
