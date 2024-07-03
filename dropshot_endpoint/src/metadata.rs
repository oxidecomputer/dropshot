// Copyright 2024 Oxide Computer Company

//! Code to handle metadata associated with an endpoint.

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned, ToTokens};
use serde::Deserialize;
use syn::{spanned::Spanned, Error};

use crate::{
    doc::ExtractedDoc,
    error_store::ErrorSink,
    util::{get_crate, MacroKind, ValidContentType},
};

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
    pub(crate) fn as_str(&self) -> &'static str {
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

impl EndpointMetadata {
    /// Returns the dropshot crate value as a TokenStream.
    pub(crate) fn dropshot_crate(&self) -> TokenStream {
        get_crate(self._dropshot_crate.as_deref())
    }

    /// Validates metadata, returning an `EndpointMetadata` if valid.
    ///
    /// Note: the only reason we pass in attr here is to provide a span for
    /// error reporting. As of Rust 1.76, just passing in `attr.span()` produces
    /// incorrect span info in error messages.
    pub(crate) fn validate(
        self,
        name_str: &str,
        attr: &dyn ToTokens,
        kind: MacroKind,
        errors: &ErrorSink<'_, Error>,
    ) -> Option<ValidatedEndpointMetadata> {
        let errors = errors.new();

        let EndpointMetadata {
            method,
            path,
            tags,
            unpublished,
            deprecated,
            content_type,
            _dropshot_crate,
        } = self;

        if kind == MacroKind::Trait && _dropshot_crate.is_some() {
            errors.push(Error::new_spanned(
                attr,
                format!(
                    "endpoint `{name_str}` must not specify `_dropshot_crate`\n\
                     note: specify this as an argument to `#[server]` instead",
                ),
            ));
        }

        if path.contains(":.*}") && !self.unpublished {
            errors.push(Error::new_spanned(
                attr,
                format!(
                    "endpoint `{name_str}` has paths that contain \
                     a wildcard match, but is not marked 'unpublished = true'",
                ),
            ));
        }

        // The content type must be one of the allowed values.
        let content_type = match content_type {
            Some(content_type) => match content_type.parse() {
                Ok(content_type) => Some(content_type),
                Err(_) => {
                    errors.push(Error::new_spanned(
                        attr,
                        format!(
                            "endpoint `{name_str}` has an invalid \
                            content type\n\
                            note: supported content types are: {}",
                            ValidContentType::to_supported_string()
                        ),
                    ));
                    None
                }
            },
            None => Some(ValidContentType::ApplicationJson),
        };

        // errors.has_errors() must be checked first, because it's possible for
        // content_type to be Some, but other errors to have occurred.
        if errors.has_errors() {
            None
        } else if let Some(content_type) = content_type {
            Some(ValidatedEndpointMetadata {
                method,
                path,
                tags,
                unpublished,
                deprecated,
                content_type,
            })
        } else {
            unreachable!("no validation errors, but content_type is None")
        }
    }
}

/// A validated form of endpoint metadata.
pub(crate) struct ValidatedEndpointMetadata {
    method: MethodType,
    path: String,
    tags: Vec<String>,
    unpublished: bool,
    deprecated: bool,
    content_type: ValidContentType,
}

impl ValidatedEndpointMetadata {
    pub(crate) fn to_api_endpoint_fn(
        &self,
        dropshot: &TokenStream,
        endpoint_name: &str,
        kind: &ApiEndpointKind<'_>,
        doc: &ExtractedDoc,
    ) -> TokenStream {
        let path = &self.path;
        let content_type = self.content_type;
        let method_ident = format_ident!("{}", self.method.as_str());

        let summary = doc.summary.as_ref().map(|summary| {
            quote! { .summary(#summary) }
        });
        let description = doc.description.as_ref().map(|description| {
            quote! { .description(#description) }
        });

        let tags = self
            .tags
            .iter()
            .map(|tag| {
                quote! { .tag(#tag) }
            })
            .collect::<Vec<_>>();

        let visible = self.unpublished.then(|| {
            quote! { .visible(false) }
        });

        let deprecated = self.deprecated.then(|| {
            quote! { .deprecated(true) }
        });

        let fn_call = match kind {
            ApiEndpointKind::Regular(endpoint_fn) => {
                quote_spanned! {endpoint_fn.span()=>
                    #dropshot::ApiEndpoint::new(
                        #endpoint_name.to_string(),
                        #endpoint_fn,
                        #dropshot::Method::#method_ident,
                        #content_type,
                        #path,
                    )
                }
            }
            ApiEndpointKind::Stub { attr, extractor_types, ret_ty } => {
                // We need to point at the closest possible span to the actual
                // error, but we can't point at something nice like the
                // function name. That's because if we do, rust-analyzer will
                // produce a lot of irrelevant results when ctrl-clicking on
                // the function name.
                //
                // So we point at the `#`, which seems out-of-the-way enough
                // for successful generation while being close by for errors.
                // Seems pretty unobjectionable.
                quote_spanned! {attr.pound_token.span()=>
                    #dropshot::ApiEndpoint::new_for_types::<(#(#extractor_types,)*), #ret_ty>(
                        #endpoint_name.to_string(),
                        #dropshot::Method::#method_ident,
                        #content_type,
                        #path,
                    )
                }
            }
        };

        quote! {
            #fn_call
            #summary
            #description
            #(#tags)*
            #visible
            #deprecated
        }
    }
}

/// The kind of API endpoint call to generate.
#[derive(Clone)]
pub(crate) enum ApiEndpointKind<'ast> {
    /// A regular API endpoint. The argument is the function identifier or path.
    Regular(&'ast dyn ToTokens),
    /// A stub API endpoint. This is used for #[server].
    Stub {
        /// The attribute, used for span information.
        attr: &'ast syn::Attribute,

        /// The extractor types in use.
        extractor_types: Vec<&'ast syn::Type>,

        /// The return type.
        ret_ty: &'ast syn::Type,
    },
}
