// Copyright 2024 Oxide Computer Company

//! Code to manage request and response parameters for Dropshot endpoints.

use std::{fmt, iter::Peekable};

use proc_macro2::{extra::DelimSpan, TokenStream};
use quote::{quote_spanned, ToTokens};
use syn::{parse_quote, spanned::Spanned, visit::Visit, Error};

use crate::error_store::ErrorSink;

/// Validate general properties of a function signature, not including the
/// parameters.
pub(crate) fn validate_fn_ast(
    sig: &syn::Signature,
    name_str: &str,
    errors: &ErrorSink<'_, Error>,
) {
    if sig.constness.is_some() {
        errors.push(Error::new_spanned(
            &sig.constness,
            format!("endpoint `{name_str}` must not be a const fn"),
        ));
    }

    if sig.asyncness.is_none() {
        errors.push(Error::new_spanned(
            &sig.fn_token,
            format!("endpoint `{name_str}` must be async"),
        ));
    }

    if sig.unsafety.is_some() {
        errors.push(Error::new_spanned(
            &sig.unsafety,
            format!("endpoint `{name_str}` must not be unsafe"),
        ));
    }

    if sig.abi.is_some() {
        errors.push(Error::new_spanned(
            &sig.abi,
            format!("endpoint `{name_str}` must not use an alternate ABI"),
        ));
    }

    if !sig.generics.params.is_empty() {
        errors.push(Error::new_spanned(
            &sig.generics,
            format!("endpoint `{name_str}` must not have generics"),
        ));
    }

    if let Some(where_clause) = &sig.generics.where_clause {
        // Empty where clauses are no-ops and therefore permitted.
        if !where_clause.predicates.is_empty() {
            errors.push(Error::new_spanned(
                where_clause,
                format!("endpoint `{name_str}` must not have a where clause"),
            ));
        }
    }

    if sig.variadic.is_some() {
        errors.push(Error::new_spanned(
            &sig.variadic,
            format!("endpoint `{name_str}` must not have a variadic argument",),
        ));
    }
}

/// Processor and validator for parameters in a function signature.
///
/// The caller is responsible for calling functions in the right order.
pub(crate) struct ParamValidator<'ast> {
    sig: &'ast syn::Signature,
    inputs: Peekable<syn::punctuated::Iter<'ast, syn::FnArg>>,
    name_str: String,
}

impl<'ast> ParamValidator<'ast> {
    pub(crate) fn new(sig: &'ast syn::Signature, name_str: &str) -> Self {
        Self {
            sig,
            inputs: sig.inputs.iter().peekable(),
            name_str: name_str.to_string(),
        }
    }

    pub(crate) fn maybe_discard_self_arg(
        &mut self,
        errors: &ErrorSink<'_, Error>,
    ) {
        // If there's a self argument, the second argument is often a
        // `RequestContext`, so consume the first argument.
        if let Some(syn::FnArg::Receiver(_)) = self.inputs.peek() {
            // Consume this argument.
            let self_arg = self.inputs.next();
            errors.push(Error::new_spanned(
                self_arg,
                format!(
                    "endpoint `{}` must not have a `self` argument",
                    self.name_str,
                ),
            ));
        }
    }

    pub(crate) fn next_rqctx_arg(
        &mut self,
        rqctx_kind: RqctxKind<'_>,
        paren_span: &DelimSpan,
        errors: &ErrorSink<'_, Error>,
    ) -> Option<RqctxTy<'ast>> {
        match self.inputs.next() {
            Some(syn::FnArg::Typed(syn::PatType {
                attrs: _,
                pat: _,
                colon_token: _,
                ty,
            })) => RqctxTy::new(&self.name_str, rqctx_kind, ty, &errors),
            _ => {
                errors.push(Error::new(
                    paren_span.join(),
                    format!(
                        "endpoint `{}` must have at least one \
                         RequestContext argument",
                        self.name_str,
                    ),
                ));
                None
            }
        }
    }

    pub(crate) fn rest_extractor_args(
        &mut self,
        errors: &ErrorSink<'_, Error>,
    ) -> Vec<&'ast syn::Type> {
        let mut extractors = Vec::with_capacity(self.inputs.len());
        while let Some(syn::FnArg::Typed(pat)) = self.inputs.next() {
            if let Some(ty) = validate_param_ty(
                &pat.ty,
                ParamTyKind::Extractor,
                &self.name_str,
                errors,
            ) {
                extractors.push(ty);
            }
        }

        extractors
    }

    pub(crate) fn return_type(
        &self,
        errors: &ErrorSink<'_, Error>,
    ) -> Option<&'ast syn::Type> {
        match &self.sig.output {
            syn::ReturnType::Default => {
                errors.push(Error::new_spanned(
                    self.sig,
                    format!(
                        "endpoint `{}` must return a Result",
                        self.name_str,
                    ),
                ));
                None
            }
            syn::ReturnType::Type(_, ty) => validate_param_ty(
                ty,
                ParamTyKind::Return,
                &self.name_str,
                errors,
            ),
        }
    }
}

/// Perform syntactic validation for an argument or return type.
///
/// This returns the input type if it is valid.
fn validate_param_ty<'ast>(
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

    /// This is a trait-based macro.
    Trait {
        /// The original type.
        orig: &'ast syn::Type,

        /// A transformed type, with the type parameter replaced with the unit
        /// type.
        transformed_unit: syn::Type,
    },
}

impl<'ast> RqctxTy<'ast> {
    /// Ensures that the type parameter for RequestContext is valid.
    pub(crate) fn new(
        name_str: &str,
        rqctx_kind: RqctxKind<'_>,
        ty: &'ast syn::Type,
        errors: &ErrorSink<'_, Error>,
    ) -> Option<Self> {
        match rqctx_kind {
            // Functions are straightforward -- extract the parameter inside,
            // and if it's present validate it.
            RqctxKind::Function => {
                let param = match extract_rqctx_param(ty) {
                    Ok(Some(ty)) => ty,
                    Ok(None) => {
                        // This is okay -- hopefully a type alias.
                        return Some(Self::Function(ty));
                    }
                    Err(_) => {
                        // Can't do any further validation on the type.
                        errors.push(Error::new_spanned(
                            ty,
                            rqctx_kind.to_error_message(name_str),
                        ));
                        return None;
                    }
                };

                // For functions, we can use standard parameter validation.
                return validate_param_ty(
                    param,
                    ParamTyKind::RequestContext,
                    name_str,
                    errors,
                )
                .map(|_| Self::Function(ty));
            }

            // Traits are a bit more challenging. We need to:
            //
            // 1. Ensure that the type parameter is exactly Self::{context_ident}.
            // 2. Also generate a transformed type, where the type parameter is
            //    replaced with the unit type.
            RqctxKind::Trait { trait_ident, context_ident } => {
                // We must use the _mut variant, because we're going to mutate
                // the inner type in place as part of our whole deal. ty2 is
                // going to become the transformed type.
                let mut transformed_unit = ty.clone();
                let param = match extract_rqctx_param_mut(&mut transformed_unit)
                {
                    Ok(Some(ty)) => ty,
                    Ok(None) => {
                        // For trait-based macros, this isn't supported -- we
                        // must be able to replace the type parameter with the
                        // unit type.
                        errors.push(Error::new_spanned(
                            ty,
                            rqctx_kind.to_error_message(name_str),
                        ));
                        return None;
                    }
                    Err(_) => {
                        errors.push(Error::new_spanned(
                            ty,
                            rqctx_kind.to_error_message(name_str),
                        ));
                        return None;
                    }
                };

                // The parameter must be exactly Self::{context_ident}.
                let self_context: syn::Type =
                    parse_quote! { Self::#context_ident };
                let self_as_trait_context =
                    parse_quote! { <Self as #trait_ident>::#context_ident };
                if param != &self_context && param != &self_as_trait_context {
                    errors.push(Error::new_spanned(
                        param,
                        rqctx_kind.to_error_message(name_str),
                    ));
                    return None;
                }

                // Now replace the type parameter with the unit type.
                *param = parse_quote! { () };

                Some(Self::Trait { orig: ty, transformed_unit })
            }
        }
    }

    /// Returns the transformed-to-unit type if this is a trait-based
    /// RequestContext, otherwise returns the original type.
    pub(crate) fn transformed_unit_type(&self) -> &syn::Type {
        match self {
            RqctxTy::Function(ty) => ty,
            RqctxTy::Trait { transformed_unit, .. } => transformed_unit,
        }
    }

    /// Returns a token stream that obtains the corresponding context type.
    pub(crate) fn to_context(&self, dropshot: &TokenStream) -> TokenStream {
        let transformed = self.transformed_unit_type();
        quote_spanned! { self.orig_span()=>
            <#transformed as #dropshot::RequestContextArgument>::Context
        }
    }

    /// Returns the original span.
    pub(crate) fn orig_span(&self) -> proc_macro2::Span {
        match self {
            RqctxTy::Function(ty) => ty.span(),
            RqctxTy::Trait { orig, .. } => orig.span(),
        }
    }
}

impl<'ast> fmt::Debug for RqctxTy<'ast> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RqctxTy::Function(ty) => {
                write!(f, "Function({})", ty.to_token_stream())
            }
            RqctxTy::Trait { orig, transformed_unit } => {
                write!(
                    f,
                    "Trait {{ orig: {}, transformed_unit: {} }}",
                    orig.to_token_stream(),
                    transformed_unit.to_token_stream(),
                )
            }
        }
    }
}

/// Extracts and ensures that the type parameter for RequestContext is valid.
fn extract_rqctx_param(
    ty: &syn::Type,
) -> Result<Option<&syn::Type>, RqctxTyError> {
    let syn::Type::Path(p) = ty else {
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

/// Exactly like extract_rqctx_param, but works on mutable references.
fn extract_rqctx_param_mut(
    ty: &mut syn::Type,
) -> Result<Option<&mut syn::Type>, RqctxTyError> {
    let syn::Type::Path(p) = &mut *ty else {
        return Err(RqctxTyError::NotTypePath);
    };

    // Inspect the last path segment.
    let Some(last_segment) = p.path.segments.last_mut() else {
        return Err(RqctxTyError::NoPathSegments);
    };

    // It must either not have type arguments at all, or if so then exactly one
    // argument.
    let a = match &mut last_segment.arguments {
        syn::PathArguments::None => {
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
    let syn::GenericArgument::Type(tp) = a.args.first_mut().unwrap() else {
        return Err(RqctxTyError::ArgNotType);
    };

    Ok(Some(tp))
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum RqctxKind<'a> {
    Function,
    Trait { trait_ident: &'a syn::Ident, context_ident: &'a syn::Ident },
}

impl RqctxKind<'_> {
    fn to_error_message(self, name_str: &str) -> String {
        match self {
            RqctxKind::Function => {
                format!(
                    "endpoint `{name_str}` must accept a \
                     RequestContext<T> as its first argument"
                )
            }
            RqctxKind::Trait { context_ident, .. } => {
                // The <Self as {trait_ident}>::{context_ident} type is too
                // niche to be worth explaining in the error message -- hope
                // that users will figure it out. If not, then we'll have to
                // expand on this.
                format!(
                    "endpoint `{name_str}` must accept \
                     RequestContext<Self::{context_ident}> as its first \
                     argument"
                )
            }
        }
    }
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
enum ParamTyKind {
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
    use quote::{format_ident, quote};
    use syn::parse_quote;

    use super::*;

    #[test]
    fn test_extract_rqctx_ty_param() {
        let some_type = parse_quote! { SomeType };
        let self_context = parse_quote! { Self::Context };
        let self_some_context = parse_quote! { Self::SomeContext };
        let self_as_trait_context =
            parse_quote! { <Self as SomeTrait>::Context };
        let unit = parse_quote! { () };
        let tuple = parse_quote! { (SomeType, OtherType) };

        // Valid types.
        let valid: &[(syn::Type, _)] = &[
            (parse_quote! { RequestContext<SomeType> }, Some(&some_type)),
            // Self types for trait-based macros.
            (
                parse_quote! { RequestContext<Self::Context> },
                Some(&self_context),
            ),
            (
                parse_quote! { RequestContext<Self::SomeContext> },
                Some(&self_some_context),
            ),
            (
                parse_quote! { RequestContext<<Self as SomeTrait>::Context> },
                Some(&self_as_trait_context),
            ),
            // Tuple types.
            (parse_quote! { RequestContext<()> }, Some(&unit)),
            (
                parse_quote! { ::path::to::dropshot::RequestContext<(SomeType, OtherType)> },
                Some(&tuple),
            ),
            // Type alias.
            (parse_quote! { MyRequestContext }, None),
        ];

        // We can't parse parenthesized generic arguments via parse_quote -
        // only supports them via trait bounds. So we have to do this ugly
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

            let mut ty2 = ty.clone();
            match extract_rqctx_param_mut(&mut ty2) {
                Ok(actual) => assert_eq!(
                    *expected,
                    actual.map(|x| &*x),
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

            let mut ty2 = ty.clone();
            match extract_rqctx_param_mut(&mut ty2) {
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
            }
        }
    }
}
