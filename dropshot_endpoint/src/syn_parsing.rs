// Copyright 2021 Oxide Computer Company

use syn::{
    parse::{Parse, ParseStream},
    Attribute, Signature, Visibility,
};

/// Represent an item without concern for its body which may (or may not)
/// contain syntax errors.
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
