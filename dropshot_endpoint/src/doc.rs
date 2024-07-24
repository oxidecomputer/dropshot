// Copyright 2024 Oxide Computer Company

use proc_macro2::TokenStream;
use quote::quote;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExtractedDoc {
    pub(crate) summary: Option<String>,
    pub(crate) description: Option<String>,
}

impl ExtractedDoc {
    pub(crate) fn from_attrs(attrs: &[syn::Attribute]) -> Self {
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

        let description = first.map(|first| {
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
                        format!("{}\n\n", acc)
                    } else {
                        // Default to space-separating comment fragments.
                        format!("{} {}", acc, comment)
                    }
                })
                .trim_end()
                .to_string()
        });

        Self { summary, description }
    }

    pub(crate) fn comment_text(&self, name: &str) -> String {
        let mut buf = String::new();
        buf.push_str("API Endpoint: ");
        buf.push_str(name);
        if let Some(s) = &self.summary {
            buf.push_str("\n");
            buf.push_str(&s);
        }
        if let Some(s) = &self.description {
            buf.push_str("\n");
            buf.push_str(&s);
        }

        buf
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

pub(crate) fn string_to_doc_attrs(s: &str) -> TokenStream {
    let lines = s.lines().map(|line| {
        // Add a preceding space to make it look nice.
        let line = format!(" {line}");
        quote! {
            #[doc = #line]
        }
    });

    quote! {
        #(#lines)*
    }
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
            ExtractedDoc::from_attrs(&JavadocComments::schema().attrs),
            ExtractedDoc {
                summary: Some("Javadoc summary".to_string()),
                description: Some(
                    "Maybe there's another name for these... ... but Java \
                    is the first place I saw these types of comments."
                        .to_string()
                )
            },
        );

        /// Javadoc summary
        ///
        /// Skip that blank.
        #[derive(Schema)]
        struct JavadocCommentsWithABlank;
        assert_eq!(
            ExtractedDoc::from_attrs(
                &JavadocCommentsWithABlank::schema().attrs
            ),
            ExtractedDoc {
                summary: Some("Javadoc summary".to_string()),
                description: Some("Skip that blank.".to_string())
            },
        );

        /// Terse Javadoc summary
        #[derive(Schema)]
        struct JavadocCommentsTerse;
        assert_eq!(
            ExtractedDoc::from_attrs(&JavadocCommentsTerse::schema().attrs),
            ExtractedDoc {
                summary: Some("Terse Javadoc summary".to_string()),
                description: None
            },
        );

        /// Rustdoc summary
        /// Did other folks do this or what this an invention I can right-
        /// fully ascribe to Rust?
        #[derive(Schema)]
        struct RustdocComments;
        assert_eq!(
            ExtractedDoc::from_attrs(&RustdocComments::schema().attrs),
            ExtractedDoc {
                summary: Some("Rustdoc summary".to_string()),
                description: Some(
                    "Did other folks do this or what this an invention I can \
                    right-fully ascribe to Rust?"
                        .to_string()
                )
            },
        );

        /// Rustdoc summary
        ///
        /// Skip that blank.
        #[derive(Schema)]
        struct RustdocCommentsWithABlank;
        assert_eq!(
            ExtractedDoc::from_attrs(
                &RustdocCommentsWithABlank::schema().attrs
            ),
            ExtractedDoc {
                summary: Some("Rustdoc summary".to_string()),
                description: Some("Skip that blank.".to_string())
            },
        );

        /// Just a Rustdoc summary
        #[derive(Schema)]
        struct JustTheRustdocSummary;
        assert_eq!(
            ExtractedDoc::from_attrs(&JustTheRustdocSummary::schema().attrs),
            ExtractedDoc {
                summary: Some("Just a Rustdoc summary".to_string()),
                description: None
            },
        );

        /// Just a Javadoc summary
        #[derive(Schema)]
        struct JustTheJavadocSummary;
        assert_eq!(
            ExtractedDoc::from_attrs(&JustTheJavadocSummary::schema().attrs),
            ExtractedDoc {
                summary: Some("Just a Javadoc summary".to_string()),
                description: None
            },
        );

        /// Summary
        /// Text
        /// More
        ///
        /// Even
        /// More
        ///
        ///
        ///
        /// And another
        /// paragraph
        #[derive(Schema)]
        struct SummaryDescriptionBreak;
        assert_eq!(
            ExtractedDoc::from_attrs(&SummaryDescriptionBreak::schema().attrs),
            ExtractedDoc {
                summary: Some("Summary".to_string()),
                description: Some(
                    "Text More\n\nEven More\n\nAnd another paragraph"
                        .to_string()
                )
            },
        );
    }
}
