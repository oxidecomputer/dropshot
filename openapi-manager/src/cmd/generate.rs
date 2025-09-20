// Copyright 2025 Oxide Computer Company

use crate::{
    apis::ManagedApis,
    environment::{BlessedSource, GeneratedSource, ResolvedEnv},
    output::{
        display_api_spec_version, display_load_problems, display_resolution,
        display_resolution_problems,
        headers::{self, *},
        plural, CheckResult, OutputOpts, Styles,
    },
    resolved::{Problem, Resolved},
    FAILURE_EXIT_CODE,
};
use anyhow::{anyhow, bail, Result};
use owo_colors::OwoColorize;
use std::process::ExitCode;

#[derive(Clone, Copy, Debug)]
pub(crate) enum GenerateResult {
    Success,
    Failures,
}

impl GenerateResult {
    pub(crate) fn to_exit_code(self) -> ExitCode {
        match self {
            GenerateResult::Success => ExitCode::SUCCESS,
            GenerateResult::Failures => FAILURE_EXIT_CODE.into(),
        }
    }
}

pub(crate) fn generate_impl(
    apis: &ManagedApis,
    env: &ResolvedEnv,
    blessed_source: &BlessedSource,
    generated_source: &GeneratedSource,
    output: &OutputOpts,
) -> Result<GenerateResult> {
    let mut styles = Styles::default();
    if output.use_color(supports_color::Stream::Stderr) {
        styles.colorize();
    }

    let (generated, errors) = generated_source.load(&apis, &styles)?;
    display_load_problems(&errors, &styles)?;

    let (local_files, errors) = env.local_source.load(&apis, &styles)?;
    display_load_problems(&errors, &styles)?;

    let (blessed, errors) = blessed_source.load(&apis, &styles)?;
    display_load_problems(&errors, &styles)?;

    let resolved =
        Resolved::new(env, &apis, &blessed, &generated, &local_files);
    eprintln!("{:>HEADER_WIDTH$}", SEPARATOR);

    let total = resolved.nexpected_documents();
    eprintln!(
        "{:>HEADER_WIDTH$} {} OpenAPI {}...",
        "Updating".style(styles.success_header),
        total.style(styles.bold),
        plural::documents(total),
    );

    if resolved.has_unfixable_problems() {
        return match display_resolution(env, &apis, &resolved, &styles)? {
            CheckResult::Failures => Ok(GenerateResult::Failures),
            unexpected => {
                Err(anyhow!("unexpectedly got {unexpected:?} from summarize()"))
            }
        };
    }

    let mut num_updated = 0;
    let mut num_unchanged = 0;
    let mut num_errors = 0;

    // Apply fixes for problems with supported API versions.
    for api in apis.iter_apis() {
        let ident = api.ident();
        for version in api.iter_versions_semver() {
            // unwrap(): there should be a resolution for every managed API
            let resolution =
                resolved.resolution_for_api_version(ident, version).unwrap();
            assert!(
                !resolution.has_errors(),
                "found unfixable problems, but that should have been \
                 checked above"
            );

            let problems: Vec<_> = resolution.problems().collect();
            if problems.is_empty() {
                eprintln!(
                    "{:>HEADER_WIDTH$} {}",
                    UNCHANGED.style(styles.unchanged_header),
                    display_api_spec_version(api, version, &styles, resolution),
                );
                num_unchanged += 1;
            } else {
                eprintln!(
                    "{:>HEADER_WIDTH$} {}",
                    STALE.style(styles.warning_header),
                    display_api_spec_version(api, version, &styles, resolution),
                );

                fix_problems(
                    env,
                    problems,
                    &styles,
                    &mut num_updated,
                    &mut num_errors,
                );
            }
        }

        if let Some(symlink_problem) = resolved.symlink_problem(ident) {
            eprintln!(
                "{:>HEADER_WIDTH$} {} \"latest\" symlink",
                STALE.style(styles.warning_header),
                ident.style(styles.filename),
            );

            fix_problems(
                env,
                std::iter::once(symlink_problem),
                &styles,
                &mut num_updated,
                &mut num_errors,
            );
        } else if api.is_versioned() {
            eprintln!(
                "{:>HEADER_WIDTH$} {} \"latest\" symlink",
                UNCHANGED.style(styles.unchanged_header),
                ident.style(styles.filename),
            );
        }
    }

    // Fix problems not associated with any supported version, if any.
    let general_problems: Vec<_> = resolved.general_problems().collect();
    fix_problems(
        env,
        general_problems,
        &styles,
        &mut num_updated,
        &mut num_errors,
    );

    if num_errors > 0 {
        print_final_status(
            &styles,
            total,
            num_updated,
            num_unchanged,
            num_errors,
        );
        return Ok(GenerateResult::Failures);
    }

    // Finally, check again for any problems.  Since we expect this should have
    // fixed everything, be quiet unless we find something amiss.
    let mut nproblems = 0;
    let (local_files, errors) = env.local_source.load(&apis, &styles)?;
    eprintln!(
        "{:>HEADER_WIDTH$} all local files",
        "Rechecking".style(styles.success_header),
    );
    display_load_problems(&errors, &styles)?;
    let resolved =
        Resolved::new(env, &apis, &blessed, &generated, &local_files);
    let general_problems: Vec<_> = resolved.general_problems().collect();
    nproblems += general_problems.len();
    if !general_problems.is_empty() {
        display_resolution_problems(env, general_problems, &styles);
    }
    for api in apis.iter_apis() {
        let ident = api.ident();
        for version in api.iter_versions_semver() {
            // unwrap(): there should be a resolution for every managed API
            let resolution =
                resolved.resolution_for_api_version(ident, version).unwrap();
            let problems: Vec<_> = resolution.problems().collect();
            nproblems += problems.len();
            if !problems.is_empty() {
                eprintln!(
                    "found unexpected problem with API {} version {} \
                     (this is a bug)",
                    ident, version
                );
                display_resolution_problems(env, problems, &styles);
            }
        }

        if let Some(symlink_problem) = resolved.symlink_problem(ident) {
            nproblems += 1;
            eprintln!(
                "found unexpected problem with API {} symlink (this is a bug)",
                ident
            );
            display_resolution_problems(
                env,
                std::iter::once(symlink_problem),
                &styles,
            );
        }
    }

    if nproblems > 0 {
        bail!(
            "ERROR: found problems after successfully fixing everything \
             (this is a BUG!)"
        );
    } else {
        print_final_status(
            &styles,
            total,
            num_updated,
            num_unchanged,
            num_errors,
        );
        Ok(GenerateResult::Success)
    }
}

fn print_final_status(
    styles: &Styles,
    ndocuments: usize,
    num_updated: usize,
    num_unchanged: usize,
    num_errors: usize,
) {
    eprintln!("{:>HEADER_WIDTH$}", SEPARATOR);
    let status_header = if num_errors == 0 {
        headers::SUCCESS.style(styles.success_header)
    } else {
        headers::FAILURE.style(styles.failure_header)
    };
    eprintln!(
        "{:>HEADER_WIDTH$} {} {}: {} {} made, {} unchanged, {} failed",
        status_header,
        ndocuments.style(styles.bold),
        plural::documents(ndocuments),
        num_updated.style(styles.bold),
        plural::changes(num_updated),
        num_unchanged.style(styles.bold),
        num_errors.style(styles.bold),
    );
}

fn fix_problems<'a, T>(
    env: &ResolvedEnv,
    problems: T,
    styles: &Styles,
    num_updated: &mut usize,
    num_errors: &mut usize,
) where
    T: IntoIterator<Item = &'a Problem<'a>>,
{
    for p in problems {
        // We should have already bailed out if there were any unfixable
        // problems.
        let fix = p.fix().expect("attempting to fix unfixable problem");
        match fix.execute(env) {
            Ok(steps) => {
                *num_updated += 1;
                for s in steps {
                    eprintln!(
                        "{:>HEADER_WIDTH$} {}",
                        "Fixed".style(styles.success_header),
                        s,
                    );
                }
            }
            Err(error) => {
                *num_errors += 1;
                eprintln!(
                    "{:>HEADER_WIDTH$} fix {:?}: {:#}",
                    "FIX FAILED".style(styles.failure_header),
                    fix.to_string(),
                    error
                );
            }
        }
    }
}
