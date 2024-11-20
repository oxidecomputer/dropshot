// Copyright 2024 Oxide Computer Company

//! Code to handle metadata associated with an endpoint.

use proc_macro2::{TokenStream, TokenTree};
use quote::{format_ident, quote_spanned, ToTokens};
use serde::Deserialize;
use serde_tokenstream::ParseWrapper;
use syn::{spanned::Spanned, Error};

use crate::{
    doc::ExtractedDoc,
    error_store::ErrorSink,
    util::{get_crate, is_wildcard_path, MacroKind, ValidContentType},
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
    #[serde(default)]
    pub(crate) operation_id: Option<String>,
    pub(crate) method: MethodType,
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) unpublished: bool,
    #[serde(default)]
    pub(crate) deprecated: bool,
    // Optional expression of type `usize`.
    #[serde(default)]
    pub(crate) request_body_max_bytes: Option<ParseWrapper<syn::Expr>>,
    pub(crate) content_type: Option<String>,
    pub(crate) _dropshot_crate: Option<String>,
    pub(crate) versions: Option<ParseWrapper<VersionRange>>,
}

impl EndpointMetadata {
    /// Returns the dropshot crate value as a TokenStream.
    pub(crate) fn dropshot_crate(&self) -> TokenStream {
        get_crate(self._dropshot_crate.as_deref())
    }

    /// Validates metadata, returning a `ValidatedEndpointMetadata` if valid.
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
            operation_id,
            method,
            path,
            tags,
            unpublished,
            deprecated,
            request_body_max_bytes,
            content_type,
            _dropshot_crate,
            versions,
        } = self;

        if kind == MacroKind::Trait && _dropshot_crate.is_some() {
            errors.push(Error::new_spanned(
                attr,
                format!(
                    "endpoint `{name_str}` must not specify `_dropshot_crate`\n\
                     note: specify this as an argument to `#[api_description]` \
                     instead",
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
                operation_id,
                method,
                path,
                tags,
                unpublished,
                deprecated,
                request_body_max_bytes: request_body_max_bytes
                    .map(|x| x.into_inner()),
                content_type,
                versions: versions
                    .map(|h| h.into_inner())
                    .unwrap_or(VersionRange::All),
            })
        } else {
            unreachable!("no validation errors, but content_type is None")
        }
    }
}

/// A validated form of endpoint metadata.
pub(crate) struct ValidatedEndpointMetadata {
    operation_id: Option<String>,
    method: MethodType,
    path: String,
    tags: Vec<String>,
    unpublished: bool,
    deprecated: bool,
    request_body_max_bytes: Option<syn::Expr>,
    content_type: ValidContentType,
    versions: VersionRange,
}

fn semver_parts(x: &semver::Version) -> (u64, u64, u64) {
    // This was validated during validation.
    assert_eq!(x.pre, semver::Prerelease::EMPTY);
    assert_eq!(x.build, semver::BuildMetadata::EMPTY);
    (x.major, x.minor, x.patch)
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
        let operation_id =
            self.operation_id.as_deref().unwrap_or(endpoint_name);
        let method_ident = format_ident!("{}", self.method.as_str());

        // Apply a span to all generated code to avoid call-site attribution.
        let span = match kind {
            ApiEndpointKind::Regular(endpoint_fn) => endpoint_fn.span(),
            ApiEndpointKind::Stub { attr, .. } => {
                // We need to point at the closest possible span to the actual
                // error, but we can't point at something nice like the
                // function name. That's because if we do, rust-analyzer will
                // produce a lot of irrelevant results when ctrl-clicking on
                // the function name.
                //
                // So we point at the `#`, which seems out-of-the-way enough
                // for successful generation while being close by for errors.
                // Seems pretty unobjectionable.
                attr.pound_token.span()
            }
        };

        // Set the span for all of the bits and pieces. Most of these fields are
        // unlikely to produce compile errors or warnings, but some of them
        // (like request_body_max_bytes) can do so.
        let summary = doc.summary.as_ref().map(|summary| {
            quote_spanned! {span=> .summary(#summary) }
        });
        let description = doc.description.as_ref().map(|description| {
            quote_spanned! {span=> .description(#description) }
        });

        let tags = self
            .tags
            .iter()
            .map(|tag| {
                quote_spanned! {span=> .tag(#tag) }
            })
            .collect::<Vec<_>>();

        let visible = self.unpublished.then(|| {
            quote_spanned! {span=> .visible(false) }
        });

        let deprecated = self.deprecated.then(|| {
            quote_spanned! {span=> .deprecated(true) }
        });

        let request_body_max_bytes =
            self.request_body_max_bytes.as_ref().map(|max_bytes| {
                quote_spanned! {span=> .request_body_max_bytes(#max_bytes) }
            });

        let versions = match &self.versions {
            VersionRange::All => {
                quote_spanned! {span=> #dropshot::ApiEndpointVersions::All }
            }
            VersionRange::From(x) => {
                let (major, minor, patch) = semver_parts(&x);
                quote_spanned! {span=>
                    #dropshot::ApiEndpointVersions::From(
                        semver::Version::new(#major, #minor, #patch)
                    )
                }
            }
            VersionRange::Until(y) => {
                let (major, minor, patch) = semver_parts(&y);
                quote_spanned! {span=>
                    #dropshot::ApiEndpointVersions::Until(
                        semver::Version::new(#major, #minor, #patch)
                    )
                }
            }
            VersionRange::FromUntil(x, y) => {
                let (xmajor, xminor, xpatch) = semver_parts(&x);
                let (ymajor, yminor, ypatch) = semver_parts(&y);
                quote_spanned! {span=>
                    #dropshot::ApiEndpointVersions::from_until(
                        semver::Version::new(#xmajor, #xminor, #xpatch),
                        semver::Version::new(#ymajor, #yminor, #ypatch),
                    ).unwrap()
                }
            }
        };

        let fn_call = match kind {
            ApiEndpointKind::Regular(endpoint_fn) => {
                quote_spanned! {span=>
                    #dropshot::ApiEndpoint::new(
                        #operation_id.to_string(),
                        #endpoint_fn,
                        #dropshot::Method::#method_ident,
                        #content_type,
                        #path,
                        #versions,
                    )
                }
            }
            ApiEndpointKind::Stub { attr: _, extractor_types, ret_ty } => {
                quote_spanned! {span=>
                    #dropshot::ApiEndpoint::new_for_types::<(#(#extractor_types,)*), #ret_ty>(
                        #operation_id.to_string(),
                        #dropshot::Method::#method_ident,
                        #content_type,
                        #path,
                        #versions,
                    )
                }
            }
        };

        quote_spanned! {span=>
            #fn_call
            #summary
            #description
            #(#tags)*
            #visible
            #deprecated
            #request_body_max_bytes
        }
    }
}

#[derive(Debug)]
pub(crate) enum VersionRange {
    All,
    From(semver::Version),
    Until(semver::Version),
    FromUntil(semver::Version, semver::Version),
}

fn parse_semver(v: &syn::LitStr) -> syn::Result<semver::Version> {
    v.value()
        .parse::<semver::Version>()
        .map_err(|e| {
            syn::Error::new_spanned(v, format!("expected semver: {}", e))
        })
        .and_then(|s| {
            if s.pre == semver::Prerelease::EMPTY {
                Ok(s)
            } else {
                Err(syn::Error::new_spanned(
                    v,
                    String::from(
                        "semver pre-release string is not supported here",
                    ),
                ))
            }
        })
        .and_then(|s| {
            if s.build == semver::BuildMetadata::EMPTY {
                Ok(s)
            } else {
                Err(syn::Error::new_spanned(
                    v,
                    String::from("semver build metadata is not supported here"),
                ))
            }
        })
}

impl syn::parse::Parse for VersionRange {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let lookahead = input.lookahead1();
        if lookahead.peek(syn::Token![..]) {
            let _ = input.parse::<syn::Token![..]>()?;
            if input.is_empty() {
                Ok(VersionRange::All)
            } else {
                let latest = input.parse::<syn::LitStr>()?;
                let latest_semver = parse_semver(&latest)?;
                Ok(VersionRange::Until(latest_semver))
            }
        } else {
            let earliest = input.parse::<syn::LitStr>()?;
            let earliest_semver = parse_semver(&earliest)?;
            let _ = input.parse::<syn::Token![..]>()?;
            let lookahead = input.lookahead1();
            if lookahead.peek(syn::LitStr) {
                let latest = input.parse::<syn::LitStr>()?;
                let latest_semver = parse_semver(&latest)?;
                if latest_semver < earliest_semver {
                    let span: TokenStream = [
                        TokenTree::from(earliest.token()),
                        TokenTree::from(latest.token()),
                    ]
                    .into_iter()
                    .collect();
                    return Err(syn::Error::new_spanned(
                        span,
                        format!(
                            "\"from\" version ({}) must be earlier than \
                            \"until\" version ({})",
                            earliest_semver, latest_semver,
                        ),
                    ));
                }
                Ok(VersionRange::FromUntil(earliest_semver, latest_semver))
            } else {
                Ok(VersionRange::From(earliest_semver))
            }
        }
    }
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub(crate) enum ChannelProtocol {
    WEBSOCKETS,
}

#[derive(Deserialize, Debug)]
pub(crate) struct ChannelMetadata {
    pub(crate) protocol: ChannelProtocol,
    #[serde(default)]
    pub(crate) operation_id: Option<String>,
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) unpublished: bool,
    #[serde(default)]
    pub(crate) deprecated: bool,
    pub(crate) _dropshot_crate: Option<String>,
    pub(crate) versions: Option<ParseWrapper<VersionRange>>,
}

impl ChannelMetadata {
    /// Returns the dropshot crate value as a TokenStream.
    pub(crate) fn dropshot_crate(&self) -> TokenStream {
        get_crate(self._dropshot_crate.as_deref())
    }

    /// Validates metadata, returning a `ValidatedChannelMetadata` is valid.
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
    ) -> Option<ValidatedChannelMetadata> {
        let errors = errors.new();

        let ChannelMetadata {
            protocol: ChannelProtocol::WEBSOCKETS,
            operation_id,
            path,
            tags,
            unpublished,
            deprecated,
            _dropshot_crate,
            versions,
        } = self;

        if kind == MacroKind::Trait && _dropshot_crate.is_some() {
            errors.push(Error::new_spanned(
                attr,
                format!(
                    "channel `{name_str}` must not specify `_dropshot_crate`\n\
                     note: specify this as an argument to `#[api_description]` \
                     instead",
                ),
            ));
        }

        // Wildcard paths are not allowed.
        if is_wildcard_path(&path) {
            errors.push(Error::new_spanned(
                attr,
                format!(
                    "channel `{name_str}` has a wildcard path, which is not allowed",
                )
            ));
        }

        if errors.has_errors() {
            None
        } else {
            // Validating channel metadata also validates the corresponding
            // endpoint metadata.
            let inner = ValidatedEndpointMetadata {
                operation_id,
                method: MethodType::GET,
                path,
                tags,
                unpublished,
                deprecated,
                content_type: ValidContentType::ApplicationJson,
                versions: versions
                    .map(|h| h.into_inner())
                    .unwrap_or(VersionRange::All),
                // Channels are arbitrary-length and don't have a limit on
                // request body size.
                request_body_max_bytes: None,
            };

            Some(ValidatedChannelMetadata { inner })
        }
    }
}

pub(crate) struct ValidatedChannelMetadata {
    // Currently just a wrapper around endpoint metadata, but provided as a
    // separate type to be less surprising, and in case more custom
    // functionality is desired.
    inner: ValidatedEndpointMetadata,
}

impl ValidatedChannelMetadata {
    pub(crate) fn to_api_endpoint_fn(
        &self,
        dropshot: &TokenStream,
        endpoint_name: &str,
        kind: &ApiEndpointKind<'_>,
        doc: &ExtractedDoc,
    ) -> TokenStream {
        // Just forward to the inner endpoint -- any differences are captured in
        // `kind`.
        self.inner.to_api_endpoint_fn(dropshot, endpoint_name, kind, doc)
    }
}

/// The kind of API endpoint call to generate.
#[derive(Clone)]
pub(crate) enum ApiEndpointKind<'ast> {
    /// A regular API endpoint. The argument is the function identifier or path.
    Regular(&'ast dyn ToTokens),

    /// A stub API endpoint. This is used for #[api_description].
    Stub {
        /// The attribute, used for span information.
        attr: &'ast syn::Attribute,

        /// The extractor types in use.
        extractor_types: Vec<&'ast syn::Type>,

        /// The return type.
        ret_ty: &'ast syn::Type,
    },
}
