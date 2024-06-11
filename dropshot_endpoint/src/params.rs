// Copyright 2023 Oxide Computer Company

//! Code to manage request and response parameters for Dropshot endpoints.

use std::fmt;

use quote::quote;
use syn::{visit::Visit, Error};

use crate::error_store::ErrorSink;

/// Perform syntactic validation for an argument or return type.
///
/// This returns the input type if it is valid.
pub(crate) fn validate_param_ty<'ast>(
    ty: &'ast syn::Type,
    kind: ParamTyKind,
    name_str: &str,
    errors: &ErrorSink<'_, Error>,
) -> Option<&'ast syn::Type> {
    // Types can be arbitrarily nested, so to keep these checks simple we use
    // the visitor pattern.

    let errors = errors.new();

    // This just needs a second 'store lifetime because the one inside ErrorSink
    // is invariant. Everything else is covariant and can share lifetimes.
    struct Visitor<'store, 'ast> {
        kind: ParamTyKind,
        name_str: &'ast str,
        errors: &'ast ErrorSink<'store, Error>,
    }

    impl<'store, 'ast> Visit<'ast> for Visitor<'store, 'ast> {
        fn visit_bound_lifetimes(&mut self, i: &'ast syn::BoundLifetimes) {
            let name_str = self.name_str;
            let kind = self.kind;
            self.errors.push(Error::new_spanned(
                i,
                format!(
                    "endpoint `{name_str}` must not have lifetime bounds \
                     in {kind}",
                ),
            ));
        }

        fn visit_lifetime(&mut self, i: &'ast syn::Lifetime) {
            let name_str = self.name_str;
            let kind = self.kind;
            if i.ident != "static" {
                self.errors.push(Error::new_spanned(
                    i,
                    format!(
                        "endpoint `{name_str}` must not have lifetime parameters \
                         in {kind}",
                    ),
                ));
            }
        }

        fn visit_ident(&mut self, i: &'ast syn::Ident) {
            if i == "Self" {
                let name_str = self.name_str;
                let kind = self.kind;
                self.errors.push(Error::new_spanned(
                    i,
                    format!(
                        "endpoint `{name_str}` must not have `Self` in {kind}",
                    ),
                ));
            }
        }

        fn visit_type_impl_trait(&mut self, i: &'ast syn::TypeImplTrait) {
            let name_str = self.name_str;
            let kind = self.kind;
            self.errors.push(Error::new_spanned(
                i,
                format!(
                    "endpoint `{name_str}` must not have impl Trait in {kind}",
                ),
            ));
        }
    }

    let mut visitor = Visitor { kind, name_str, errors: &errors };
    visitor.visit_type(ty);

    // Don't return the type if there were errors.
    (!errors.has_errors()).then(|| ty)
}

/// A representation of the RequestContext type.
#[derive(Clone, Eq, PartialEq)]
pub(crate) enum RqctxTy<'ast> {
    /// This is a function-based macro, with the payload being the full type.
    Function(&'ast syn::Type),
}

impl<'ast> RqctxTy<'ast> {
    /// Ensures that the type parameter for RequestContext is valid.
    pub(crate) fn new(
        name_str: &str,
        ty: &'ast syn::Type,
        errors: &ErrorSink<'_, Error>,
    ) -> Option<Self> {
        let param = match extract_rqctx_param(ty) {
            Ok(Some(ty)) => ty,
            Ok(None) => {
                return Some(Self::Function(ty));
            }
            Err(_) => {
                // Can't do any further validation on the type.
                errors.push(Error::new_spanned(
                    ty,
                    format!(
                        "endpoint `{name_str}` must accept a \
                        RequestContext<T> as its first argument",
                    ),
                ));
                return None;
            }
        };

        // Now validate the inner parameter.
        validate_param_ty(param, ParamTyKind::RequestContext, name_str, errors)
            .map(|_| Self::Function(ty))
    }

    pub(crate) fn as_type(&self) -> &'ast syn::Type {
        match self {
            RqctxTy::Function(ty) => ty,
        }
    }
}

impl<'ast> fmt::Debug for RqctxTy<'ast> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RqctxTy::Function(ty) => write!(f, "Function({})", quote! { #ty }),
        }
    }
}

/// Extracts and ensures that the type parameter for RequestContext is valid.
fn extract_rqctx_param<'ast>(
    ty: &'ast syn::Type,
) -> Result<Option<&'ast syn::Type>, RqctxTyError> {
    let syn::Type::Path(p) = &*ty else {
        return Err(RqctxTyError::NotTypePath);
    };

    // Inspect the last path segment.
    let Some(last_segment) = p.path.segments.last() else {
        return Err(RqctxTyError::NoPathSegments);
    };

    // It must either not have type arguments at all, or if so then exactly one
    // argument.
    let a = match &last_segment.arguments {
        syn::PathArguments::None => {
            // This is all right -- hopefully a type alias.
            return Ok(None);
        }
        syn::PathArguments::AngleBracketed(a) => a,
        syn::PathArguments::Parenthesized(_) => {
            // This isn't really possible in this position?
            return Err(RqctxTyError::ArgsNotAngleBracketed);
        }
    };

    if a.args.len() != 1 {
        return Err(RqctxTyError::IncorrectTypeArgCount(a.args.len()));
    }

    // The argument must be a type.
    let syn::GenericArgument::Type(tp) = a.args.first().unwrap() else {
        return Err(RqctxTyError::ArgNotType);
    };

    Ok(Some(tp))
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum RqctxTyError {
    NotTypePath,
    NoPathSegments,
    ArgsNotAngleBracketed,
    IncorrectTypeArgCount(usize),
    ArgNotType,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParamTyKind {
    RequestContext,
    Extractor,
    Return,
}

impl fmt::Display for ParamTyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParamTyKind::RequestContext => write!(f, "RequestContext"),
            ParamTyKind::Extractor => write!(f, "extractor"),
            ParamTyKind::Return => write!(f, "return type"),
        }
    }
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::*;

    #[test]
    fn test_extract_rqctx_ty_param() {
        let some_type = parse_quote! { SomeType };
        let unit = parse_quote! { () };
        let tuple = parse_quote! { (SomeType, OtherType) };

        // Valid types.
        let valid: &[(syn::Type, _)] = &[
            (parse_quote! { RequestContext<SomeType> }, Some(&some_type)),
            // Tuple types.
            (parse_quote! { RequestContext<()> }, Some(&unit)),
            (
                parse_quote! { ::path::to::dropshot::RequestContext<(SomeType, OtherType)> },
                Some(&tuple),
            ),
            // Type alias.
            (parse_quote! { MyRequestContext }, None),
        ];

        // We can't parse parenthesized generic arguments via parse_quote -- syn
        // only supports them via trait bounds. So we have to do this ugly thing
        // to test that case.
        let paren_generic_type = syn::Type::Path(syn::TypePath {
            qself: None,
            path: syn::Path {
                leading_colon: None,
                segments: [syn::PathSegment {
                    ident: format_ident!("RequestContext"),
                    arguments: syn::PathArguments::Parenthesized(
                        syn::ParenthesizedGenericArguments {
                            paren_token: Default::default(),
                            inputs: Default::default(),
                            output: syn::ReturnType::Default,
                        },
                    ),
                }]
                .into_iter()
                .collect(),
            },
        });

        // Invalid types.
        let invalid: &[(syn::Type, RqctxTyError)] = &[
            (parse_quote! { &'a MyRequestContext }, RqctxTyError::NotTypePath),
            (paren_generic_type, RqctxTyError::ArgsNotAngleBracketed),
            (
                parse_quote! { RequestContext<SomeType, OtherType> },
                RqctxTyError::IncorrectTypeArgCount(2),
            ),
            (parse_quote! { RequestContext<'a> }, RqctxTyError::ArgNotType),
        ];

        for (ty, expected) in valid {
            match extract_rqctx_param(ty) {
                Ok(actual) => assert_eq!(
                    *expected,
                    actual,
                    "for type {}, expected matches actual",
                    quote! { #ty },
                ),
                Err(error) => {
                    panic!(
                        "type {} should have successfully been parsed \
                         as {expected:?}, but got {error:?}",
                        quote! { #ty },
                    )
                }
            }
        }

        for (ty, expected) in invalid {
            match extract_rqctx_param(ty) {
                Ok(ret) => panic!(
                    "type {} should have failed to parse, but succeeded: \
                    {ret:?}",
                    quote! { #ty },
                ),
                Err(actual) => assert_eq!(
                    expected,
                    &actual,
                    "for invalid type {}, expected error matches actual",
                    quote! { #ty },
                ),
            };
        }
    }
}
