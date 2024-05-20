// Copyright 2021-2024 Oxide Computer Company
//
// Portions of this file are adapted from syn (https://github.com/dtolnay/syn),
// and are used under the terms of the Apache 2.0 license.

use quote::{ToTokens, TokenStreamExt};
use syn::{
    braced, bracketed,
    parse::{discouraged::Speculative, Parse, ParseStream},
    punctuated::Punctuated,
    token, Abi, AttrStyle, Attribute, Generics, Ident, ImplRestriction, Result,
    Signature, Token, TraitItem, TypeParamBound, Visibility,
};

/// Represent an item without concern for its body which may (or may not)
/// contain syntax errors.
#[derive(Clone)]
pub(crate) struct ItemFnForSignature {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub sig: Signature,
    pub _block: proc_macro2::TokenStream,
}

impl Parse for ItemFnForSignature {
    fn parse(input: ParseStream) -> syn::parse::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        let sig: Signature = input.parse()?;
        let block = input.parse()?;
        Ok(ItemFnForSignature { attrs, vis, sig, _block: block })
    }
}

/// Represent a trait with partially parsed functions.
///
/// Only function signatures are parsed, not their bodies.
#[derive(Clone)]
pub(crate) struct ItemTraitForFnSignatures {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub unsafety: Option<Token![unsafe]>,
    pub auto_token: Option<Token![auto]>,
    // As of syn 2.0.63, "restriction" is reserved for RFC 3323 restrictions.
    #[allow(dead_code)]
    pub restriction: Option<ImplRestriction>,
    pub trait_token: Token![trait],
    pub ident: Ident,
    pub generics: Generics,
    pub colon_token: Option<Token![:]>,
    pub supertraits: Punctuated<TypeParamBound, Token![+]>,
    pub brace_token: token::Brace,
    pub items: Vec<TraitItemForEndpoint>,
}

impl Parse for ItemTraitForFnSignatures {
    fn parse(input: ParseStream) -> syn::parse::Result<Self> {
        // Adapted from syn.
        let outer_attrs = input.call(Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        let unsafety: Option<Token![unsafe]> = input.parse()?;
        let auto_token: Option<Token![auto]> = input.parse()?;
        let trait_token: Token![trait] = input.parse()?;
        let ident: Ident = input.parse()?;
        let generics: Generics = input.parse()?;
        parse_rest_of_trait(
            input,
            outer_attrs,
            vis,
            unsafety,
            auto_token,
            trait_token,
            ident,
            generics,
        )
    }
}

fn parse_rest_of_trait(
    input: ParseStream,
    mut attrs: Vec<Attribute>,
    vis: Visibility,
    unsafety: Option<Token![unsafe]>,
    auto_token: Option<Token![auto]>,
    trait_token: Token![trait],
    ident: Ident,
    mut generics: Generics,
) -> Result<ItemTraitForFnSignatures> {
    // Adapted from syn.
    let colon_token: Option<Token![:]> = input.parse()?;

    let mut supertraits = Punctuated::new();
    if colon_token.is_some() {
        loop {
            if input.peek(Token![where]) || input.peek(token::Brace) {
                break;
            }
            supertraits.push_value(input.parse()?);
            if input.peek(Token![where]) || input.peek(token::Brace) {
                break;
            }
            supertraits.push_punct(input.parse()?);
        }
    }

    generics.where_clause = input.parse()?;

    let content;
    let brace_token = braced!(content in input);
    parse_inner(&content, &mut attrs)?;
    let mut items = Vec::new();
    while !content.is_empty() {
        items.push(content.parse()?);
    }

    Ok(ItemTraitForFnSignatures {
        attrs,
        vis,
        unsafety,
        auto_token,
        restriction: None,
        trait_token,
        ident,
        generics,
        colon_token,
        supertraits,
        brace_token,
        items,
    })
}

impl ToTokens for ItemTraitForFnSignatures {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        // Adapted from syn.
        let Self {
            attrs,
            vis,
            unsafety,
            auto_token,
            restriction: _,
            trait_token,
            ident,
            generics,
            colon_token,
            supertraits,
            brace_token,
            items,
        } = self;

        tokens.append_all(attrs.outer());
        vis.to_tokens(tokens);
        unsafety.to_tokens(tokens);
        auto_token.to_tokens(tokens);
        trait_token.to_tokens(tokens);
        ident.to_tokens(tokens);
        generics.to_tokens(tokens);
        if !supertraits.is_empty() {
            TokensOrDefault(&colon_token).to_tokens(tokens);
            supertraits.to_tokens(tokens);
        }
        generics.where_clause.to_tokens(tokens);
        brace_token.surround(tokens, |tokens| {
            tokens.append_all(attrs.inner());
            tokens.append_all(items);
        });
    }
}

/// Similar to `syn::TraitItem`, except function bodies aren't parsed.
#[derive(Clone)]
pub(crate) enum TraitItemForEndpoint {
    /// An associated function within the definition of a trait.
    Fn(TraitItemFnForSignature),

    /// Something else.
    Other(TraitItem),
}

impl Parse for TraitItemForEndpoint {
    fn parse(input: ParseStream) -> syn::parse::Result<Self> {
        // The only case we need to consider is a function -- for everything
        // else, we defer to syn.
        //
        // As an alternative, we could reimplement all of TraitItem::parse here.
        // That would have the advantage of no longer requiring the `advance_to`
        // That has the distinct disadvantage that we'd have to keep up with
        // changes to syn's parsing implementations, even more than we have to
        // now.
        //
        // Delegating to syn saves us that hassle.
        let begin = input.fork();
        let mut attrs = input.call(Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        let defaultness: Option<Token![default]> = input.parse()?;

        let other: TraitItem = match (vis, defaultness) {
            (Visibility::Inherited, None) => {
                let ahead = input.fork();
                let lookahead = ahead.lookahead1();
                if lookahead.peek(Token![fn]) || peek_signature(&ahead) {
                    // This is a function signature.
                    let mut fn_item: TraitItemFnForSignature = input.parse()?;
                    attrs.append(&mut fn_item.attrs);
                    fn_item.attrs = attrs;
                    return Ok(Self::Fn(fn_item));
                } else {
                    // Restart parsing from the beginning, giving it to syn.
                    begin.parse()?
                }
            }
            _ => {
                // vis and defaultness aren't supported by us, so delegate to
                // syn. As of version 2.0.63, upstream syn will use the
                // verbatim parser in this case.
                begin.parse()?
            }
        };

        if let TraitItem::Fn(f) = &other {
            return Err(syn::Error::new_spanned(
                f.sig.fn_token,
                "functions should have been handled separately, \
                we should never get here",
            ));
        }

        // Speculative::advance_to is discouraged by syn in cases where users
        // attempt to speculatively parse content with one parser, then fall
        // back to another. That can lead to bad or duplicated error messages.
        //
        // In our case, we are switching to another parser, not falling back to
        // it after failing to parse the input one way -- so that downside
        // doesn't apply.
        input.advance_to(&begin);

        Ok(Self::Other(other))
    }
}

impl ToTokens for TraitItemForEndpoint {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::Fn(f) => f.to_tokens(tokens),
            Self::Other(o) => o.to_tokens(tokens),
        }
    }
}

/// An associated function within the definition of a trait.
#[derive(Clone)]
pub(crate) struct TraitItemFnForSignature {
    pub attrs: Vec<Attribute>,
    pub sig: Signature,
    pub block: Option<UnparsedBlock>,
    pub semi_token: Option<Token![;]>,
}

impl Parse for TraitItemFnForSignature {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut attrs = input.call(Attribute::parse_outer)?;
        let sig: Signature = input.parse()?;

        let lookahead = input.lookahead1();
        let (block, semi_token) = if lookahead.peek(token::Brace) {
            let content;
            let brace_token = braced!(content in input);
            parse_inner(&content, &mut attrs)?;
            // Here's where we diverge from syn::TraitItemFn -- we don't parse
            // the function body.
            let tokens = content.parse()?;
            (Some(UnparsedBlock { brace_token, tokens }), None)
        } else if lookahead.peek(Token![;]) {
            let semi_token: Token![;] = input.parse()?;
            (Default::default(), Some(semi_token))
        } else {
            return Err(lookahead.error());
        };

        Ok(Self { attrs, sig, block, semi_token })
    }
}

impl ToTokens for TraitItemFnForSignature {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        tokens.append_all(self.attrs.outer());
        self.sig.to_tokens(tokens);
        match &self.block {
            Some(block) => {
                block.to_tokens(tokens);
            }
            None => {
                TokensOrDefault(&self.semi_token).to_tokens(tokens);
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct UnparsedBlock {
    pub brace_token: token::Brace,
    pub tokens: proc_macro2::TokenStream,
}

impl ToTokens for UnparsedBlock {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.brace_token.surround(tokens, |tokens| {
            tokens.extend(self.tokens.clone());
        });
    }
}

// --- Helper functions, all adapted from syn. ---

fn peek_signature(input: ParseStream) -> bool {
    let fork = input.fork();
    fork.parse::<Option<Token![const]>>().is_ok()
        && fork.parse::<Option<Token![async]>>().is_ok()
        && fork.parse::<Option<Token![unsafe]>>().is_ok()
        && fork.parse::<Option<Abi>>().is_ok()
        && fork.peek(Token![fn])
}

fn parse_inner(input: ParseStream, attrs: &mut Vec<Attribute>) -> Result<()> {
    while input.peek(Token![#]) && input.peek2(Token![!]) {
        attrs.push(input.call(single_parse_inner)?);
    }
    Ok(())
}

fn single_parse_inner(input: ParseStream) -> Result<Attribute> {
    let content;
    Ok(Attribute {
        pound_token: input.parse()?,
        style: AttrStyle::Inner(input.parse()?),
        bracket_token: bracketed!(content in input),
        meta: content.parse()?,
    })
}

trait FilterAttrs<'a> {
    type Ret: Iterator<Item = &'a Attribute>;

    fn outer(self) -> Self::Ret;
    fn inner(self) -> Self::Ret;
}

impl<'a> FilterAttrs<'a> for &'a [Attribute] {
    type Ret = std::iter::Filter<
        std::slice::Iter<'a, Attribute>,
        fn(&&Attribute) -> bool,
    >;

    fn outer(self) -> Self::Ret {
        fn is_outer(attr: &&Attribute) -> bool {
            match attr.style {
                AttrStyle::Outer => true,
                AttrStyle::Inner(_) => false,
            }
        }
        self.iter().filter(is_outer)
    }

    fn inner(self) -> Self::Ret {
        fn is_inner(attr: &&Attribute) -> bool {
            match attr.style {
                AttrStyle::Inner(_) => true,
                AttrStyle::Outer => false,
            }
        }
        self.iter().filter(is_inner)
    }
}
struct TokensOrDefault<'a, T: 'a>(&'a Option<T>);

impl<'a, T> ToTokens for TokensOrDefault<'a, T>
where
    T: ToTokens + Default,
{
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self.0 {
            Some(t) => t.to_tokens(tokens),
            None => T::default().to_tokens(tokens),
        }
    }
}

#[cfg(test)]
mod tests {
    use quote::quote;
    use syn::{Signature, Visibility};

    use crate::syn_parsing::ItemFnForSignature;

    #[test]
    fn test_busted_function() {
        let f = quote! {
            fn foo(parameter: u32) -> u32 {
                (void) printf("%s", "no language C here!");
                return (0);
            }
        };
        let ast: ItemFnForSignature = syn::parse2(f).unwrap();

        let sig: Signature = syn::parse2(quote! {
            fn foo(parameter: u32) -> u32
        })
        .unwrap();

        assert!(ast.attrs.is_empty());
        assert_eq!(ast.vis, Visibility::Inherited);
        assert_eq!(ast.sig, sig);
    }
}
