// Copyright 2025 Oxide Computer Company

//! Binary for the OpenAPI manager examples.

use std::process::ExitCode;

use anyhow::{Context, anyhow};
use camino::Utf8PathBuf;
use clap::Parser;
use dropshot_api_manager::{Environment, ManagedApiConfig, ManagedApis};
use dropshot_api_manager_example_apis::*;
use dropshot_api_manager_types::{ApiBoundary, ValidationContext, Versions};
use openapiv3::OpenAPI;

pub fn environment() -> anyhow::Result<Environment> {
    // The workspace root is two levels up from this crate's directory.
    let workspace_root = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let env = Environment::new(
        // This is the command used to run the OpenAPI manager.
        "cargo example-openapi".to_owned(),
        workspace_root,
        // This is the location within the workspace root where the OpenAPI
        // documents are stored.
        "dropshot-api-manager-example/documents".into(),
    )?;
    Ok(env)
}

/// The list of APIs managed by the OpenAPI manager.
pub fn all_apis() -> anyhow::Result<ManagedApis> {
    let apis = vec![
        // This API is managed in a simple, lockstep fashion.
        ManagedApiConfig {
            ident: "lockstep",
            versions: Versions::Lockstep { version: "1.0.0".parse().unwrap() },
            title: "Lockstep API",
            description: "A simple lockstep-versioned API",
            boundary: ApiBoundary::Internal,
            api_description: lockstep::lockstep_api_mod::stub_api_description,
            extra_validation: None,
        },
        // This API is versioned.
        ManagedApiConfig {
            ident: "versioned",
            versions: Versions::Versioned {
                // The `api_versions!` macro is used to define supported
                // versions.
                supported_versions: versioned::supported_versions(),
            },
            title: "Versioned API",
            description: "A versioned API",
            boundary: ApiBoundary::External,
            api_description: versioned::versioned_api_mod::stub_api_description,
            extra_validation: None,
        },
    ];

    let apis = ManagedApis::new(apis)
        .context("error creating ManagedApis")?
        // A global validation function can be provided to the OpenAPI manager.
        // This function will be called for each API under consideration.
        .with_validation(validate);
    Ok(apis)
}

fn validate(spec: &OpenAPI, mut cx: ValidationContext<'_>) {
    // Here, we use Oxide's openapi-lint crate to perform some linting on the
    // OpenAPI document. This kind of validation is optional.
    let errors = match cx.boundary() {
        ApiBoundary::Internal => openapi_lint::validate(spec),
        ApiBoundary::External => openapi_lint::validate_external(spec),
    };
    for error in errors {
        cx.report_error(anyhow!(error));
    }
}

fn main() -> anyhow::Result<ExitCode> {
    let app = dropshot_api_manager::App::parse();
    let env = environment()?;
    let apis = all_apis()?;

    Ok(app.exec(&env, &apis))
}

#[cfg(test)]
mod tests {
    use dropshot_api_manager::test_util::check_apis_up_to_date;

    use super::*;

    // Also recommended: a test which ensures documents are up-to-date. The
    // OpenAPI manager comes with a helper function for this, called
    // `check_apis_up_to_date`.
    #[test]
    fn test_apis_up_to_date() -> anyhow::Result<ExitCode> {
        let env = environment()?;
        let apis = all_apis()?;

        let result = check_apis_up_to_date(&env, &apis)?;
        Ok(result.to_exit_code())
    }
}
