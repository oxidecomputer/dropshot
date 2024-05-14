// Copyright 2023 Oxide Computer Company

use quote::ToTokens;

const DROPSHOT: &str = "dropshot";

/// Given an optional string, returns the crate name as a token stream.
pub(crate) fn get_crate(var: Option<String>) -> proc_macro2::TokenStream {
    if let Some(s) = var {
        if let Ok(ts) = syn::parse_str(s.as_str()) {
            return ts;
        }
    }
    syn::Ident::new(DROPSHOT, proc_macro2::Span::call_site()).to_token_stream()
}

/// Turns doc comments into endpoint documentation.
pub(crate) fn extract_doc_from_attrs(
    attrs: &[syn::Attribute],
) -> (Option<String>, Option<String>) {
    let doc = syn::Ident::new("doc", proc_macro2::Span::call_site());

    let mut lines = attrs.iter().flat_map(|attr| {
        if let syn::Meta::NameValue(nv) = &attr.meta {
            if nv.path.is_ident(&doc) {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                }) = &nv.value
                {
                    return normalize_comment_string(s.value());
                }
            }
        }
        Vec::new()
    });

    // Skip initial blank lines; they make for excessively terse summaries.
    let summary = loop {
        match lines.next() {
            Some(s) if s.is_empty() => (),
            next => break next,
        }
    };
    // Skip initial blank description lines.
    let first = loop {
        match lines.next() {
            Some(s) if s.is_empty() => (),
            next => break next,
        }
    };

    match (summary, first) {
        (None, _) => (None, None),
        (summary, None) => (summary, None),
        (Some(summary), Some(first)) => (
            Some(summary),
            Some(
                lines
                    .fold(first, |acc, comment| {
                        if acc.ends_with('-')
                            || acc.ends_with('\n')
                            || acc.is_empty()
                        {
                            // Continuation lines and newlines.
                            format!("{}{}", acc, comment)
                        } else if comment.is_empty() {
                            // Handle fully blank comments as newlines we keep.
                            format!("{}\n", acc)
                        } else {
                            // Default to space-separating comment fragments.
                            format!("{} {}", acc, comment)
                        }
                    })
                    .trim_end()
                    .to_string(),
            ),
        ),
    }
}

fn normalize_comment_string(s: String) -> Vec<String> {
    s.split('\n')
        .enumerate()
        .map(|(idx, s)| {
            // Rust-style comments are intrinsically single-line. We don't want
            // to trim away formatting such as an initial '*'.
            if idx == 0 {
                s.trim_start().trim_end()
            } else {
                let trimmed = s.trim_start().trim_end();
                trimmed.strip_prefix("* ").unwrap_or_else(|| {
                    trimmed.strip_prefix('*').unwrap_or(trimmed)
                })
            }
        })
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use schema::Schema;

    #[test]
    fn test_extract_summary_description() {
        /// Javadoc summary
        /// Maybe there's another name for these...
        /// ... but Java is the first place I saw these types of comments.
        #[derive(Schema)]
        struct JavadocComments;
        assert_eq!(
            extract_doc_from_attrs(&JavadocComments::schema().attrs),
            (
                Some("Javadoc summary".to_string()),
                Some(
                    "Maybe there's another name for these... ... but Java \
                    is the first place I saw these types of comments."
                        .to_string()
                )
            )
        );

        /// Javadoc summary
        ///
        /// Skip that blank.
        #[derive(Schema)]
        struct JavadocCommentsWithABlank;
        assert_eq!(
            extract_doc_from_attrs(&JavadocCommentsWithABlank::schema().attrs),
            (
                Some("Javadoc summary".to_string()),
                Some("Skip that blank.".to_string())
            )
        );

        /// Terse Javadoc summary
        #[derive(Schema)]
        struct JavadocCommentsTerse;
        assert_eq!(
            extract_doc_from_attrs(&JavadocCommentsTerse::schema().attrs),
            (Some("Terse Javadoc summary".to_string()), None)
        );

        /// Rustdoc summary
        /// Did other folks do this or what this an invention I can right-
        /// fully ascribe to Rust?
        #[derive(Schema)]
        struct RustdocComments;
        assert_eq!(
            extract_doc_from_attrs(&RustdocComments::schema().attrs),
            (
                Some("Rustdoc summary".to_string()),
                Some(
                    "Did other folks do this or what this an invention \
                    I can right-fully ascribe to Rust?"
                        .to_string()
                )
            )
        );

        /// Rustdoc summary
        ///
        /// Skip that blank.
        #[derive(Schema)]
        struct RustdocCommentsWithABlank;
        assert_eq!(
            extract_doc_from_attrs(&RustdocCommentsWithABlank::schema().attrs),
            (
                Some("Rustdoc summary".to_string()),
                Some("Skip that blank.".to_string())
            )
        );

        /// Just a Rustdoc summary
        #[derive(Schema)]
        struct JustTheRustdocSummary;
        assert_eq!(
            extract_doc_from_attrs(&JustTheRustdocSummary::schema().attrs),
            (Some("Just a Rustdoc summary".to_string()), None)
        );

        /// Just a Javadoc summary
        #[derive(Schema)]
        struct JustTheJavadocSummary;
        assert_eq!(
            extract_doc_from_attrs(&JustTheJavadocSummary::schema().attrs),
            (Some("Just a Javadoc summary".to_string()), None)
        );

        /// Summary
        /// Text
        /// More
        ///
        /// Even
        /// More
        #[derive(Schema)]
        struct SummaryDescriptionBreak;
        assert_eq!(
            extract_doc_from_attrs(&SummaryDescriptionBreak::schema().attrs),
            (
                Some("Summary".to_string()),
                Some("Text More\nEven More".to_string())
            )
        );
    }
}
