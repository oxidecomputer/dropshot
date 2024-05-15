// Copyright 2023 Oxide Computer Company

use quote::ToTokens;
use std::str::FromStr;

pub(crate) const APPLICATION_JSON: &str = "application/json";
pub(crate) const APPLICATION_X_WWW_FORM_URLENCODED: &str =
    "application/x-www-form-urlencoded";
pub(crate) const MULTIPART_FORM_DATA: &str = "multipart/form-data";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ValidContentType {
    ApplicationJson,
    ApplicationXWwwFormUrlencoded,
    MultipartFormData,
}

impl ValidContentType {
    pub(crate) fn as_static_str(&self) -> &'static str {
        match self {
            ValidContentType::ApplicationJson => APPLICATION_JSON,
            ValidContentType::ApplicationXWwwFormUrlencoded => {
                APPLICATION_X_WWW_FORM_URLENCODED
            }
            ValidContentType::MultipartFormData => MULTIPART_FORM_DATA,
        }
    }
}

impl FromStr for ValidContentType {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            APPLICATION_JSON => Ok(ValidContentType::ApplicationJson),
            APPLICATION_X_WWW_FORM_URLENCODED => {
                Ok(ValidContentType::ApplicationXWwwFormUrlencoded)
            }
            MULTIPART_FORM_DATA => Ok(ValidContentType::MultipartFormData),
            _ => Err("invalid content type for endpoint"),
        }
    }
}

impl ToTokens for ValidContentType {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let s = self.as_static_str();
        tokens.extend(quote::quote! { #s });
    }
}

/// Given an optional string, returns the crate name as a token stream.
pub(crate) fn get_crate(var: Option<&str>) -> proc_macro2::TokenStream {
    const DROPSHOT: &str = "dropshot";

    if let Some(s) = var {
        if let Ok(ts) = syn::parse_str(s) {
            return ts;
        }
    }
    syn::Ident::new(DROPSHOT, proc_macro2::Span::call_site()).to_token_stream()
}
