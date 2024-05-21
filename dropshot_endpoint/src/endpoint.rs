// Copyright 2023 Oxide Computer Company

//! Support for HTTP `#[endpoint]` macros.

use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use quote::quote_spanned;
use quote::ToTokens;
use serde::Deserialize;
use serde_tokenstream::from_tokenstream;
use serde_tokenstream::Error;
use syn::spanned::Spanned;

use crate::doc::ExtractedDoc;
use crate::error_store::ErrorSink;
use crate::error_store::ErrorStore;
use crate::syn_parsing::ItemFnForSignature;
use crate::util::get_crate;
use crate::util::ValidContentType;

/// Endpoint usage message, produced if there were parameter errors.
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
    let mut error_store = ErrorStore::new();
    let errors = error_store.sink();

    // Parse attributes. (Do this before parsing the function since that's the
    // order they're in, in source code.)
    let metadata = match from_tokenstream(&attr) {
        Ok(metadata) => Some(metadata),
        Err(e) => {
            // If there is an error while parsing the metadata, report it, but
            // continue to generate the original function.
            errors.push(e);
            None
        }
    };

    // Attempt to parse the function.
    let item_fn = match syn::parse2::<ItemFnForSignature>(item.clone()) {
        Ok(item_fn) => Some(item_fn),
        Err(e) => {
            errors.push(e);
            None
        }
    };

    let output = match (metadata, item_fn.as_ref()) {
        (Some(metadata), Some(item_fn)) => {
            // The happy path.
            do_endpoint_inner(metadata, attr, item, &item_fn, errors)
        }
        (None, Some(_)) => {
            // In this case, continue to generate the original function (but not
            // the attribute proc macro).
            EndpointOutput {
                output: quote! { #item },
                // We don't validate parameters, so we don't know if there are
                // errors in them.
                has_param_errors: false,
            }
        }
        (_, None) => {
            // Can't do anything here, just return errors.
            EndpointOutput { output: quote! {}, has_param_errors: false }
        }
    };

    let mut errors = error_store.into_inner();

    // If there are any errors, we also want to provide a usage message as an error.
    if output.has_param_errors {
        let item_fn = item_fn
            .as_ref()
            .expect("has_param_errors is true => item_fn is Some");
        errors.insert(0, Error::new_spanned(&item_fn.sig, USAGE));
    }

    Ok((output.output, errors))
}

/// The result of calling `do_endpoint_inner`.
pub(crate) struct EndpointOutput {
    /// The actual output.
    pub(crate) output: TokenStream,

    /// Whether there were any parameter-related errors.
    ///
    /// If there were, then we provide a usage message.
    pub(crate) has_param_errors: bool,
}

// The return value is the ItemFnForSignature and the TokenStream.
pub(crate) fn do_endpoint_inner(
    metadata: EndpointMetadata,
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
    item_fn: &ItemFnForSignature,
    errors: ErrorSink<'_, Error>,
) -> EndpointOutput {
    let dropshot = metadata.dropshot_crate();

    // Perform validations first.
    let metadata = metadata.validate(&attr, &errors);
    let params = EndpointParams::new(&item_fn.sig, &errors);

    let name = &item_fn.sig.ident;
    let name_str = name.to_string();
    let visibility = &item_fn.vis;

    let doc = ExtractedDoc::from_attrs(&item_fn.attrs);
    let comment_text = doc.comment_text(&name_str);

    let description_doc_comment = quote! {
        #[doc = #comment_text]
    };

    // If the metadata is valid, output the corresponding ApiEndpoint.
    let construct = if let Some(metadata) = metadata {
        metadata.to_api_endpoint_fn(&dropshot, &name_str, name, &doc)
    } else {
        quote! {
            unreachable!()
        }
    };

    // If the params are valid, output the corresponding type checks and impl
    // statement.
    let (has_param_errors, type_checks, from_impl) =
        if let Some(params) = &params {
            let type_checks = params.to_type_checks(&dropshot);
            let impl_checks = params.to_impl_checks(name);

            let rqctx_context = params.rqctx_context(&dropshot);

            let from_impl = quote! {
                impl From<#name>
                    for #dropshot::ApiEndpoint< #rqctx_context >
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

            (false, type_checks, from_impl)
        } else {
            (true, quote! {}, quote! {})
        };

    // For reasons that are not well understood unused constants that use the
    // (default) call_site() Span do not trigger the dead_code lint. Because
    // defining but not using an endpoint is likely a programming error, we
    // want to be sure to have the compiler flag this. We force this by using
    // the span from the name of the function to which this macro was applied.
    let span = item_fn.sig.ident.span();
    let const_struct = quote_spanned! {span=>
        #visibility const #name: #name = #name {};
    };

    // The final TokenStream returned will have a few components that reference
    // `#name`, the name of the function to which this macro was applied...
    let stream = quote! {
        // ... type validation for parameter and return types
        #type_checks

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
        #from_impl
    };

    EndpointOutput { output: stream, has_param_errors }
}

/// Request and return types for an endpoint.
struct EndpointParams<'ast> {
    rqctx_ty: &'ast syn::Type,
    shared_extractors: Vec<&'ast syn::Type>,
    // This is the last request argument -- it could also be a shared extractor,
    // because shared extractors are also exclusive.
    exclusive_extractor: Option<&'ast syn::Type>,
    ret_ty: &'ast syn::Type,
}

impl<'ast> EndpointParams<'ast> {
    /// Creates a new EndpointParams from an ItemFnForSignature.
    ///
    /// Validates that the AST looks reasonable and that all the types make
    /// sense, and return None if it does not.
    fn new(
        sig: &'ast syn::Signature,
        errors: &ErrorSink<'_, Error>,
    ) -> Option<Self> {
        let errors = errors.new();

        // Perform AST validations.
        if sig.constness.is_some() {
            errors.push(Error::new_spanned(
                &sig.constness,
                "endpoint handlers may not be const functions",
            ));
        }

        if sig.asyncness.is_none() {
            errors.push(Error::new_spanned(
                &sig.fn_token,
                "endpoint handler functions must be async",
            ));
        }

        if sig.unsafety.is_some() {
            errors.push(Error::new_spanned(
                &sig.unsafety,
                "endpoint handlers may not be unsafe",
            ));
        }

        if sig.abi.is_some() {
            errors.push(Error::new_spanned(
                &sig.abi,
                "endpoint handler may not use an alternate ABI",
            ));
        }

        if !sig.generics.params.is_empty() {
            errors.push(Error::new_spanned(
                &sig.generics,
                "generics are not permitted for endpoint handlers",
            ));
        }

        if sig.variadic.is_some() {
            errors
                .push(Error::new_spanned(&sig.variadic, "no language C here"));
        }

        let mut inputs = sig.inputs.iter();

        let rqctx_ty = match inputs.next() {
            Some(syn::FnArg::Typed(syn::PatType {
                attrs: _,
                pat: _,
                colon_token: _,
                ty,
            })) => Some(&**ty),
            Some(first_arg @ syn::FnArg::Receiver(_)) => {
                errors.push(Error::new_spanned(
                    first_arg,
                    "Expected a non-receiver argument",
                ));
                None
            }
            None => {
                errors.push(Error::new(
                    sig.paren_token.span.join(),
                    "Endpoint requires arguments",
                ));
                None
            }
        };

        // Subsequent parameters other than the last one must impl
        // SharedExtractor.
        let mut shared_extractors = Vec::new();
        while let Some(syn::FnArg::Typed(pat)) = inputs.next() {
            shared_extractors.push(&*pat.ty);
        }

        // Pop the last one off the iterator -- it must impl ExclusiveExtractor.
        // (A SharedExtractor can impl ExclusiveExtractor too.)
        let exclusive_extractor = shared_extractors.pop();

        let ret_ty = match &sig.output {
            syn::ReturnType::Default => {
                errors.push(Error::new_spanned(
                    sig,
                    "Endpoint must return a Result",
                ));
                None
            }
            syn::ReturnType::Type(_, ty) => Some(&**ty),
        };

        // errors.has_errors() must be checked first, because it's possible for
        // rqctx_ty and ret_ty to both be Some, but one of the extractors to
        // have errored out.
        if errors.has_errors() {
            None
        } else if let (Some(rqctx_ty), Some(ret_ty)) = (rqctx_ty, ret_ty) {
            Some(Self {
                rqctx_ty,
                shared_extractors,
                exclusive_extractor,
                ret_ty,
            })
        } else {
            unreachable!("no param errors, but rqctx_ty or ret_ty is None");
        }
    }

    /// Returns a token stream that obtains the rqctx context type.
    fn rqctx_context(&self, dropshot: &TokenStream) -> TokenStream {
        let rqctx_ty = self.rqctx_ty;
        quote_spanned! { rqctx_ty.span()=>
            <#rqctx_ty as #dropshot::RequestContextArgument>::Context
        }
    }

    /// Returns a list of generated argument names.
    fn arg_names(&self) -> impl Iterator<Item = syn::Ident> + '_ {
        // The total number of arguments is 1 (rqctx) + the number of shared
        // extractors + 0 or 1 exclusive extractors.
        let arg_count = 1
            + self.shared_extractors.len()
            + self.exclusive_extractor.map_or(0, |_| 1);

        (0..arg_count).map(|i| format_ident!("arg{}", i))
    }

    /// Returns a list of all argument types, including the request context.
    fn arg_types(&self) -> impl Iterator<Item = &syn::Type> + '_ {
        std::iter::once(self.rqctx_ty)
            .chain(self.shared_extractors.iter().copied())
            .chain(self.exclusive_extractor)
    }

    /// Returns semantic type checks for the endpoint.
    ///
    /// When the user attaches this proc macro to a function with the wrong type
    /// signature, the resulting errors can be deeply inscrutable. To attempt to
    /// make failures easier to understand, we inject code that asserts the
    /// types of the various parameters. We do this by calling dummy functions
    /// that require a type that satisfies SharedExtractor or
    /// ExclusiveExtractor.
    fn to_type_checks(&self, dropshot: &TokenStream) -> TokenStream {
        let rqctx_context = self.rqctx_context(dropshot);
        let rqctx_check = quote_spanned! { self.rqctx_ty.span()=>
            const _: fn() = || {
                struct NeedRequestContext(#rqctx_context);
            };
        };

        let shared_extractor_checks = self.shared_extractors.iter().map(|ty| {
            quote_spanned! { ty.span()=>
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

        let exclusive_extractor_check = self.exclusive_extractor.map(|ty| {
            quote_spanned! { ty.span()=>
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

        let ret_ty = self.ret_ty;
        let ret_check = quote_spanned! { ret_ty.span()=>
            const _: fn() = || {
                trait ResultTrait {
                    type T;
                    type E;
                }
                impl<TT, EE> ResultTrait for Result<TT, EE>
                where
                    TT: #dropshot::HttpResponse,
                {
                    type T = TT;
                    type E = EE;
                }
                struct NeedHttpResponse(
                    <#ret_ty as ResultTrait>::T,
                );
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
        };

        quote! {
            #rqctx_check

            #(#shared_extractor_checks)*

            #exclusive_extractor_check

            #ret_check
        }
    }

    /// Constructs implementation checks for the endpoint.
    ///
    /// These checks are placed in the same scope as the definition of the
    /// endpoint.
    fn to_impl_checks(&self, name: &syn::Ident) -> TokenStream {
        // We want to construct a function that will call the user's endpoint,
        // so we can check the future it returns for bounds that otherwise
        // produce inscrutable error messages (like returning a non-`Send`
        // future). We produce a wrapper function that takes all the same
        // argument types, which requires building up a list of argument names:
        // we can't use the original definition's argument names since they
        // could have multiple args named `_`, so we use "arg0", "arg1", etc.
        let arg_names: Vec<_> = self.arg_names().collect();
        let arg_types = self.arg_types();

        quote! {
            const _: fn() = || {
                fn future_endpoint_must_be_send<T: ::std::marker::Send>(_t: T) {}
                fn check_future_bounds(#( #arg_names: #arg_types ),*) {
                    future_endpoint_must_be_send(#name(#(#arg_names),*));
                }
            };
        }
    }
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
        attr: &dyn ToTokens,
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

        if path.contains(":.*}") && !self.unpublished {
            errors.push(Error::new_spanned(
                attr,
                "paths that contain a wildcard match must include 'unpublished = \
                 true'",
            ));
        }

        // The content type must be one of the allowed values.
        let content_type = match content_type {
            Some(content_type) => match content_type.parse() {
                Ok(content_type) => Some(content_type),
                Err(_) => {
                    errors.push(Error::new_spanned(
                        attr,
                        "invalid content type for endpoint",
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
    fn to_api_endpoint_fn(
        &self,
        dropshot: &TokenStream,
        endpoint_name: &str,
        endpoint_fn: &dyn ToTokens,
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

        quote! {
            #dropshot::ApiEndpoint::new(
                #endpoint_name.to_string(),
                #endpoint_fn,
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
    }
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

        assert!(errors.is_empty());
        assert_contents(
            "tests/output/endpoint_content_type.rs",
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
}
