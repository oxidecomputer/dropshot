// Copyright 2025 Oxide Computer Company

use dropshot_api_manager_types::ApiIdent;

use crate::{
    apis::ManagedApis,
    environment::{
        BlessedSource, ErrorAccumulator, GeneratedSource, ResolvedEnv,
    },
    output::{OutputOpts, Styles},
    resolved::Resolved,
    spec_files_generic::{ApiFiles, AsRawFiles},
};
use std::collections::BTreeMap;

pub(crate) fn debug_impl(
    apis: &ManagedApis,
    env: &ResolvedEnv,
    blessed_source: &BlessedSource,
    generated_source: &GeneratedSource,
    output: &OutputOpts,
) -> anyhow::Result<()> {
    let mut styles = Styles::default();
    if output.use_color(supports_color::Stream::Stderr) {
        styles.colorize();
    }

    // Print information about local files.

    let (local_files, errors) = env.local_source.load(apis, &styles)?;
    dump_structure(&local_files, &errors);

    // Print information about what we found in Git.
    let (blessed, errors) = blessed_source.load(apis, &styles)?;
    dump_structure(&blessed, &errors);

    // Print information about generated files.
    let (generated, errors) = generated_source.load(apis, &styles)?;
    dump_structure(&generated, &errors);

    // Print result of resolving the differences.
    println!("Resolving specs");
    let resolved = Resolved::new(env, apis, &blessed, &generated, &local_files);
    for note in resolved.notes() {
        println!("NOTE: {}", note);
    }
    for problem in resolved.general_problems() {
        println!("PROBLEM: {}", problem);
    }

    for api in apis.iter_apis() {
        let ident = api.ident();
        println!("    API: {}", ident);

        if let Some(symlink_problem) = resolved.symlink_problem(ident) {
            println!("        PROBLEM: {}\n", symlink_problem);
            if let Some(fix) = symlink_problem.fix() {
                println!("        FIX: {}\n", fix);
            }
        }

        for version in api.iter_versions_semver() {
            let resolution = resolved
                .resolution_for_api_version(ident, version)
                .expect("must have a resolution for every managed API");
            print!("        version {}: {}: ", version, resolution.kind());

            let problems: Vec<_> = resolution.problems().collect();
            if problems.is_empty() {
                println!("OK");
            } else {
                println!("ERROR");
                for p in problems {
                    println!("    PROBLEM: {}\n", p);
                    if let Some(fix) = p.fix() {
                        println!("        FIX: {}", fix);
                    }
                }
            }
        }
    }

    Ok(())
}

fn dump_structure<T: AsRawFiles>(
    spec_files: &BTreeMap<ApiIdent, ApiFiles<T>>,
    error_accumulator: &ErrorAccumulator,
) {
    let warnings: Vec<_> = error_accumulator.iter_warnings().collect();
    println!("warnings: {}", warnings.len());
    for w in warnings {
        println!("    warn: {:#}", w);
    }

    let errors: Vec<_> = error_accumulator.iter_errors().collect();
    println!("errors: {}", errors.len());
    for e in errors {
        println!("    error: {:#}", e);
    }

    for (api_ident, info) in spec_files {
        println!("    API: {}", api_ident);
        println!(
            "        latest: {}",
            info.latest_link()
                .map(|s| s.to_string())
                .as_deref()
                .unwrap_or("none")
        );
        for (version, files) in info.versions() {
            println!("        version {}:", version);
            for api_spec in files.as_raw_files() {
                println!(
                    "            file {} (v{})",
                    api_spec.spec_file_name().path(),
                    api_spec.version()
                );
            }
        }
    }
}
