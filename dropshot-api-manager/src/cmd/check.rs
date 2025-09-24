// Copyright 2025 Oxide Computer Company

use crate::{
    apis::ManagedApis,
    environment::{BlessedSource, GeneratedSource, ResolvedEnv},
    output::{
        display_load_problems, display_resolution, headers::*, CheckResult,
        OutputOpts, Styles,
    },
    resolved::Resolved,
    FAILURE_EXIT_CODE, NEEDS_UPDATE_EXIT_CODE,
};
use std::process::ExitCode;

impl CheckResult {
    pub fn to_exit_code(self) -> ExitCode {
        match self {
            CheckResult::Success => ExitCode::SUCCESS,
            CheckResult::NeedsUpdate => NEEDS_UPDATE_EXIT_CODE.into(),
            CheckResult::Failures => FAILURE_EXIT_CODE.into(),
        }
    }
}

pub(crate) fn check_impl(
    apis: &ManagedApis,
    env: &ResolvedEnv,
    blessed_source: &BlessedSource,
    generated_source: &GeneratedSource,
    output: &OutputOpts,
) -> anyhow::Result<CheckResult> {
    let mut styles = Styles::default();
    if output.use_color(supports_color::Stream::Stderr) {
        styles.colorize();
    }

    eprintln!("{:>HEADER_WIDTH$}", SEPARATOR);

    let (generated, errors) = generated_source.load(apis, &styles)?;
    display_load_problems(&errors, &styles)?;

    let (local_files, errors) = env.local_source.load(apis, &styles)?;
    display_load_problems(&errors, &styles)?;

    let (blessed, errors) = blessed_source.load(apis, &styles)?;
    display_load_problems(&errors, &styles)?;

    let resolved = Resolved::new(env, apis, &blessed, &generated, &local_files);

    eprintln!("{:>HEADER_WIDTH$}", SEPARATOR);
    display_resolution(env, apis, &resolved, &styles)
}
