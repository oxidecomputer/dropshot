// Copyright 2023 Oxide Computer Company

use std::collections::{BTreeSet, HashSet};

use syn::visit::Visit;

/// Assert that the provided identifiers are not in use within a particular file.
pub(crate) fn assert_banned_idents<'a>(
    file: &syn::File,
    idents: impl IntoIterator<Item = &'a str>,
) {
    let idents: HashSet<_> = idents.into_iter().collect();

    fn assert_banned_idents<'a>(file: &syn::File, idents: HashSet<&'a str>) {
        let mut visitor =
            BanIdentsVisitor { idents: &idents, found: BTreeSet::new() };
        visitor.visit_file(file);

        if !visitor.found.is_empty() {
            let found = visitor.found.into_iter().collect::<Vec<_>>();
            panic!("banned identifiers found in file: {}", found.join(", "));
        }
    }

    assert_banned_idents(file, idents);
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
    fn visit_ident(&mut self, ident: &syn::Ident) {
        let ident = ident.to_string();
        if self.idents.contains(&ident.as_str()) {
            self.found.insert(ident);
        }
    }
}
