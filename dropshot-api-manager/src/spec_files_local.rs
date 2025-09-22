// Copyright 2025 Oxide Computer Company

//! Newtype and collection to represent OpenAPI documents local to this working
//! tree

use crate::{
    apis::ManagedApis,
    environment::ErrorAccumulator,
    spec_files_generic::{
        ApiFiles, ApiLoad, ApiSpecFile, ApiSpecFilesBuilder, AsRawFiles,
    },
};
use anyhow::{anyhow, Context};
use camino::Utf8Path;
use dropshot_api_manager_types::ApiIdent;
use std::{collections::BTreeMap, ops::Deref};

/// Newtype wrapper around [`ApiSpecFile`] to describe OpenAPI documents found
/// in this working tree
///
/// This includes documents for lockstep APIs and versioned APIs, for both
/// blessed and locally-added versions.
pub struct LocalApiSpecFile(ApiSpecFile);
NewtypeDebug! { () pub struct LocalApiSpecFile(ApiSpecFile); }
NewtypeDeref! { () pub struct LocalApiSpecFile(ApiSpecFile); }
NewtypeDerefMut! { () pub struct LocalApiSpecFile(ApiSpecFile); }
NewtypeFrom! { () pub struct LocalApiSpecFile(ApiSpecFile); }

// Trait impls that allow us to use `ApiFiles<Vec<LocalApiSpecFile>>`
//
// Note that this is a `Vec` because it's allowed to have more than one
// LocalApiSpecFile for a given version.

impl ApiLoad for Vec<LocalApiSpecFile> {
    const MISCONFIGURATIONS_ALLOWED: bool = false;

    fn try_extend(&mut self, item: ApiSpecFile) -> anyhow::Result<()> {
        self.push(LocalApiSpecFile::from(item));
        Ok(())
    }

    fn make_item(raw: ApiSpecFile) -> Self {
        vec![LocalApiSpecFile::from(raw)]
    }
}

impl AsRawFiles for Vec<LocalApiSpecFile> {
    fn as_raw_files<'a>(
        &'a self,
    ) -> Box<dyn Iterator<Item = &'a ApiSpecFile> + 'a> {
        Box::new(self.iter().map(|t| t.deref()))
    }
}

/// Container for OpenAPI documents found in the local working tree
///
/// **Be sure to check for load errors and warnings before using this
/// structure.**
///
/// For more on what's been validated at this point, see
/// [`ApiSpecFilesBuilder`].
#[derive(Debug)]
pub struct LocalFiles(BTreeMap<ApiIdent, ApiFiles<Vec<LocalApiSpecFile>>>);

NewtypeDeref! {
    () pub struct LocalFiles(
        BTreeMap<ApiIdent, ApiFiles<Vec<LocalApiSpecFile>>>
    );
}

impl LocalFiles {
    /// Load OpenAPI documents from a given directory tree
    ///
    /// If it's at all possible to load any documents, this will return an `Ok`
    /// value, but you should still check the `errors` field on the returned
    /// [`LocalFiles`].
    pub fn load_from_directory(
        dir: &Utf8Path,
        apis: &ManagedApis,
        error_accumulator: &mut ErrorAccumulator,
    ) -> anyhow::Result<LocalFiles> {
        let api_files = walk_local_directory(dir, apis, error_accumulator)?;
        Ok(Self::from(api_files))
    }
}

impl From<ApiSpecFilesBuilder<'_, Vec<LocalApiSpecFile>>> for LocalFiles {
    fn from(api_files: ApiSpecFilesBuilder<Vec<LocalApiSpecFile>>) -> Self {
        LocalFiles(api_files.into_map())
    }
}

/// Load OpenAPI documents for the local directory tree
///
/// Under `dir`, we expect to find either:
///
/// * for each lockstep API, a file called `api-ident.json` (e.g., `wicketd.json`)
/// * for each versioned API, a directory called `api-ident` that contains:
///     * any number of files called `api-ident-SEMVER-HASH.json`
///       (e.g., dns-server-1.0.0-eb52aeeb.json)
///     * one symlink called `api-ident-latest.json` that points to a file in
///       the same directory
///
/// Here's an example:
///
/// ```text
/// wicketd.json                                # file for lockstep API
/// dns-server/                                 # directory for versioned API
/// dns-server/dns-server-1.0.0-eb2aeeb.json    # file for versioned API
/// dns-server/dns-server-2.0.0-298ea47.json    # file for versioned API
/// dns-server/dns-server-latest.json           # symlink
/// ```
// This function is always used for the "local" files.  It can sometimes be
// used for both generated and blessed files, if the user asks to load those
// from the local filesystem instead of their usual sources.
pub fn walk_local_directory<'a, T: ApiLoad + AsRawFiles>(
    dir: &'_ Utf8Path,
    apis: &'a ManagedApis,
    error_accumulator: &'a mut ErrorAccumulator,
) -> anyhow::Result<ApiSpecFilesBuilder<'a, T>> {
    let mut api_files = ApiSpecFilesBuilder::new(apis, error_accumulator);
    let entry_iter =
        dir.read_dir_utf8().with_context(|| format!("readdir {:?}", dir))?;
    for maybe_entry in entry_iter {
        let entry =
            maybe_entry.with_context(|| format!("readdir {:?} entry", dir))?;

        // If this entry is a file, then we'd expect it to be the JSON file
        // for one of our lockstep APIs.  Check and see.
        let path = entry.path();
        let file_name = entry.file_name();
        let file_type = entry
            .file_type()
            .with_context(|| format!("file type of {:?}", path))?;
        if file_type.is_file() {
            match fs_err::read(path) {
                Ok(contents) => {
                    if let Some(file_name) =
                        api_files.lockstep_file_name(file_name)
                    {
                        api_files.load_contents(file_name, contents);
                    }
                }
                Err(error) => {
                    api_files.load_error(anyhow!(error));
                }
            };
        } else if file_type.is_dir() {
            load_versioned_directory(&mut api_files, path, file_name);
        } else {
            // This is not something the tool cares about, but it's not
            // obviously a problem, either.
            api_files.load_warning(anyhow!(
                "ignored (not a file or directory): {:?}",
                path
            ));
        };
    }

    Ok(api_files)
}

/// Load the contents of a directory that corresponds to a versioned API.
///
/// See [`walk_local_directory()`] for what we expect to find.
fn load_versioned_directory<T: ApiLoad + AsRawFiles>(
    api_files: &mut ApiSpecFilesBuilder<'_, T>,
    path: &Utf8Path,
    basename: &str,
) {
    let Some(ident) = api_files.versioned_directory(basename) else {
        return;
    };

    let entries = match path
        .read_dir_utf8()
        .and_then(|entry_iter| entry_iter.collect::<Result<Vec<_>, _>>())
    {
        Ok(entries) => entries,
        Err(error) => {
            api_files.load_error(
                anyhow!(error).context(format!("readdir {:?}", path)),
            );
            return;
        }
    };

    for entry in entries {
        let file_name = entry.file_name();

        if ident.versioned_api_is_latest_symlink(file_name) {
            // We should be looking at a symlink.
            let symlink = match entry.path().read_link_utf8() {
                Ok(s) => s,
                Err(error) => {
                    api_files.load_error(anyhow!(error).context(format!(
                        "read what should be a symlink {:?}",
                        entry.path()
                    )));
                    continue;
                }
            };

            if let Some(v) = api_files.symlink_contents(
                entry.path(),
                &ident,
                symlink.as_str(),
            ) {
                api_files.load_latest_link(&ident, v);
            }
            continue;
        }

        let Some(file_name) = api_files.versioned_file_name(&ident, file_name)
        else {
            continue;
        };

        let contents = match fs_err::read(&entry.path()) {
            Ok(contents) => contents,
            Err(error) => {
                api_files.load_error(anyhow!(error));
                continue;
            }
        };

        api_files.load_contents(file_name, contents);
    }
}
