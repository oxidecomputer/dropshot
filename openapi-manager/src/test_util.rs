// Copyright 2025 Oxide Computer Company

use crate::{
    apis::ManagedApis,
    cmd::check::check_impl,
    environment::{Environment, GeneratedSource},
    output::{CheckResult, OutputOpts},
    BlessedSourceArgs, GeneratedSourceArgs,
};

/// Check that a set of APIs is up-to-date.
///
/// This is meant to be called within a test.
pub fn check_apis_up_to_date(
    env: &Environment,
    apis: &ManagedApis,
) -> Result<CheckResult, anyhow::Error> {
    // env.resolve(None) assumes that env.default_openapi_dir is where the
    // OpenAPI documents live and doesn't need a further override. (If a custom
    // directory is desired, it can always be passed in via `env`.)
    let env = env.resolve(None)?;

    let blessed_source =
        BlessedSourceArgs { blessed_from_git: None, blessed_from_dir: None }
            .to_blessed_source(&env)?;
    let generated_source = GeneratedSource::try_from(GeneratedSourceArgs {
        generated_from_dir: None,
    })?;
    let output = OutputOpts { color: clap::ColorChoice::Auto };

    check_impl(apis, &env, &blessed_source, &generated_source, &output)
}
