// Copyright 2025 Oxide Computer Company

use crate::{
    apis::ManagedApis,
    output::{display_api_spec, display_error, plural, OutputOpts, Styles},
};
use indent_write::io::IndentWriter;
use openapiv3::OpenAPI;
use owo_colors::OwoColorize;
use std::io::Write;

pub(crate) fn list_impl(
    apis: &ManagedApis,
    verbose: bool,
    output: &OutputOpts,
) -> anyhow::Result<()> {
    let mut styles = Styles::default();
    if output.use_color(supports_color::Stream::Stdout) {
        styles.colorize();
    }
    let mut out = std::io::stdout();

    let total = apis.len();
    let count_width = total.to_string().len();

    if verbose {
        // A string for verbose indentation. +1 for the closing ), and +2 for
        // further indentation.
        let initial_indent = " ".repeat(count_width + 1 + 2);
        // This plus 4 more for continued indentation.
        let continued_indent = " ".repeat(count_width + 1 + 2 + 4);

        for (ix, api) in apis.iter_apis().enumerate() {
            let count = ix + 1;

            writeln!(
                &mut out,
                "{count:count_width$}) {}",
                api.ident().style(styles.bold),
            )?;

            writeln!(
                &mut out,
                "{initial_indent} {}: {} ({})",
                "title".style(styles.header),
                api.title(),
                if api.is_versioned() { "versioned" } else { "lockstep" },
            )?;

            let metadata = api.metadata();
            if let Some(description) = metadata.description {
                write!(
                    &mut out,
                    "{initial_indent} {}: ",
                    "description".style(styles.header)
                )?;
                writeln!(
                    IndentWriter::new_skip_initial(&continued_indent, &mut out),
                    "{}",
                    description,
                )?;
            }
            if let Some(contact_url) = metadata.contact_url {
                write!(
                    &mut out,
                    "{initial_indent} {}: ",
                    "contact url".style(styles.header)
                )?;
                writeln!(
                    IndentWriter::new_skip_initial(&continued_indent, &mut out),
                    "{}",
                    contact_url,
                )?;
            }
            if let Some(contact_email) = metadata.contact_email {
                write!(
                    &mut out,
                    "{initial_indent} {}: ",
                    "contact email".style(styles.header)
                )?;
                writeln!(
                    IndentWriter::new_skip_initial(&continued_indent, &mut out),
                    "{}",
                    contact_email,
                )?;
            }

            if !metadata.extra.is_null() {
                writeln!(
                    &mut out,
                    "{initial_indent} {}: {}",
                    "extra".style(styles.header),
                    metadata.extra,
                )?;
            }

            writeln!(
                &mut out,
                "{initial_indent} {}:",
                "spec details".style(styles.header),
            )?;

            for v in api.iter_versions_semver() {
                match api.generate_openapi_doc(v) {
                    Ok(openapi) => {
                        let summary = DocumentSummary::new(&openapi);
                        let num_schemas = summary.schema_count.map_or_else(
                            || "(data missing)".to_owned(),
                            |c| format!("{} {}", c, plural::schemas(c)),
                        );
                        writeln!(
                            &mut out,
                            "{continued_indent} {}: {} {}, {}",
                            format!("v{}", v).style(styles.header),
                            summary.path_count.style(styles.bold),
                            plural::paths(summary.path_count),
                            num_schemas
                        )?;
                    }
                    Err(error) => {
                        write!(
                            &mut out,
                            "{initial_indent} {}: ",
                            "error".style(styles.failure),
                        )?;
                        let display = display_error(&error, styles.failure);
                        write!(
                            IndentWriter::new_skip_initial(
                                &continued_indent,
                                std::io::stderr(),
                            ),
                            "{}",
                            display,
                        )?;
                    }
                };
            }

            if ix + 1 < total {
                writeln!(&mut out)?;
            }
        }
    } else {
        for (ix, spec) in apis.iter_apis().enumerate() {
            let count = ix + 1;

            writeln!(
                &mut out,
                "{count:count_width$}) {}",
                display_api_spec(spec, &styles),
            )?;
        }

        writeln!(
            &mut out,
            "note: run with {} for more information",
            "-v".style(styles.bold),
        )?;
    }

    Ok(())
}

#[derive(Debug)]
struct DocumentSummary {
    path_count: usize,
    // None if data is missing.
    schema_count: Option<usize>,
}

impl DocumentSummary {
    fn new(doc: &OpenAPI) -> Self {
        Self {
            path_count: doc.paths.paths.len(),
            schema_count: doc
                .components
                .as_ref()
                .map_or(None, |c| Some(c.schemas.len())),
        }
    }
}
