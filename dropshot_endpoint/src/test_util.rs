// Copyright 2024 Oxide Computer Company

use std::collections::{BTreeSet, HashSet};

use syn::visit::Visit;

/// Assert that the provided identifiers are not in use within a particular file.
pub(crate) fn assert_banned_idents<'a>(
    file: &syn::File,
    idents: impl IntoIterator<Item = &'a str>,
) {
    let found = find_idents(file, idents);
    if !found.is_empty() {
        let found = found.into_iter().collect::<Vec<_>>();
        panic!("banned identifiers found in file: {}", found.join(", "));
    }
}

/// Look for the provided identifiers in use within a particular file.
///
/// [`assert_banned_idents`] is a wrapper around this function that panics if
/// any of the provided identifiers are found.
pub(crate) fn find_idents<'a>(
    file: &syn::File,
    idents: impl IntoIterator<Item = &'a str>,
) -> BTreeSet<String> {
    let idents: HashSet<_> = idents.into_iter().collect();

    let mut visitor =
        BanIdentsVisitor { idents: &idents, found: BTreeSet::new() };
    visitor.visit_file(file);

    visitor.found
}

/// A syn visitor that bans the provided identifiers.
///
/// Used to ensure that if custom identifiers are provided, the generated code
/// doesn't use the default identifiers.
struct BanIdentsVisitor<'a> {
    idents: &'a HashSet<&'a str>,
    found: BTreeSet<String>,
}

impl<'a> syn::visit::Visit<'a> for BanIdentsVisitor<'a> {
    fn visit_ident(&mut self, ident: &'a syn::Ident) {
        let ident = ident.to_string();
        if self.idents.contains(&ident.as_str()) {
            self.found.insert(ident);
        }
    }
}
