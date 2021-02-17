// Copyright 2020 Oxide Computer Company

//! This package defines macro attributes associated with HTTP handlers. These
//! attributes are used both to define an HTTP API and to generate an OpenAPI
//! Spec (OAS) v3 document that describes the API.

/*
 * Clippy's style advice is definitely valuable, but not worth the trouble for
 * automated enforcement.
 */
#![allow(clippy::style)]

extern crate proc_macro;

use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use quote::{quote_spanned, ToTokens};
use serde::Deserialize;
use serde_tokenstream::from_tokenstream;
use serde_tokenstream::Error;
use syn::spanned::Spanned;
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

fn usage(err_msg: &str, fn_name: &str) -> String {
    format!(
        "{}\nEndpoint handlers must have the following signature:
    async fn {}(
        rqctx: std::sync::Arc<dropshot::RequestContext<MyContext>>,
        [query_params: Query<Q>,]
        [path_params: Path<P>,]
        [body_param: TypedBody<J>,]
        [body_param: UntypedBody<J>,]
    ) -> Result<HttpResponse*, HttpError>",
        err_msg, fn_name
    )
}

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
///     // Optional fields
///     tags = [ "all", "your", "OpenAPI", "tags" ],
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

    if ast.sig.asyncness.is_none() {
        return Err(Error::new_spanned(
            ast.sig.fn_token,
            "endpoint handler functions must be async",
        ));
    }

    let name = &ast.sig.ident;
    let name_str = name.to_string();
    let method_ident = format_ident!("{}", method);
    let visibility = &ast.vis;

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

    let first_arg = ast.sig.inputs.first().ok_or_else(|| {
        Error::new_spanned(
            (&ast.sig).into_token_stream(),
            usage("Endpoint requires arguments", &name_str),
        )
    })?;
    let first_arg_type = {
        match first_arg {
            syn::FnArg::Typed(syn::PatType {
                attrs: _,
                pat: _,
                colon_token: _,
                ty,
            }) => ty,
            _ => {
                return Err(Error::new(
                    first_arg.span(),
                    usage("Expected a non-receiver argument", &name_str),
                ));
            }
        }
    };

    // When the user attaches this proc macro to a function with the wrong type
    // signature, the resulting errors can be deeply inscrutable. To attempt to
    // make failures easier to understand, we inject code that asserts the types
    // of the various parameters. We do this by calling a dummy function that
    // requires a type that satisfies the trait Extractor.
    let checks = ast
        .sig
        .inputs
        .iter()
        .skip(1)
        .map(|arg| {
            let req = quote! { #dropshot::Extractor };

            match arg {
                syn::FnArg::Receiver(_) => {
                    // The compiler failure here is already comprehensible.
                    quote! {}
                }
                syn::FnArg::Typed(pat) => {
                    let span = Error::new_spanned(pat.ty.as_ref(), "").span();
                    let ty = pat.ty.as_ref().into_token_stream();

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
        })
        .collect::<Vec<_>>();

    // The final TokenStream returned will have a few components that reference
    // `#name`, the name of the method to which this macro was applied...
    let stream = quote! {
        #(#checks)*

        // ... a struct type called `#name` that has no members
        #[allow(non_camel_case_types, missing_docs)]
        #description_doc_comment
        #visibility struct #name {}
        // ... a constant of type `#name` whose identifier is also #name
        #[allow(non_upper_case_globals, missing_docs)]
        #description_doc_comment
        #visibility const #name: #name = #name {};

        // ... an impl of `From<#name>` for ApiEndpoint that allows the constant
        // `#name` to be passed into `ApiDescription::register()`
        impl From<#name> for #dropshot::ApiEndpoint<<#first_arg_type as #dropshot::RequestContextArgument>::Context> {
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

fn extract_doc_from_attrs(attrs: &[syn::Attribute]) -> Option<String> {
    let doc = syn::Ident::new("doc", proc_macro2::Span::call_site());

    attrs
        .iter()
        .filter_map(|attr| {
            if let Ok(meta) = attr.parse_meta() {
                if let syn::Meta::NameValue(nv) = meta {
                    if nv.path.is_ident(&doc) {
                        if let syn::Lit::Str(s) = nv.lit {
                            return Some(normalize_comment_string(s.value()));
                        }
                    }
                }
            }
            None
        })
        .fold(None, |acc, comment| match acc {
            None => Some(comment),
            Some(prev) if prev.ends_with('-') => {
                Some(format!("{}{}", prev, comment))
            }
            Some(prev) => Some(format!("{} {}", prev, comment)),
        })
}

fn normalize_comment_string(s: String) -> String {
    let ret = s
        .replace("-\n * ", "-")
        .replace("\n * ", " ")
        .trim_end_matches(&[' ', '\n'] as &[char])
        .to_string();
    if ret.starts_with(' ') && !ret.starts_with("  ") {
        // Trim off the first character if the comment
        // begins with a single space.
        ret.as_str()[1..].to_string()
    } else {
        ret
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_endpoint_basic() {
        let ret = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c"
            }
            .into(),
            quote! {
                pub async fn handler_xyz(_rqctx: Arc<RequestContext<()>>) {}
            }
            .into(),
        );
        let expected = quote! {
            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub struct handler_xyz {}

            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub const handler_xyz: handler_xyz = handler_xyz {};

            impl From<handler_xyz> for dropshot::ApiEndpoint<<Arc<RequestContext<()> > as dropshot::RequestContextArgument>::Context> {
                fn from(_: handler_xyz) -> Self {
                    pub async fn handler_xyz(_rqctx: Arc<RequestContext<()>>) {}
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
    fn test_endpoint_context_fully_qualified_names() {
        let ret = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c"
            }
            .into(),
            quote! {
                pub async fn handler_xyz(_rqctx: std::sync::Arc<dropshot::RequestContext<()>>) {}
            }
            .into(),
        );
        let expected = quote! {
            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub struct handler_xyz {}

            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub const handler_xyz: handler_xyz = handler_xyz {};

            impl From<handler_xyz> for dropshot::ApiEndpoint<<std::sync::Arc<dropshot::RequestContext<()> > as dropshot::RequestContextArgument>::Context> {
                fn from(_: handler_xyz) -> Self {
                    pub async fn handler_xyz(_rqctx: std::sync::Arc<dropshot::RequestContext<()>>) {}
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
    fn test_endpoint_with_query() {
        let ret = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c"
            }
            .into(),
            quote! {
                async fn handler_xyz(_rqctx: Arc<RequestContext<std::i32>>, q: Query<Q>) {}
            }
            .into(),
        );
        let query = quote! {
            Query<Q>
        };
        let expected = quote! {
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
            struct handler_xyz {}

            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            const handler_xyz: handler_xyz = handler_xyz {};

            impl From<handler_xyz> for dropshot::ApiEndpoint<<Arc<RequestContext<std::i32> > as dropshot::RequestContextArgument>::Context> {
                fn from(_: handler_xyz) -> Self {
                    async fn handler_xyz(_rqctx: Arc<RequestContext<std::i32>>, q: Query<Q>) {}
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
    fn test_endpoint_pub_crate() {
        let ret = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c"
            }
            .into(),
            quote! {
                pub(crate) async fn handler_xyz(_rqctx: Arc<RequestContext<()>>, q: Query<Q>) {}
            }
            .into(),
        );
        let query = quote! {
            Query<Q>
        };
        let expected = quote! {
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
            pub(crate) struct handler_xyz {}

            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub(crate) const handler_xyz: handler_xyz = handler_xyz {};

            impl From<handler_xyz> for dropshot::ApiEndpoint<<Arc<RequestContext<()> > as dropshot::RequestContextArgument>::Context> {
                fn from(_: handler_xyz) -> Self {
                    pub(crate) async fn handler_xyz(_rqctx: Arc<RequestContext<()>>, q: Query<Q>) {}
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
                async fn handler_xyz(_rqctx: Arc<RequestContext<()>>) {}
            }
            .into(),
        );
        let expected = quote! {
            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            struct handler_xyz {}
            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            const handler_xyz: handler_xyz = handler_xyz {};
            impl From<handler_xyz> for dropshot::ApiEndpoint<<Arc<RequestContext<()> > as dropshot::RequestContextArgument>::Context> {
                fn from(_: handler_xyz) -> Self {
                    async fn handler_xyz(_rqctx: Arc<RequestContext<()>>) {}
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
                async fn handler_xyz(_rqctx: Arc<RequestContext<()>>) {}
            }
            .into(),
        );
        let expected = quote! {
            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handle \"xyz\" requests"]
            struct handler_xyz {}
            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handle \"xyz\" requests"]
            const handler_xyz: handler_xyz = handler_xyz {};
            impl From<handler_xyz> for dropshot::ApiEndpoint<<Arc<RequestContext<()> > as dropshot::RequestContextArgument>::Context> {
                fn from(_: handler_xyz) -> Self {
                    #[doc = r#" handle "xyz" requests "#]
                    async fn handler_xyz(_rqctx: Arc<RequestContext<()>>) {}
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

    #[test]
    fn test_endpoint_not_async() {
        let ret = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c",
            }
            .into(),
            quote! {
                fn handler_xyz(_rqctx: Arc<RequestContext>) {}
            }
            .into(),
        );

        let msg = format!("{}", ret.err().unwrap());
        assert_eq!("endpoint handler functions must be async", msg);
    }

    #[test]
    fn test_endpoint_bad_context_receiver() {
        let ret = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c",
            }
            .into(),
            quote! {
                async fn handler_xyz(&self) {}
            }
            .into(),
        );

        let msg = format!("{}", ret.err().unwrap());
        assert_eq!(
            usage("Expected a non-receiver argument", "handler_xyz"),
            msg
        );
    }

    #[test]
    fn test_endpoint_no_arguments() {
        let ret = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c",
            }
            .into(),
            quote! {
                async fn handler_xyz() {}
            }
            .into(),
        );

        let msg = format!("{}", ret.err().unwrap());
        assert_eq!(usage("Endpoint requires arguments", "handler_xyz"), msg);
    }
}
