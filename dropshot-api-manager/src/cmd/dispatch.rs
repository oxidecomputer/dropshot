// Copyright 2025 Oxide Computer Company

use crate::{
    apis::ManagedApis,
    cmd::{
        check::check_impl, debug::debug_impl, generate::generate_impl,
        list::list_impl,
    },
    environment::{BlessedSource, Environment, GeneratedSource, ResolvedEnv},
    git::GitRevision,
    output::OutputOpts,
};
use anyhow::Result;
use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand};
use std::process::ExitCode;

/// Manage OpenAPI documents for this repository.
///
/// For more information, see <https://crates.io/crates/dropshot-api-manager>.
#[derive(Debug, Parser)]
pub struct App {
    #[clap(flatten)]
    output_opts: OutputOpts,

    #[clap(subcommand)]
    command: Command,
}

impl App {
    pub fn exec(self, env: &Environment, apis: &ManagedApis) -> ExitCode {
        let result = match self.command {
            Command::Debug(args) => args.exec(env, apis, &self.output_opts),
            Command::List(args) => args.exec(apis, &self.output_opts),
            Command::Generate(args) => args.exec(env, apis, &self.output_opts),
            Command::Check(args) => args.exec(env, apis, &self.output_opts),
        };

        match result {
            Ok(exit_code) => exit_code,
            Err(error) => {
                eprintln!("failure: {:#}", error);
                ExitCode::FAILURE
            }
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Dump debug information about everything the tool knows
    Debug(DebugArgs),

    /// List managed APIs.
    ///
    /// Returns information purely from code without consulting JSON files on
    /// disk. To compare against files on disk, use the `check` command.
    List(ListArgs),

    /// Generate latest OpenAPI documents and validate the results.
    Generate(GenerateArgs),

    /// Check that OpenAPI documents are up-to-date and valid.
    Check(CheckArgs),
}

#[derive(Debug, Args)]
pub struct BlessedSourceArgs {
    /// Loads blessed OpenAPI documents from path PATH in the given Git
    /// REVISION.
    ///
    /// The REVISION is not used as-is; instead, the tool always looks at the
    /// merge-base between HEAD and REVISION. So if you provide `main:openapi`,
    /// then it will look at the merge-base of HEAD and `main`, in directory
    /// "openapi" in that commit.
    ///
    /// REVISION is optional and defaults to the `default_git_branch` provided
    /// by the OpenAPI manager binary (typically `origin/main`).
    ///
    /// PATH is optional and defaults to `default_openapi_dir` provided by the
    /// OpenAPI manager binary.
    #[clap(
        long,
        env("OPENAPI_MGR_BLESSED_FROM_GIT"),
        value_name("REVISION[:PATH]")
    )]
    pub blessed_from_git: Option<String>,

    /// Loads blessed OpenAPI documents from a local directory (instead of the
    /// default, from Git).
    ///
    /// This is intended for testing and debugging this tool.
    #[clap(
        long,
        conflicts_with("blessed_from_git"),
        env("OPENAPI_MGR_BLESSED_FROM_DIR"),
        value_name("DIRECTORY")
    )]
    pub blessed_from_dir: Option<Utf8PathBuf>,
}

impl BlessedSourceArgs {
    pub(crate) fn to_blessed_source(
        &self,
        env: &ResolvedEnv,
    ) -> Result<BlessedSource, anyhow::Error> {
        assert!(
            self.blessed_from_dir.is_none() || self.blessed_from_git.is_none()
        );

        if let Some(local_directory) = &self.blessed_from_dir {
            return Ok(BlessedSource::Directory {
                local_directory: local_directory.clone(),
            });
        }

        let (revision_str, maybe_directory) = match &self.blessed_from_git {
            None => (env.default_git_branch.as_str(), None),
            Some(arg) => match arg.split_once(":") {
                Some((r, d)) => (r, Some(d)),
                None => (arg.as_str(), None),
            },
        };
        let revision = GitRevision::from(String::from(revision_str));
        let directory = Utf8PathBuf::from(maybe_directory.map_or(
            // We must use the relative directory path for Git commands.
            env.openapi_rel_dir(),
            |d| d.as_ref(),
        ));
        Ok(BlessedSource::GitRevisionMergeBase { revision, directory })
    }
}

#[derive(Debug, Args)]
pub struct GeneratedSourceArgs {
    /// Instead of generating OpenAPI documents directly from the API
    /// implementation, load OpenAPI documents from this directory.
    #[clap(long, value_name("DIRECTORY"))]
    pub generated_from_dir: Option<Utf8PathBuf>,
}

impl From<GeneratedSourceArgs> for GeneratedSource {
    fn from(value: GeneratedSourceArgs) -> Self {
        match value.generated_from_dir {
            Some(local_directory) => {
                GeneratedSource::Directory { local_directory }
            }
            None => GeneratedSource::Generated,
        }
    }
}

#[derive(Debug, Args)]
pub struct LocalSourceArgs {
    /// Loads this workspace's OpenAPI documents from local path DIRECTORY.
    #[clap(long, env("OPENAPI_MGR_DIR"), value_name("DIRECTORY"))]
    dir: Option<Utf8PathBuf>,
}

#[derive(Debug, Args)]
pub struct DebugArgs {
    #[clap(flatten)]
    local: LocalSourceArgs,
    #[clap(flatten)]
    blessed: BlessedSourceArgs,
    #[clap(flatten)]
    generated: GeneratedSourceArgs,
}

impl DebugArgs {
    fn exec(
        self,
        env: &Environment,
        apis: &ManagedApis,
        output: &OutputOpts,
    ) -> anyhow::Result<ExitCode> {
        let env = env.resolve(self.local.dir)?;
        let blessed_source = self.blessed.to_blessed_source(&env)?;
        let generated_source = GeneratedSource::from(self.generated);
        debug_impl(apis, &env, &blessed_source, &generated_source, output)?;
        Ok(ExitCode::SUCCESS)
    }
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Show verbose output including descriptions.
    #[clap(long, short)]
    verbose: bool,
}

impl ListArgs {
    fn exec(
        self,
        apis: &ManagedApis,
        output: &OutputOpts,
    ) -> anyhow::Result<ExitCode> {
        list_impl(apis, self.verbose, output)?;
        Ok(ExitCode::SUCCESS)
    }
}

#[derive(Debug, Args)]
pub struct GenerateArgs {
    #[clap(flatten)]
    local: LocalSourceArgs,
    #[clap(flatten)]
    blessed: BlessedSourceArgs,
    #[clap(flatten)]
    generated: GeneratedSourceArgs,
}

impl GenerateArgs {
    fn exec(
        self,
        env: &Environment,
        apis: &ManagedApis,
        output: &OutputOpts,
    ) -> anyhow::Result<ExitCode> {
        let env = env.resolve(self.local.dir)?;
        let blessed_source = self.blessed.to_blessed_source(&env)?;
        let generated_source = GeneratedSource::from(self.generated);
        Ok(generate_impl(
            apis,
            &env,
            &blessed_source,
            &generated_source,
            output,
        )?
        .to_exit_code())
    }
}

#[derive(Debug, Args)]
pub struct CheckArgs {
    #[clap(flatten)]
    local: LocalSourceArgs,
    #[clap(flatten)]
    blessed: BlessedSourceArgs,
    #[clap(flatten)]
    generated: GeneratedSourceArgs,
}

impl CheckArgs {
    fn exec(
        self,
        env: &Environment,
        apis: &ManagedApis,
        output: &OutputOpts,
    ) -> anyhow::Result<ExitCode> {
        let env = env.resolve(self.local.dir)?;
        let blessed_source = self.blessed.to_blessed_source(&env)?;
        let generated_source = GeneratedSource::from(self.generated);
        Ok(check_impl(apis, &env, &blessed_source, &generated_source, output)?
            .to_exit_code())
    }
}

// This code is not 0 or 1 (general anyhow errors) and indicates out-of-date.
pub(crate) const NEEDS_UPDATE_EXIT_CODE: u8 = 4;

// This code indicates failures during generation, e.g. validation errors.
pub(crate) const FAILURE_EXIT_CODE: u8 = 100;

#[cfg(test)]
mod test {
    use super::{
        App, BlessedSourceArgs, CheckArgs, Command, GeneratedSourceArgs,
        LocalSourceArgs,
    };
    use crate::environment::{BlessedSource, Environment, GeneratedSource};
    use assert_matches::assert_matches;
    use camino::{Utf8Path, Utf8PathBuf};
    use clap::Parser;

    #[test]
    fn test_arg_parsing() {
        // Default case
        let app = App::parse_from(&["dummy", "check"]);
        assert_matches!(
            app.command,
            Command::Check(CheckArgs {
                local: LocalSourceArgs { dir: None },
                blessed: BlessedSourceArgs {
                    blessed_from_git: None,
                    blessed_from_dir: None
                },
                generated: GeneratedSourceArgs { generated_from_dir: None },
            })
        );

        // Override local dir
        let app = App::parse_from(&["dummy", "check", "--dir", "foo"]);
        assert_matches!(app.command, Command::Check(CheckArgs {
            local: LocalSourceArgs { dir: Some(local_dir) },
            blessed:
                BlessedSourceArgs { blessed_from_git: None, blessed_from_dir: None },
            generated: GeneratedSourceArgs { generated_from_dir: None },
        }) if local_dir == "foo");

        // Override generated dir differently
        let app = App::parse_from(&[
            "dummy",
            "check",
            "--dir",
            "foo",
            "--generated-from-dir",
            "bar",
        ]);
        assert_matches!(app.command, Command::Check(CheckArgs {
            local: LocalSourceArgs { dir: Some(local_dir) },
            blessed:
                BlessedSourceArgs { blessed_from_git: None, blessed_from_dir: None },
            generated: GeneratedSourceArgs { generated_from_dir: Some(generated_dir) },
        }) if local_dir == "foo" && generated_dir == "bar");

        // Override blessed with a local directory.
        let app = App::parse_from(&[
            "dummy",
            "check",
            "--dir",
            "foo",
            "--generated-from-dir",
            "bar",
            "--blessed-from-dir",
            "baz",
        ]);
        assert_matches!(app.command, Command::Check(CheckArgs {
            local: LocalSourceArgs { dir: Some(local_dir) },
            blessed:
                BlessedSourceArgs { blessed_from_git: None, blessed_from_dir: Some(blessed_dir) },
            generated: GeneratedSourceArgs { generated_from_dir: Some(generated_dir) },
        }) if local_dir == "foo" && generated_dir == "bar" && blessed_dir == "baz");

        // Override blessed from Git.
        let app = App::parse_from(&[
            "dummy",
            "check",
            "--blessed-from-git",
            "some/other/upstream",
        ]);
        assert_matches!(app.command, Command::Check(CheckArgs {
            local: LocalSourceArgs { dir: None },
            blessed:
                BlessedSourceArgs { blessed_from_git: Some(git), blessed_from_dir: None },
            generated: GeneratedSourceArgs { generated_from_dir: None },
        }) if git == "some/other/upstream");

        // Error case: specifying both --blessed-from-git and --blessed-from-dir
        let error = App::try_parse_from(&[
            "dummy",
            "check",
            "--blessed-from-git",
            "git_revision",
            "--blessed-from-dir",
            "dir",
        ])
        .unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
        assert!(error.to_string().contains(
            "error: the argument '--blessed-from-git <REVISION[:PATH]>' cannot \
             be used with '--blessed-from-dir <DIRECTORY>"
        ));
    }

    // Test how we turn `LocalSourceArgs` into `Environment`.
    #[test]
    fn test_local_args() {
        #[cfg(unix)]
        const ABS_DIR: &str = "/tmp";
        #[cfg(windows)]
        const ABS_DIR: &str = "C:\\tmp";

        {
            let env = Environment::new(
                "cargo openapi".to_owned(),
                Utf8PathBuf::from(ABS_DIR),
                Utf8PathBuf::from("foo"),
            )
            .expect("loading environment");
            let env = env.resolve(None).expect("resolving environment");
            assert_eq!(
                env.openapi_abs_dir(),
                Utf8Path::new(ABS_DIR).join("foo")
            );
        }

        {
            let error = Environment::new(
                "cargo openapi".to_owned(),
                Utf8PathBuf::from(ABS_DIR),
                Utf8PathBuf::from(ABS_DIR),
            )
            .unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "default_openapi_dir must be a relative path with \
                     normal components, found: {}",
                    ABS_DIR
                )
            );
        }

        {
            let current_dir =
                Utf8PathBuf::try_from(std::env::current_dir().unwrap())
                    .unwrap();
            let env = Environment::new(
                "cargo openapi".to_owned(),
                current_dir.clone(),
                Utf8PathBuf::from("foo"),
            )
            .expect("loading environment");
            let env = env
                .resolve(Some(Utf8PathBuf::from("bar")))
                .expect("resolving environment");
            assert_eq!(env.openapi_abs_dir(), current_dir.join("bar"));
        }
    }

    // Test how we convert `GeneratedSourceArgs` into `GeneratedSource`.
    #[test]
    fn test_generated_args() {
        let source = GeneratedSource::try_from(GeneratedSourceArgs {
            generated_from_dir: None,
        })
        .unwrap();
        assert_matches!(source, GeneratedSource::Generated);

        let source = GeneratedSource::try_from(GeneratedSourceArgs {
            generated_from_dir: Some(Utf8PathBuf::from("/tmp")),
        })
        .unwrap();
        assert_matches!(
            source,
            GeneratedSource::Directory { local_directory }
                if local_directory == "/tmp"
        );
    }

    // Test how we convert `BlessedSourceArgs` into `BlessedSource`.
    #[test]
    fn test_blessed_args() {
        let env = Environment::new(
            "cargo openapi".to_owned(),
            "/test".into(),
            "foo-openapi".into(),
        )
        .unwrap()
        .with_default_git_branch("upstream/dev".to_owned());
        let env = env.resolve(None).unwrap();

        let source = BlessedSourceArgs {
            blessed_from_git: None,
            blessed_from_dir: None,
        }
        .to_blessed_source(&env)
        .unwrap();
        assert_matches!(
            source,
            BlessedSource::GitRevisionMergeBase { revision, directory }
                if *revision == "upstream/dev" && directory == "foo-openapi"
        );

        // Override branch only
        let source = BlessedSourceArgs {
            blessed_from_git: Some(String::from("my/other/main")),
            blessed_from_dir: None,
        }
        .to_blessed_source(&env)
        .unwrap();
        assert_matches!(
            source,
            BlessedSource::GitRevisionMergeBase { revision, directory}
                if *revision == "my/other/main" && directory == "foo-openapi"
        );

        // Override branch and directory
        let source = BlessedSourceArgs {
            blessed_from_git: Some(String::from(
                "my/other/main:other_openapi/bar",
            )),
            blessed_from_dir: None,
        }
        .to_blessed_source(&env)
        .unwrap();
        assert_matches!(
            source,
            BlessedSource::GitRevisionMergeBase { revision, directory}
                if *revision == "my/other/main" &&
                     directory == "other_openapi/bar"
        );

        // Override with a local directory
        let source = BlessedSourceArgs {
            blessed_from_git: None,
            blessed_from_dir: Some(Utf8PathBuf::from("/tmp")),
        }
        .to_blessed_source(&env)
        .unwrap();
        assert_matches!(
            source,
            BlessedSource::Directory { local_directory }
                if local_directory == "/tmp"
        );
    }
}
