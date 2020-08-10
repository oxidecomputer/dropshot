// Copyright 2020 Oxide Computer Company
//! This macro defines attributes associated with HTTP handlers. These
//! attributes are used both to define an HTTP API and to generate an OpenAPI
//! Spec (OAS) v3 document that describes the API.

extern crate proc_macro;

use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use quote::{quote_spanned, ToTokens};
use serde::Deserialize;
use serde_tokenstream::from_tokenstream;
use serde_tokenstream::Error;
use syn::ItemFn;

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
    _dropshot_crate: Option<String>,
}

const DROPSHOT: &str = "dropshot";

/// Attribute to apply to an HTTP endpoint.
/// TODO(doc) explain intended use
#[proc_macro_attribute]
pub fn endpoint(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    match do_endpoint(attr.into(), item.into()) {
        Ok(result) => result.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn do_endpoint(
    attr: TokenStream,
    item: TokenStream,
) -> Result<TokenStream, Error> {
    let metadata = from_tokenstream::<Metadata>(&attr)?;

    let method = metadata.method.as_str();
    let path = metadata.path;

    let ast: ItemFn = syn::parse2(item.clone())?;

    let name = &ast.sig.ident;
    let name_str = name.to_string();
    let method_ident = format_ident!("{}", method);

    let description_text_provided = extract_doc_from_attrs(&ast.attrs);
    let description_text_annotated = format!(
        "API Endpoint: {}",
        description_text_provided.as_ref().unwrap_or(&name_str).as_str().trim()
    );
    let description_doc_comment = quote! {
        #[doc = #description_text_annotated]
    };
    let description = description_text_provided.map(|description| {
        quote! {
            .description(#description)
        }
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

    let dropshot = get_crate(metadata._dropshot_crate);

    // When the user attaches this proc macro to a function with the wrong type
    // signature, the resulting errors are deeply inscrutable. To attempt to
    // make failures easier to understand, we inject code that asserts the types
    // of the various parameters. For the first parameter of type
    // Arc<RequestContext>, we turn that type into a trait and then construct
    // a dummy function that requires a type match. It will fail for anything
    // of the wrong type. Subsequent parameters are simpler: we simply need to
    // call a dummy function that requires a type that satisfies the trait
    // Extractor.
    let mut checks = ast
        .sig
        .inputs
        .iter()
        .enumerate()
        .map(|(i, arg)| {
            let req = if i == 0 {
                quote! { std::sync::Arc<#dropshot::RequestContext> }
            } else {
                quote! { #dropshot::Extractor }
            };

            match arg {
                syn::FnArg::Receiver(_) => {
                    // The compiler failure here is already comprehensible.
                    quote! {}
                }
                syn::FnArg::Typed(pat) => {
                    let span = Error::new_spanned(pat.ty.as_ref(), "").span();
                    let ty = pat.ty.as_ref().into_token_stream();

                    if i == 0 {
                        quote_spanned! { span=>
                            const _: fn() = ||{
                                trait TypeEq {
                                    type This: ?Sized;
                                }
                                impl<T: ?Sized> TypeEq for T {
                                    type This = Self;
                                }
                                fn need_arc_requestcontext<T>()
                                where
                                    T: ?Sized + TypeEq<This = #req>,
                                {
                                }
                                need_arc_requestcontext::<#ty>();
                            };
                        }
                    } else {
                        quote_spanned! { span=>
                            const _: fn() = ||{
                                fn need_extractor<T>()
                                where
                                    T: ?Sized + #req,
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

    // Help the user if they don't give any parameters.
    if ast.sig.inputs.len() == 0 {
        checks.push(
            Error::new_spanned(
                (&ast.sig).into_token_stream(),
                "incompatible function signature; expected async fn \
                 (Arc<RequestContext>(, Extractor)*) -> Result<HttpResponse, \
                 HttpError>",
            )
            .to_compile_error(),
        );
    }

    // The final TokenStream returned will have a few components that reference
    // `#name`, the name of the method to which this macro was applied...
    let stream = quote! {
        #(#checks)*

        // ... a struct type called `#name` that has no members
        #[allow(non_camel_case_types, missing_docs)]
        #description_doc_comment
        pub struct #name {}
        // ... a constant of type `#name` whose identifier is also #name
        #[allow(non_upper_case_globals, missing_docs)]
        #description_doc_comment
        const #name: #name = #name {};

        // ... an impl of `From<#name>` for ApiEndpoint that allows the constant
        // `#name` to be passed into `ApiDescription::register()`
        impl From<#name> for #dropshot::ApiEndpoint {
            fn from(_: #name) -> Self {
                #item

                #dropshot::ApiEndpoint::new(
                    #name_str.to_string(),
                    #name,
                    #dropshot::Method::#method_ident,
                    #path,
                )
                #description
                #(#tags)*
            }
        }
    };

    Ok(stream)
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

fn extract_doc_from_attrs(attrs: &Vec<syn::Attribute>) -> Option<String> {
    let doc = syn::Ident::new("doc", proc_macro2::Span::call_site());

    attrs
        .iter()
        .filter_map(|attr| {
            if let Ok(meta) = attr.parse_meta() {
                if let syn::Meta::NameValue(nv) = meta {
                    if nv.path.is_ident(&doc) {
                        if let syn::Lit::Str(s) = nv.lit {
                            let comment = s.value();
                            if comment.starts_with(" ")
                                && !comment.starts_with("  ")
                            {
                                // Trim off the first character if the comment
                                // begins with a single space.
                                return Some(comment.as_str()[1..].to_string());
                            } else {
                                return Some(comment);
                            }
                        }
                    }
                }
            }
            None
        })
        .fold(None, |acc, comment| {
            Some(format!("{}{}", acc.unwrap_or_default(), comment))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_endpoint1() {
        let ret = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c"
            }
            .into(),
            quote! {
                fn handler_xyz(_rqctx: Arc<RequestContext>) {}
            }
            .into(),
        );
        let full = quote! {
            std::sync::Arc< dropshot::RequestContext>
        };
        let short = quote! {
            Arc<RequestContext>
        };
        let expected = quote! {
            const _: fn() = || {
                trait TypeEq {
                    type This: ?Sized;
                }
                impl<T: ?Sized> TypeEq for T {
                    type This = Self;
                }
                fn need_arc_requestcontext<T>()
                where
                    T: ?Sized + TypeEq<This = #full>,
                {
                }
                need_arc_requestcontext::<#short>();
            };

            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub struct handler_xyz {}

            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            const handler_xyz: handler_xyz = handler_xyz {};

            impl From<handler_xyz> for dropshot::ApiEndpoint {
                fn from(_: handler_xyz) -> Self {
                    fn handler_xyz(_rqctx: Arc<RequestContext>) {}
                    dropshot::ApiEndpoint::new(
                        "handler_xyz".to_string(),
                        handler_xyz,
                        dropshot::Method::GET,
                        "/a/b/c",
                    )
                }
            }
        };

        assert_eq!(expected.to_string(), ret.unwrap().to_string());
    }

    #[test]
    fn test_endpoint2() {
        let ret = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c"
            }
            .into(),
            quote! {
                fn handler_xyz(_rqctx: Arc<RequestContext>, q: Query<Q>) {}
            }
            .into(),
        );
        let full = quote! {
            std::sync::Arc< dropshot::RequestContext>
        };
        let short = quote! {
            Arc<RequestContext>
        };
        let query = quote! {
            Query<Q>
        };
        let expected = quote! {
            const _: fn() = || {
                trait TypeEq {
                    type This: ?Sized;
                }
                impl<T: ?Sized> TypeEq for T {
                    type This = Self;
                }
                fn need_arc_requestcontext<T>()
                where
                    T: ?Sized + TypeEq<This = #full>,
                {
                }
                need_arc_requestcontext::<#short>();
            };

            const _: fn() = || {
                fn need_extractor<T>()
                where
                    T: ?Sized + dropshot::Extractor,
                {
                }
                need_extractor::<#query>();
            };

            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub struct handler_xyz {}

            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            const handler_xyz: handler_xyz = handler_xyz {};

            impl From<handler_xyz> for dropshot::ApiEndpoint {
                fn from(_: handler_xyz) -> Self {
                    fn handler_xyz(_rqctx: Arc<RequestContext>, q: Query<Q>) {}
                    dropshot::ApiEndpoint::new(
                        "handler_xyz".to_string(),
                        handler_xyz,
                        dropshot::Method::GET,
                        "/a/b/c",
                    )
                }
            }
        };

        assert_eq!(expected.to_string(), ret.unwrap().to_string());
    }

    #[test]
    fn test_endpoint_with_tags() {
        let ret = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c",
                tags = ["stuff", "things"],
            }
            .into(),
            quote! {
                fn handler_xyz(_rqctx: Arc<RequestContext>) {}
            }
            .into(),
        );
        let full = quote! {
            std::sync::Arc< dropshot::RequestContext>
        };
        let short = quote! {
            Arc<RequestContext>
        };
        let expected = quote! {
            const _: fn() = || {
                trait TypeEq {
                    type This: ?Sized;
                }
                impl<T: ?Sized> TypeEq for T {
                    type This = Self;
                }
                fn need_arc_requestcontext<T>()
                where
                    T: ?Sized + TypeEq<This = #full>,
                {
                }
                need_arc_requestcontext::<#short>();
            };

            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub struct handler_xyz {}
            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            const handler_xyz: handler_xyz = handler_xyz {};
            impl From<handler_xyz> for dropshot::ApiEndpoint {
                fn from(_: handler_xyz) -> Self {
                    fn handler_xyz(_rqctx: Arc<RequestContext>) {}
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

        assert_eq!(expected.to_string(), ret.unwrap().to_string());
    }

    #[test]
    fn test_endpoint_with_doc() {
        let ret = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c"
            }
            .into(),
            quote! {
                /** handle "xyz" requests */
                fn handler_xyz(_rqctx: Arc<RequestContext>) {}
            }
            .into(),
        );
        let full = quote! {
            std::sync::Arc< dropshot::RequestContext>
        };
        let short = quote! {
            Arc<RequestContext>
        };
        let expected = quote! {
            const _: fn() = || {
                trait TypeEq {
                    type This: ?Sized;
                }
                impl<T: ?Sized> TypeEq for T {
                    type This = Self;
                }
                fn need_arc_requestcontext<T>()
                where
                    T: ?Sized + TypeEq<This = #full>,
                {
                }
                need_arc_requestcontext::<#short>();
            };

            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handle \"xyz\" requests"]
            pub struct handler_xyz {}
            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handle \"xyz\" requests"]
            const handler_xyz: handler_xyz = handler_xyz {};
            impl From<handler_xyz> for dropshot::ApiEndpoint {
                fn from(_: handler_xyz) -> Self {
                    #[doc = r#" handle "xyz" requests "#]
                    fn handler_xyz(_rqctx: Arc<RequestContext>) {}
                    dropshot::ApiEndpoint::new(
                        "handler_xyz".to_string(),
                        handler_xyz,
                        dropshot::Method::GET,
                        "/a/b/c",
                    )
                    .description("handle \"xyz\" requests ")
                }
            }
        };

        assert_eq!(expected.to_string(), ret.unwrap().to_string());
    }

    #[test]
    fn test_endpoint_invalid_item() {
        let ret = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c"
            }
            .into(),
            quote! {
                const POTATO = "potato";
            }
            .into(),
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
            }
            .into(),
            quote! {
                const POTATO = "potato";
            }
            .into(),
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
            }
            .into(),
            quote! {
                const POTATO = "potato";
            }
            .into(),
        );

        let msg = format!("{}", ret.err().unwrap());
        assert_eq!("extraneous member `methud`", msg);
    }
}
