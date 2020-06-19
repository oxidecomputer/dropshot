//! This macro defines attributes associated with HTTP handlers. These
//! attributes are used both to define an HTTP API and to generate an OpenAPI
//! Spec (OAS) v3 document that describes the API.

extern crate proc_macro;

use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use quote::ToTokens;
use serde::Deserialize;
use serde_derive_internals::ast::Container;
use serde_derive_internals::{Ctxt, Derive};
use serde_tokenstream::from_tokenstream;
use serde_tokenstream::Error;
use syn::{parse_macro_input, DeriveInput, ItemFn};

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
    fn as_str(self) -> &'static str {
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

    let ast: ItemFn = syn::parse2(item)?;

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
    let description = description_text_provided.map(|s| {
        quote! {
            endpoint.description = Some(#s.to_string());
        }
    });

    let dropshot = get_crate(metadata._dropshot_crate);

    // The final TokenStream returned will have a few components that reference
    // `#name`, the name of the method to which this macro was applied...
    let stream = quote! {
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
                #ast

                #[allow(unused_mut)]
                let mut endpoint = #dropshot::ApiEndpoint::new(
                    #name_str.to_string(),
                    #name,
                    #dropshot::Method::#method_ident,
                    #path,
                );
                #description
                endpoint
            }
        }
    };

    Ok(stream.into())
}

/// Derive the implementation for dropshot::ExtractedParameter
#[proc_macro_derive(ExtractedParameter, attributes(dropshot))]
pub fn derive_parameter(
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    do_derive_parameter(&input).unwrap_or_else(to_compile_errors).into()
}

fn do_derive_parameter(
    input: &DeriveInput,
) -> Result<TokenStream, Vec<syn::Error>> {
    let ctxt = Ctxt::new();

    let cont = match Container::from_ast(&ctxt, input, Derive::Deserialize) {
        Some(cont) => cont,
        None => return Err(ctxt.check().unwrap_err()),
    };

    ctxt.check()?;

    let dropshot = get_crate(get_crate_attr(&cont));

    let fields = cont
        .data
        .all_fields()
        .filter_map(|f| {
            match &f.member {
                syn::Member::Named(ident) => {
                    let doc = extract_doc_from_attrs(&f.original.attrs)
                        .map_or_else(
                            || quote! { None },
                            |s| quote! { Some(#s.to_string()) },
                        );
                    let name = ident.to_string();
                    Some(quote! {
                        #dropshot::ApiEndpointParameter {
                            name: (_in.clone(), #name.to_string()).into(),
                            description: #doc ,
                            required: true, // TODO look for Option type
                            schema: None,
                            examples: vec![],
                        }
                    })
                }
                _ => None,
            }
        })
        .collect::<Vec<_>>();

    // Construct the appropriate where clause.
    let name = cont.ident;
    let mut generics = cont.generics.clone();

    for tp in cont.generics.type_params() {
        let ident = &tp.ident;
        let pred: syn::WherePredicate = syn::parse2(quote! {
            #ident : serde::de::DeserializeOwned
        })
        .map_err(|e| vec![e])?;
        generics.make_where_clause().predicates.push(pred);
    }

    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let stream = quote! {
        impl #impl_generics #dropshot::ExtractedParameter for #name #ty_generics
        #where_clause
        {
            fn metadata(
                _in: #dropshot::ApiEndpointParameterLocation,
            ) -> Vec<#dropshot::ApiEndpointParameter>
            {
                vec![ #(#fields,)* ]
            }
        }
    };

    Ok(stream.into())
}

fn get_crate(var: Option<String>) -> TokenStream {
    if let Some(s) = var {
        if let Ok(ts) = syn::parse_str(s.as_str()) {
            return ts;
        }
    }
    syn::Ident::new(DROPSHOT, proc_macro2::Span::call_site()).to_token_stream()
}

fn get_crate_attr(cont: &Container) -> Option<String> {
    cont.original
        .attrs
        .iter()
        .filter_map(|attr| {
            if let Ok(meta) = attr.parse_meta() {
                if let syn::Meta::List(list) = meta {
                    if list.path.is_ident(&syn::Ident::new(
                        "dropshot",
                        proc_macro2::Span::call_site(),
                    )) && list.nested.len() == 1
                    {
                        if let Some(syn::NestedMeta::Meta(
                            syn::Meta::NameValue(nv),
                        )) = list.nested.first()
                        {
                            if nv.path.is_ident(&syn::Ident::new(
                                "crate",
                                proc_macro2::Span::call_site(),
                            )) {
                                if let syn::Lit::Str(s) = &nv.lit {
                                    return Some(s.value());
                                }
                            }
                        }
                    }
                }
            }
            None
        })
        .last()
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
                                return Some(format!(
                                    "{}",
                                    &comment.as_str()[1..]
                                ));
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
            Some(format!("{}{}", acc.unwrap_or(String::new()), comment))
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
                fn handler_xyz() {}
            }
            .into(),
        );
        let expected = quote! {
            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            pub struct handler_xyz {}
            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handler_xyz"]
            const handler_xyz: handler_xyz = handler_xyz {};
            impl From<handler_xyz> for dropshot::ApiEndpoint {
                fn from(_: handler_xyz) -> Self {
                    fn handler_xyz() {}
                    #[allow(unused_mut)]
                    let mut endpoint =
                        dropshot::ApiEndpoint::new(
                            "handler_xyz".to_string(),
                            handler_xyz,
                            dropshot::Method::GET,
                            "/a/b/c",
                        );
                    endpoint
                }
            }
        };

        assert_eq!(expected.to_string(), ret.unwrap().to_string());
    }

    #[test]
    fn test_endpoint1_with_doc() {
        let ret = do_endpoint(
            quote! {
                method = GET,
                path = "/a/b/c"
            }
            .into(),
            quote! {
                /** handle "xyz" requests */
                fn handler_xyz() {}
            }
            .into(),
        );
        let expected = quote! {
            #[allow(non_camel_case_types, missing_docs)]
            #[doc = "API Endpoint: handle \"xyz\" requests"]
            pub struct handler_xyz {}
            #[allow(non_upper_case_globals, missing_docs)]
            #[doc = "API Endpoint: handle \"xyz\" requests"]
            const handler_xyz: handler_xyz = handler_xyz {};
            impl From<handler_xyz> for dropshot::ApiEndpoint {
                fn from(_: handler_xyz) -> Self {
                    #[doc = r#" handle "xyz" requests "#]
                    fn handler_xyz() {}
                    #[allow(unused_mut)]
                    let mut endpoint =
                        dropshot::ApiEndpoint::new(
                            "handler_xyz".to_string(),
                            handler_xyz,
                            dropshot::Method::GET,
                            "/a/b/c",
                        );
                    endpoint.description = Some(
                        "handle \"xyz\" requests ".to_string());
                    endpoint
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
                const POTATO = "potato";
            }
            .into(),
        );

        let msg = format!("{}", ret.err().unwrap());
        assert_eq!("expected `fn`", msg);
    }

    #[test]
    fn test_endpoint3() {
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
    fn test_endpoint4() {
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

    /// This tests the actual generated output; while this is quite strict, it
    /// is intended to require care when modifying the generated code.
    #[test]
    fn test_derive_parameter() {
        let ret = do_derive_parameter(
            &syn::parse2::<syn::DeriveInput>(quote! {
                struct Foo {
                    a: String,
                    b: String,
                }
            })
            .unwrap(),
        );

        let expected = quote! {
            impl dropshot::ExtractedParameter for Foo {
                fn metadata(
                    _in: dropshot::ApiEndpointParameterLocation,
                ) -> Vec<dropshot::ApiEndpointParameter> {
                    vec![
                        dropshot::ApiEndpointParameter {
                            name: (_in.clone(), "a".to_string()).into(),
                            description: None,
                            required: true,
                            schema: None,
                            examples: vec![],
                        },
                        dropshot::ApiEndpointParameter {
                            name: (_in.clone(), "b".to_string()).into(),
                            description: None,
                            required: true,
                            schema: None,
                            examples: vec![],
                        },
                    ]
                }
            }
        };

        assert_eq!(expected.to_string(), ret.unwrap().to_string());
    }
}
