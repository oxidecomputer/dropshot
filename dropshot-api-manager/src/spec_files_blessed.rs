// Copyright 2025 Oxide Computer Company

//! Newtype and collection to represent OpenAPI documents from the "blessed"
//! source

use crate::{
    apis::ManagedApis,
    environment::ErrorAccumulator,
    git::{git_ls_tree, git_merge_base_head, git_show_file, GitRevision},
    spec_files_generic::{
        ApiFiles, ApiLoad, ApiSpecFile, ApiSpecFilesBuilder, AsRawFiles,
    },
};
use anyhow::{anyhow, bail};
use camino::Utf8Path;
use dropshot_api_manager_types::ApiIdent;
use std::{collections::BTreeMap, ops::Deref};

/// Newtype wrapper around [`ApiSpecFile`] to describe OpenAPI documents from
/// the "blessed" source.
///
/// The blessed source contains the documents that are not allowed to be changed
/// locally because they've been committed-to upstream.
///
/// Note that this type can represent documents for both lockstep APIs and
/// versioned APIs, but it's meaningless for lockstep APIs.  Any documents for
/// versioned APIs are blessed by definition.
pub struct BlessedApiSpecFile(ApiSpecFile);
NewtypeDebug! { () pub struct BlessedApiSpecFile(ApiSpecFile); }
NewtypeDeref! { () pub struct BlessedApiSpecFile(ApiSpecFile); }
NewtypeDerefMut! { () pub struct BlessedApiSpecFile(ApiSpecFile); }
NewtypeFrom! { () pub struct BlessedApiSpecFile(ApiSpecFile); }

// Trait impls that allow us to use `ApiFiles<BlessedApiSpecFile>`
//
// Note that this is NOT a `Vec` because it's NOT allowed to have more than one
// BlessedApiSpecFile for a given version.

impl ApiLoad for BlessedApiSpecFile {
    const MISCONFIGURATIONS_ALLOWED: bool = true;

    fn make_item(raw: ApiSpecFile) -> Self {
        BlessedApiSpecFile(raw)
    }

    fn try_extend(&mut self, item: ApiSpecFile) -> anyhow::Result<()> {
        // This should be impossible.
        bail!(
            "found more than one blessed OpenAPI document for a given \
             API version: at least {} and {}",
            self.spec_file_name(),
            item.spec_file_name()
        );
    }
}

impl AsRawFiles for BlessedApiSpecFile {
    fn as_raw_files<'a>(
        &'a self,
    ) -> Box<dyn Iterator<Item = &'a ApiSpecFile> + 'a> {
        Box::new(std::iter::once(self.deref()))
    }
}

/// Container for OpenAPI documents from the "blessed" source (usually Git)
///
/// **Be sure to check for load errors and warnings before using this
/// structure.**
///
/// For more on what's been validated at this point, see
/// [`ApiSpecFilesBuilder`].
#[derive(Debug)]
pub struct BlessedFiles(BTreeMap<ApiIdent, ApiFiles<BlessedApiSpecFile>>);

NewtypeDeref! {
    () pub struct BlessedFiles(
        BTreeMap<ApiIdent, ApiFiles<BlessedApiSpecFile>>
    );
}

impl BlessedFiles {
    /// Load OpenAPI documents from the given directory in the merge base
    /// between HEAD and the given branch.
    ///
    /// This is usually what users want.  For example, if these is the Git
    /// repository history:
    ///
    /// ```text
    /// main:  M1 -> M2 -> M3 -> M4
    ///         \
    /// branch:  +-- B1 --> B2
    /// ```
    ///
    /// and you're on `B2`, `main` refers to `M4`, but you want to be looking at
    /// `M1` for blessed documents because you haven't yet merged in commits M2,
    /// M3, and M4.
    pub fn load_from_git_parent_branch(
        branch: &GitRevision,
        directory: &Utf8Path,
        apis: &ManagedApis,
        error_accumulator: &mut ErrorAccumulator,
    ) -> anyhow::Result<BlessedFiles> {
        let revision = git_merge_base_head(branch)?;
        Self::load_from_git_revision(
            &revision,
            directory,
            apis,
            error_accumulator,
        )
    }

    /// Load OpenAPI documents from the given Git revision and directory.
    pub fn load_from_git_revision(
        commit: &GitRevision,
        directory: &Utf8Path,
        apis: &ManagedApis,
        error_accumulator: &mut ErrorAccumulator,
    ) -> anyhow::Result<BlessedFiles> {
        let mut api_files: ApiSpecFilesBuilder<BlessedApiSpecFile> =
            ApiSpecFilesBuilder::new(apis, error_accumulator);
        let files_found = git_ls_tree(&commit, directory)?;
        for f in files_found {
            // We should be looking at either a single-component path
            // ("api.json") or a file inside one level of directory hierarchy
            // ("api/api-1.2.3-hash.json").  Figure out which case we're in.
            let parts: Vec<_> = f.iter().collect();
            if parts.is_empty() || parts.len() > 2 {
                api_files.load_warning(anyhow!(
                    "path {:?}: can't understand this path name",
                    f
                ));
                continue;
            }

            // Read the contents.
            let contents = git_show_file(commit, &directory.join(&f))?;
            if parts.len() == 1 {
                if let Some(file_name) = api_files.lockstep_file_name(parts[0])
                {
                    api_files.load_contents(file_name, contents);
                }
            } else if parts.len() == 2 {
                if let Some(ident) = api_files.versioned_directory(parts[0]) {
                    if ident.versioned_api_is_latest_symlink(parts[1]) {
                        // This is the "latest" symlink.  We could dereference
                        // it and report it here, but it's not relevant for
                        // anything this tool does, so we don't bother.
                        continue;
                    }

                    if let Some(file_name) =
                        api_files.versioned_file_name(&ident, parts[1])
                    {
                        api_files.load_contents(file_name, contents);
                    }
                }
            }
        }

        Ok(BlessedFiles::from(api_files))
    }
}

impl<'a> From<ApiSpecFilesBuilder<'a, BlessedApiSpecFile>> for BlessedFiles {
    fn from(api_files: ApiSpecFilesBuilder<'a, BlessedApiSpecFile>) -> Self {
        BlessedFiles(api_files.into_map())
    }
}
