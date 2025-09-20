// Copyright 2025 Oxide Computer Company

//! Describes the environment the command is running in, and particularly where
//! different sets of specifications are loaded from

use crate::apis::ManagedApis;
use crate::git::GitRevision;
use crate::output::headers::GENERATING;
use crate::output::headers::HEADER_WIDTH;
use crate::output::Styles;
use crate::spec_files_blessed::BlessedFiles;
use crate::spec_files_generated::GeneratedFiles;
use crate::spec_files_local::walk_local_directory;
use crate::spec_files_local::LocalFiles;
use anyhow::Context;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use owo_colors::OwoColorize;

#[derive(Clone, Debug)]
pub struct Environment {
    /// The command to run the OpenAPI manager.
    pub(crate) command: String,

    /// Path to the root of this repository
    pub(crate) repo_root: Utf8PathBuf,

    /// The default OpenAPI directory.
    pub(crate) default_openapi_dir: Utf8PathBuf,

    /// The default Git upstream.
    pub(crate) default_git_branch: String,
}

impl Environment {
    /// Creates a new environment with:
    ///
    /// * the command to invoke the OpenAPI manager (e.g. `"cargo openapi"`
    ///   or `"cargo xtask openapi"`)
    /// * the provided Git repository root
    /// * the default OpenAPI directory within the repository root
    ///
    /// Returns an error if `repo_root` is not an absolute path.
    pub fn new(
        command: String,
        repo_root: Utf8PathBuf,
        default_openapi_dir: Utf8PathBuf,
    ) -> anyhow::Result<Self> {
        if !repo_root.is_absolute() {
            return Err(anyhow::anyhow!(
                "repo_root must be an absolute path, found: {}",
                repo_root
            ));
        }

        Ok(Self {
            repo_root,
            default_openapi_dir,
            default_git_branch: "origin/main".to_owned(),
            command,
        })
    }

    /// Sets the default Git upstream and branch name.
    ///
    /// By default, this is `origin/main`, but it can be set to any valid Git
    /// remote and branch name separated by a forward slash, e.g.
    /// `origin/master` or `upstream/dev`.
    ///
    /// For individual commands, this can be overridden through the
    /// `--blessed-from-git` argument, or the `OPENAPI_MGR_BLESSED_FROM_GIT`
    /// environment variable.
    pub fn with_default_git_branch(mut self, branch: String) -> Self {
        self.default_git_branch = branch;
        self
    }

    pub(crate) fn resolve(
        &self,
        openapi_dir: Option<Utf8PathBuf>,
    ) -> anyhow::Result<ResolvedEnv> {
        // Use the provided `openapi_dir` joined to the *current* directory
        // if it exists, otherwise the default directory joined to the
        // *workspace root*.
        let openapi_dir = if let Some(openapi_dir) = openapi_dir {
            if openapi_dir.is_absolute() {
                openapi_dir
            } else {
                let current_dir = std::env::current_dir()
                    .context("error obtaining current directory")?;
                let current_dir = Utf8PathBuf::try_from(current_dir)
                    .context("current directory is not valid UTF-8")?;
                current_dir.join(openapi_dir)
            }
        } else {
            self.repo_root.join(&self.default_openapi_dir)
        };

        Ok(ResolvedEnv {
            command: self.command.clone(),
            repo_root: self.repo_root.clone(),
            local_source: LocalSource::Directory {
                local_directory: openapi_dir,
            },
            default_git_branch: self.default_git_branch.clone(),
        })
    }
}

/// Internal type for the environment where the OpenAPI directory is known.
#[derive(Debug)]
pub(crate) struct ResolvedEnv {
    pub(crate) command: String,
    pub(crate) repo_root: Utf8PathBuf,
    pub(crate) local_source: LocalSource,
    pub(crate) default_git_branch: String,
}

impl ResolvedEnv {
    pub(crate) fn openapi_dir(&self) -> &Utf8Path {
        match &self.local_source {
            LocalSource::Directory { local_directory } => local_directory,
        }
    }
}

/// Specifies where to find blessed OpenAPI documents (the ones that are
/// considered immutable because they've been committed-to upstream)
#[derive(Debug)]
pub enum BlessedSource {
    /// Blessed OpenAPI documents come from the Git merge base between `HEAD`
    /// and the specified revision (default "main"), in the specified directory.
    GitRevisionMergeBase { revision: GitRevision, directory: Utf8PathBuf },

    /// Blessed OpenAPI documents come from this directory
    ///
    /// This is basically just for testing and debugging this tool.
    Directory { local_directory: Utf8PathBuf },
}

impl BlessedSource {
    /// Load the blessed OpenAPI documents
    pub fn load(
        &self,
        apis: &ManagedApis,
        styles: &Styles,
    ) -> anyhow::Result<(BlessedFiles, ErrorAccumulator)> {
        let mut errors = ErrorAccumulator::new();
        match self {
            BlessedSource::Directory { local_directory } => {
                eprintln!(
                    "{:>HEADER_WIDTH$} blessed OpenAPI documents from {:?}",
                    "Loading".style(styles.success_header),
                    local_directory,
                );
                let api_files =
                    walk_local_directory(local_directory, apis, &mut errors)?;
                Ok((BlessedFiles::from(api_files), errors))
            }
            BlessedSource::GitRevisionMergeBase { revision, directory } => {
                eprintln!(
                    "{:>HEADER_WIDTH$} blessed OpenAPI documents from git \
                     revision {:?} path {:?}",
                    "Loading".style(styles.success_header),
                    revision,
                    directory
                );
                Ok((
                    BlessedFiles::load_from_git_parent_branch(
                        &revision,
                        &directory,
                        apis,
                        &mut errors,
                    )?,
                    errors,
                ))
            }
        }
    }
}

/// Specifies how to find generated OpenAPI documents
#[derive(Debug)]
pub enum GeneratedSource {
    /// Generate OpenAPI documents from the API implementation (default)
    Generated,

    /// Load "generated" OpenAPI documents from the specified directory
    ///
    /// This is basically just for testing and debugging this tool.
    Directory { local_directory: Utf8PathBuf },
}

impl GeneratedSource {
    /// Load the generated OpenAPI documents (i.e., generating them as needed)
    pub fn load(
        &self,
        apis: &ManagedApis,
        styles: &Styles,
    ) -> anyhow::Result<(GeneratedFiles, ErrorAccumulator)> {
        let mut errors = ErrorAccumulator::new();
        match self {
            GeneratedSource::Generated => {
                eprintln!(
                    "{:>HEADER_WIDTH$} OpenAPI documents from API \
                     definitions ... ",
                    GENERATING.style(styles.success_header)
                );
                Ok((GeneratedFiles::generate(apis, &mut errors)?, errors))
            }
            GeneratedSource::Directory { local_directory } => {
                eprintln!(
                    "{:>HEADER_WIDTH$} \"generated\" OpenAPI documents from \
                     {:?} ... ",
                    "Loading".style(styles.success_header),
                    local_directory,
                );
                let api_files =
                    walk_local_directory(local_directory, apis, &mut errors)?;
                Ok((GeneratedFiles::from(api_files), errors))
            }
        }
    }
}

/// Specifies where to find local OpenAPI documents
#[derive(Debug)]
pub enum LocalSource {
    /// Local OpenAPI documents come from this directory
    Directory { local_directory: Utf8PathBuf },
}

impl LocalSource {
    /// Load the local OpenAPI documents
    pub fn load(
        &self,
        apis: &ManagedApis,
        styles: &Styles,
    ) -> anyhow::Result<(LocalFiles, ErrorAccumulator)> {
        let mut errors = ErrorAccumulator::new();
        match self {
            LocalSource::Directory { local_directory } => {
                eprintln!(
                    "{:>HEADER_WIDTH$} local OpenAPI documents from \
                     {:?} ... ",
                    "Loading".style(styles.success_header),
                    local_directory,
                );
                Ok((
                    LocalFiles::load_from_directory(
                        local_directory,
                        apis,
                        &mut errors,
                    )?,
                    errors,
                ))
            }
        }
    }
}

/// Stores errors and warnings accumulated during loading
pub struct ErrorAccumulator {
    /// errors that reflect incorrectness or incompleteness of the loaded data
    errors: Vec<anyhow::Error>,
    /// problems that do not affect the correctness or completeness of the data
    warnings: Vec<anyhow::Error>,
}

impl ErrorAccumulator {
    pub fn new() -> ErrorAccumulator {
        ErrorAccumulator { errors: Vec::new(), warnings: Vec::new() }
    }

    /// Record an error
    pub fn error(&mut self, error: anyhow::Error) {
        self.errors.push(error);
    }

    /// Record a warning
    pub fn warning(&mut self, error: anyhow::Error) {
        self.warnings.push(error);
    }

    pub fn iter_errors(&self) -> impl Iterator<Item = &'_ anyhow::Error> + '_ {
        self.errors.iter()
    }

    pub fn iter_warnings(
        &self,
    ) -> impl Iterator<Item = &'_ anyhow::Error> + '_ {
        self.warnings.iter()
    }
}
