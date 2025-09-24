// Copyright 2025 Oxide Computer Company

//! Newtype and collection to represent OpenAPI documents generated from the
//! API definitions

use crate::{
    apis::ManagedApis,
    environment::ErrorAccumulator,
    spec_files_generic::{
        hash_contents, ApiFiles, ApiLoad, ApiSpecFile, ApiSpecFilesBuilder,
        AsRawFiles,
    },
};
use anyhow::{anyhow, bail};
use dropshot_api_manager_types::{
    ApiIdent, ApiSpecFileName, ApiSpecFileNameKind,
};
use std::{collections::BTreeMap, ops::Deref};

/// Newtype wrapper around [`ApiSpecFile`] to describe OpenAPI documents
/// generated from API definitions
///
/// This includes documents for lockstep APIs and versioned APIs, for both
/// blessed and locally-added versions.
pub struct GeneratedApiSpecFile(ApiSpecFile);
NewtypeDebug! { () pub struct GeneratedApiSpecFile(ApiSpecFile); }
NewtypeDeref! { () pub struct GeneratedApiSpecFile(ApiSpecFile); }
NewtypeDerefMut! { () pub struct GeneratedApiSpecFile(ApiSpecFile); }
NewtypeFrom! { () pub struct GeneratedApiSpecFile(ApiSpecFile); }

// Trait impls that allow us to use `ApiFiles<GeneratedApiSpecFile>`
//
// Note that this is NOT a `Vec` because it's NOT allowed to have more than one
// GeneratedApiSpecFile for a given version.

impl ApiLoad for GeneratedApiSpecFile {
    const MISCONFIGURATIONS_ALLOWED: bool = false;

    fn make_item(raw: ApiSpecFile) -> Self {
        GeneratedApiSpecFile(raw)
    }

    fn try_extend(&mut self, item: ApiSpecFile) -> anyhow::Result<()> {
        // This should be impossible.
        bail!(
            "found more than one generated OpenAPI document for a given \
             API version: at least {} and {}",
            self.spec_file_name(),
            item.spec_file_name()
        );
    }
}

impl AsRawFiles for GeneratedApiSpecFile {
    fn as_raw_files<'a>(
        &'a self,
    ) -> Box<dyn Iterator<Item = &'a ApiSpecFile> + 'a> {
        Box::new(std::iter::once(self.deref()))
    }
}

/// Container for OpenAPI spec files generated from API definitions
///
/// **Be sure to check for load errors and warnings before using this
/// structure.**
///
/// For more on what's been validated at this point, see
/// [`ApiSpecFilesBuilder`].
pub struct GeneratedFiles(BTreeMap<ApiIdent, ApiFiles<GeneratedApiSpecFile>>);
NewtypeDeref! {
    () pub struct GeneratedFiles(
        BTreeMap<ApiIdent, ApiFiles<GeneratedApiSpecFile>>
    );
}

impl GeneratedFiles {
    /// Generate OpenAPI documents for all supported versions of all managed
    /// APIs
    pub fn generate(
        apis: &ManagedApis,
        error_accumulator: &mut ErrorAccumulator,
    ) -> anyhow::Result<GeneratedFiles> {
        let mut api_files: ApiSpecFilesBuilder<GeneratedApiSpecFile> =
            ApiSpecFilesBuilder::new(apis, error_accumulator);

        for api in apis.iter_apis() {
            if api.is_lockstep() {
                for version in api.iter_versions_semver() {
                    match api.generate_spec_bytes(version) {
                        Err(error) => {
                            api_files.load_error(error.context(format!(
                                "generating OpenAPI document for lockstep \
                                 API {:?}",
                                api.ident()
                            )))
                        }
                        Ok(contents) => {
                            let file_name = ApiSpecFileName::new(
                                api.ident().clone(),
                                ApiSpecFileNameKind::Lockstep,
                            );
                            api_files.load_contents(file_name, contents);
                        }
                    }
                }
            } else {
                let supported_versions = api.iter_versioned_versions().expect(
                    "iter_versioned_versions() returns `Some` for versioned \
                     APIs",
                );
                let mut latest = None;
                for supported_version in supported_versions {
                    let version = supported_version.semver();
                    match api.generate_spec_bytes(version) {
                        Err(error) => {
                            api_files.load_error(error.context(format!(
                                "generating OpenAPI document for versioned \
                                 API {:?} version {}",
                                api.ident(),
                                version
                            )))
                        }
                        Ok(contents) => {
                            let file_name = ApiSpecFileName::new(
                                api.ident().clone(),
                                ApiSpecFileNameKind::Versioned {
                                    version: version.clone(),
                                    hash: hash_contents(&contents),
                                },
                            );
                            latest = Some(file_name.clone());
                            api_files.load_contents(file_name, contents);
                        }
                    }
                }

                match latest {
                    Some(latest) => {
                        api_files.load_latest_link(api.ident(), latest)
                    }
                    None => api_files.load_error(anyhow!(
                        "versioned API {:?} symlink: there is no working \
                             version (fix above error(s) first)",
                        api.ident(),
                    )),
                }
            }
        }

        Ok(Self::from(api_files))
    }
}

impl<'a> From<ApiSpecFilesBuilder<'a, GeneratedApiSpecFile>>
    for GeneratedFiles
{
    fn from(api_files: ApiSpecFilesBuilder<'a, GeneratedApiSpecFile>) -> Self {
        GeneratedFiles(api_files.into_map())
    }
}
