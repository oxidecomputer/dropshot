// Copyright 2025 Oxide Computer Company

use crate::apis::{ManagedApi, ManagedApis};
use crate::environment::{ErrorAccumulator, ResolvedEnv};
use crate::resolved::{Problem, Resolution, ResolutionKind, Resolved};
use crate::validation::CheckStale;
use anyhow::bail;
use camino::Utf8Path;
use clap::{Args, ColorChoice};
use headers::*;
use indent_write::fmt::IndentWriter;
use owo_colors::{OwoColorize, Style};
use similar::{ChangeTag, DiffableStr, TextDiff};
use std::{fmt, fmt::Write, io};

#[derive(Debug, Args)]
#[clap(next_help_heading = "Global options")]
pub struct OutputOpts {
    /// Color output
    #[clap(long, value_enum, global = true, default_value_t)]
    pub(crate) color: ColorChoice,
}

impl OutputOpts {
    /// Returns true if color should be used for the stream.
    pub(crate) fn use_color(&self, stream: supports_color::Stream) -> bool {
        match self.color {
            ColorChoice::Auto => supports_color::on_cached(stream).is_some(),
            ColorChoice::Always => true,
            ColorChoice::Never => false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Styles {
    pub(crate) bold: Style,
    pub(crate) header: Style,
    pub(crate) success_header: Style,
    pub(crate) failure: Style,
    pub(crate) failure_header: Style,
    pub(crate) warning_header: Style,
    pub(crate) unchanged_header: Style,
    pub(crate) filename: Style,
    pub(crate) diff_before: Style,
    pub(crate) diff_after: Style,
}

impl Styles {
    pub(crate) fn colorize(&mut self) {
        self.bold = Style::new().bold();
        self.header = Style::new().purple();
        self.success_header = Style::new().green().bold();
        self.failure = Style::new().red();
        self.failure_header = Style::new().red().bold();
        self.unchanged_header = Style::new().blue().bold();
        self.warning_header = Style::new().yellow().bold();
        self.filename = Style::new().cyan();
        self.diff_before = Style::new().red();
        self.diff_after = Style::new().green();
    }
}

// This is copied from similar's UnifiedDiff::to_writer, except with colorized
// output.
pub(crate) fn write_diff<'diff, 'old, 'new, 'bufs>(
    diff: &'diff TextDiff<'old, 'new, 'bufs, [u8]>,
    path1: &Utf8Path,
    path2: &Utf8Path,
    styles: &Styles,
    out: &mut dyn io::Write,
) -> io::Result<()>
where
    'diff: 'old + 'new + 'bufs,
{
    // The "a/" (/ courtesy full_path) and "b/" make it feel more like git diff.
    let a = Utf8Path::new("a").join(path1);
    writeln!(out, "{}", format!("--- {a}").style(styles.diff_before))?;
    let b = Utf8Path::new("b").join(path2);
    writeln!(out, "{}", format!("+++ {b}").style(styles.diff_after))?;

    let udiff = diff.unified_diff();
    for hunk in udiff.iter_hunks() {
        for (idx, change) in hunk.iter_changes().enumerate() {
            if idx == 0 {
                writeln!(out, "{}", hunk.header())?;
            }
            let style = match change.tag() {
                ChangeTag::Delete => styles.diff_before,
                ChangeTag::Insert => styles.diff_after,
                ChangeTag::Equal => Style::new(),
            };

            write!(out, "{}", change.tag().style(style))?;
            write!(out, "{}", change.value().to_string_lossy().style(style))?;
            if !diff.newline_terminated() {
                writeln!(out)?;
            }
            if diff.newline_terminated() && change.missing_newline() {
                writeln!(
                    out,
                    "{}",
                    MissingNewlineHint(hunk.missing_newline_hint())
                )?;
            }
        }
    }

    Ok(())
}

pub(crate) fn display_api_spec(api: &ManagedApi, styles: &Styles) -> String {
    let versions: Vec<_> = api.iter_versions_semver().collect();
    let latest_version = versions.last().expect("must be at least one version");
    if api.is_versioned() {
        format!(
            "{} ({}, versioned ({} supported), latest = {})",
            api.ident().style(styles.filename),
            api.title(),
            versions.len(),
            latest_version,
        )
    } else {
        format!(
            "{} ({}, lockstep, v{})",
            api.ident().style(styles.filename),
            api.title(),
            latest_version,
        )
    }
}

pub(crate) fn display_api_spec_version(
    api: &ManagedApi,
    version: &semver::Version,
    styles: &Styles,
    resolution: &Resolution<'_>,
) -> String {
    if api.is_lockstep() {
        assert_eq!(resolution.kind(), ResolutionKind::Lockstep);
        format!(
            "{} (lockstep v{}): {}",
            api.ident().style(styles.filename),
            version,
            api.title(),
        )
    } else {
        format!(
            "{} (versioned v{} ({})): {}",
            api.ident().style(styles.filename),
            version,
            resolution.kind(),
            api.title(),
        )
    }
}

pub(crate) fn display_error(
    error: &anyhow::Error,
    failure_style: Style,
) -> impl fmt::Display + '_ {
    struct DisplayError<'a> {
        error: &'a anyhow::Error,
        failure_style: Style,
    }

    impl fmt::Display for DisplayError<'_> {
        fn fmt(&self, mut f: &mut fmt::Formatter<'_>) -> fmt::Result {
            writeln!(f, "{}", self.error.style(self.failure_style))?;

            let mut source = self.error.source();
            while let Some(curr) = source {
                write!(f, "-> ")?;
                writeln!(
                    IndentWriter::new_skip_initial("   ", &mut f),
                    "{}",
                    curr.style(self.failure_style),
                )?;
                source = curr.source();
            }

            Ok(())
        }
    }

    DisplayError { error, failure_style }
}

struct MissingNewlineHint(bool);

impl fmt::Display for MissingNewlineHint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 {
            write!(f, "\n\\ No newline at end of file")?;
        }
        Ok(())
    }
}

pub fn display_load_problems(
    error_accumulator: &ErrorAccumulator,
    styles: &Styles,
) -> anyhow::Result<()> {
    for w in error_accumulator.iter_warnings() {
        eprintln!(
            "{:>HEADER_WIDTH$} {:#}",
            WARNING.style(styles.warning_header),
            w
        );
    }

    let mut nerrors = 0;
    for e in error_accumulator.iter_errors() {
        nerrors += 1;
        println!(
            "{:>HEADER_WIDTH$} {:#}",
            FAILURE.style(styles.failure_header),
            e
        );
    }

    if nerrors > 0 {
        bail!(
            "bailing out after {} {} above",
            nerrors,
            plural::errors(nerrors)
        );
    }

    Ok(())
}

/// Summarize the results of checking all supported API versions, plus other
/// problems found during resolution
pub fn display_resolution(
    env: &ResolvedEnv,
    apis: &ManagedApis,
    resolved: &Resolved,
    styles: &Styles,
) -> anyhow::Result<CheckResult> {
    let total = resolved.nexpected_documents();

    eprintln!(
        "{:>HEADER_WIDTH$} {} OpenAPI {}...",
        CHECKING.style(styles.success_header),
        total.style(styles.bold),
        plural::documents(total),
    );

    let mut num_fresh = 0;
    let mut num_stale = 0;
    let mut num_failed = 0;
    let mut num_general_problems = 0;

    // Print problems associated with a supported API version
    // (i.e., one of the expected OpenAPI documents).
    for api in apis.iter_apis() {
        let ident = api.ident();

        for version in api.iter_versions_semver() {
            let resolution = resolved
                .resolution_for_api_version(ident, version)
                .expect("resolution for all supported API versions");
            if resolution.has_errors() {
                num_failed += 1;
            } else if resolution.has_problems() {
                num_stale += 1;
            } else {
                num_fresh += 1;
            }
            summarize_one(env, api, version, resolution, styles);
        }

        if !api.is_versioned() {
            continue;
        }

        if let Some(symlink_problem) = resolved.symlink_problem(ident) {
            if symlink_problem.is_fixable() {
                num_general_problems += 1;
                eprintln!(
                    "{:>HEADER_WIDTH$} {} \"latest\" symlink",
                    STALE.style(styles.warning_header),
                    ident.style(styles.filename),
                );
                display_resolution_problems(
                    env,
                    std::iter::once(symlink_problem),
                    styles,
                );
            } else {
                num_failed += 1;
                eprintln!(
                    "{:>HEADER_WIDTH$} {} \"latest\" symlink",
                    FAILURE.style(styles.failure_header),
                    ident.style(styles.filename),
                );
                display_resolution_problems(
                    env,
                    std::iter::once(symlink_problem),
                    styles,
                );
            }
        } else {
            num_fresh += 1;
            eprintln!(
                "{:>HEADER_WIDTH$} {} \"latest\" symlink",
                FRESH.style(styles.success_header),
                ident.style(styles.filename),
            );
        }
    }

    // Print problems not associated with any supported version, if any.
    let general_problems: Vec<_> = resolved.general_problems().collect();
    num_general_problems += if !general_problems.is_empty() {
        eprintln!(
            "\n{:>HEADER_WIDTH$} problems not associated with a specific \
             supported API version:",
            "Other".style(styles.warning_header),
        );

        let (fixable, unfixable): (Vec<&Problem>, Vec<&Problem>) =
            general_problems.iter().partition(|p| p.is_fixable());
        num_failed += unfixable.len();
        display_resolution_problems(env, general_problems, styles);
        fixable.len()
    } else {
        0
    };

    // Print informational notes, if any.
    for n in resolved.notes() {
        let initial_indent =
            format!("{:>HEADER_WIDTH$} ", "Note".style(styles.warning_header));
        let more_indent = " ".repeat(HEADER_WIDTH + " ".len());
        eprintln!(
            "\n{}\n",
            textwrap::fill(
                &n.to_string(),
                textwrap::Options::with_termwidth()
                    .initial_indent(&initial_indent)
                    .subsequent_indent(&more_indent)
            )
        );
    }

    // Print a summary line.
    let status_header = if num_failed > 0 {
        FAILURE.style(styles.failure_header)
    } else if num_stale > 0 || num_general_problems > 0 {
        STALE.style(styles.warning_header)
    } else {
        SUCCESS.style(styles.success_header)
    };

    eprintln!("{:>HEADER_WIDTH$}", SEPARATOR);
    eprintln!(
        "{:>HEADER_WIDTH$} {} {} checked: {} fresh, {} stale, {} failed, \
         {} other {}",
        status_header,
        total.style(styles.bold),
        plural::documents(total),
        num_fresh.style(styles.bold),
        num_stale.style(styles.bold),
        num_failed.style(styles.bold),
        num_general_problems.style(styles.bold),
        plural::problems(num_general_problems),
    );
    if num_failed > 0 {
        eprintln!(
            "{:>HEADER_WIDTH$} (fix failures, then run {} to update)",
            "",
            format!("{} generate", env.command).style(styles.bold)
        );
        Ok(CheckResult::Failures)
    } else if num_stale > 0 || num_general_problems > 0 {
        eprintln!(
            "{:>HEADER_WIDTH$} (run {} to update)",
            "",
            format!("{} generate", env.command).style(styles.bold)
        );
        Ok(CheckResult::NeedsUpdate)
    } else {
        Ok(CheckResult::Success)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CheckResult {
    Success,
    NeedsUpdate,
    Failures,
}

/// Summarize the "check" status of one supported API version
fn summarize_one(
    env: &ResolvedEnv,
    api: &ManagedApi,
    version: &semver::Version,
    resolution: &Resolution<'_>,
    styles: &Styles,
) {
    let problems: Vec<_> = resolution.problems().collect();
    if problems.is_empty() {
        // Success case: file is up-to-date.
        eprintln!(
            "{:>HEADER_WIDTH$} {}",
            FRESH.style(styles.success_header),
            display_api_spec_version(api, version, &styles, resolution),
        );
    } else {
        // There were one or more problems, some of which may be unfixable.
        eprintln!(
            "{:>HEADER_WIDTH$} {}",
            if resolution.has_errors() {
                FAILURE.style(styles.failure_header)
            } else {
                assert!(resolution.has_problems());
                STALE.style(styles.warning_header)
            },
            display_api_spec_version(api, version, &styles, resolution),
        );

        display_resolution_problems(env, problems, styles);
    }
}

/// Print a formatted list of Problems
pub fn display_resolution_problems<'a, T>(
    env: &ResolvedEnv,
    problems: T,
    styles: &Styles,
) where
    T: IntoIterator<Item = &'a Problem<'a>>,
{
    for p in problems.into_iter() {
        let subheader_width = HEADER_WIDTH + 4;
        let first_indent = format!(
            "{:>subheader_width$}: ",
            if p.is_fixable() {
                "problem".style(styles.warning_header)
            } else {
                "error".style(styles.failure_header)
            }
        );
        let more_indent = " ".repeat(subheader_width + 2);
        eprintln!(
            "{}",
            textwrap::fill(
                &InlineErrorChain::new(&p).to_string(),
                textwrap::Options::with_termwidth()
                    .initial_indent(&first_indent)
                    .subsequent_indent(&more_indent)
            )
        );

        let Some(fix) = p.fix() else {
            continue;
        };

        let first_indent = format!(
            "{:>subheader_width$}: ",
            "fix".style(styles.warning_header)
        );
        let fix_str = fix.to_string();
        let steps = fix_str.trim_end().split("\n");
        for s in steps {
            eprintln!(
                "{}",
                textwrap::fill(
                    &format!("will {}", s),
                    textwrap::Options::with_termwidth()
                        .initial_indent(&first_indent)
                        .subsequent_indent(&more_indent)
                )
            );
        }

        // When possible, print a useful diff of changes.
        let do_diff = match p {
            Problem::LockstepStale { found, generated } => {
                let diff = TextDiff::from_lines(
                    found.contents(),
                    generated.contents(),
                );
                let path1 =
                    env.openapi_abs_dir().join(found.spec_file_name().path());
                let path2 = env
                    .openapi_abs_dir()
                    .join(generated.spec_file_name().path());
                Some((diff, path1, path2))
            }
            Problem::ExtraFileStale {
                check_stale:
                    CheckStale::Modified { full_path, actual, expected },
                ..
            } => {
                let diff = TextDiff::from_lines(actual, expected);
                Some((diff, full_path.clone(), full_path.clone()))
            }
            Problem::LocalVersionStale { spec_files, generated }
                if spec_files.len() == 1 =>
            {
                let diff = TextDiff::from_lines(
                    spec_files[0].contents(),
                    generated.contents(),
                );
                let path1 = env
                    .openapi_abs_dir()
                    .join(spec_files[0].spec_file_name().path());
                let path2 = env
                    .openapi_abs_dir()
                    .join(generated.spec_file_name().path());
                Some((diff, path1, path2))
            }
            _ => None,
        };

        if let Some((diff, path1, path2)) = do_diff {
            let indent = " ".repeat(HEADER_WIDTH + 1);
            // We don't care about I/O errors here (just as we don't when using
            // eprintln! above).
            let _ = write_diff(
                &diff,
                &path1,
                &path2,
                styles,
                // Add an indent to align diff with the status message.
                &mut indent_write::io::IndentWriter::new(
                    &indent,
                    std::io::stderr(),
                ),
            );
            eprintln!("");
        }
    }
}

/// Adapter for [`Error`]s that provides a [`std::fmt::Display`] implementation
/// that print the full chain of error sources, separated by `: `.
pub struct InlineErrorChain<'a>(&'a dyn std::error::Error);

impl<'a> InlineErrorChain<'a> {
    pub fn new(error: &'a dyn std::error::Error) -> Self {
        Self(error)
    }
}

impl fmt::Display for InlineErrorChain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)?;
        let mut cause = self.0.source();
        while let Some(source) = cause {
            write!(f, ": {source}")?;
            cause = source.source();
        }
        Ok(())
    }
}

/// Output headers.
pub(crate) mod headers {
    // Same width as Cargo's output.
    pub(crate) const HEADER_WIDTH: usize = 12;

    pub(crate) static SEPARATOR: &str = "-------";

    pub(crate) static CHECKING: &str = "Checking";
    pub(crate) static GENERATING: &str = "Generating";

    pub(crate) static FRESH: &str = "Fresh";
    pub(crate) static STALE: &str = "Stale";

    pub(crate) static UNCHANGED: &str = "Unchanged";

    pub(crate) static SUCCESS: &str = "Success";
    pub(crate) static FAILURE: &str = "Failure";
    pub(crate) static WARNING: &str = "Warning";
}

pub(crate) mod plural {
    pub(crate) fn files(count: usize) -> &'static str {
        if count == 1 {
            "file"
        } else {
            "files"
        }
    }

    pub(crate) fn changes(count: usize) -> &'static str {
        if count == 1 {
            "change"
        } else {
            "changes"
        }
    }

    pub(crate) fn documents(count: usize) -> &'static str {
        if count == 1 {
            "document"
        } else {
            "documents"
        }
    }

    pub(crate) fn errors(count: usize) -> &'static str {
        if count == 1 {
            "error"
        } else {
            "errors"
        }
    }

    pub(crate) fn paths(count: usize) -> &'static str {
        if count == 1 {
            "path"
        } else {
            "paths"
        }
    }

    pub(crate) fn problems(count: usize) -> &'static str {
        if count == 1 {
            "problem"
        } else {
            "problems"
        }
    }

    pub(crate) fn schemas(count: usize) -> &'static str {
        if count == 1 {
            "schema"
        } else {
            "schemas"
        }
    }
}
