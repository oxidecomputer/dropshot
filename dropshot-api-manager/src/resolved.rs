// Copyright 2025 Oxide Computer Company

//! Resolve different sources of API information (blessed, local, upstream)

use crate::apis::ManagedApi;
use crate::apis::ManagedApis;
use crate::compatibility::api_compatible;
use crate::compatibility::OpenApiCompatibilityError;
use crate::environment::ResolvedEnv;
use crate::iter_only::iter_only;
use crate::output::plural;
use crate::spec_files_blessed::BlessedApiSpecFile;
use crate::spec_files_blessed::BlessedFiles;
use crate::spec_files_generated::GeneratedApiSpecFile;
use crate::spec_files_generated::GeneratedFiles;
use crate::spec_files_generic::ApiFiles;
use crate::spec_files_local::LocalApiSpecFile;
use crate::spec_files_local::LocalFiles;
use crate::validation::overwrite_file;
use crate::validation::validate;
use crate::validation::CheckStale;
use crate::validation::CheckStatus;
use anyhow::{anyhow, Context};
use camino::Utf8Path;
use camino::Utf8PathBuf;
use dropshot_api_manager_types::ApiIdent;
use dropshot_api_manager_types::ApiSpecFileName;
use dropshot_api_manager_types::ValidationContext;
use openapiv3::OpenAPI;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Debug;
use std::fmt::Display;
use thiserror::Error;

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct DisplayableVec<T>(pub Vec<T>);
impl<T> Display for DisplayableVec<T>
where
    T: Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // slice::join would require the use of unstable Rust.
        let mut iter = self.0.iter();
        if let Some(item) = iter.next() {
            write!(f, "{item}")?;
        }

        for item in iter {
            write!(f, ", {item}")?;
        }

        Ok(())
    }
}

/// A non-error note that's worth highlighting to the user
// These are not technically errors, but it is useful to treat them the same
// way in terms of having an associated message, etc.
#[derive(Debug, Error)]
pub enum Note {
    /// A previously-supported API version has been removed locally.
    ///
    /// This is not an error because we do expect to EOL old API specs.  There's
    /// not currently a way for this tool to know if the EOL'ing is correct or
    /// not, so we at least highlight it to the user.
    #[error(
        "API {api_ident} version {version}: formerly blessed version has been \
         removed.  This version will no longer be supported!  This will break \
         upgrade from software that still uses this version.  If this is \
         unexpected, check the list of supported versions in Rust for a \
         possible mismerge."
    )]
    BlessedVersionRemoved { api_ident: ApiIdent, version: semver::Version },
}

/// Describes the result of resolving the blessed spec(s), generated spec(s),
/// and local spec files for a particular API
pub struct Resolution<'a> {
    kind: ResolutionKind,
    problems: Vec<Problem<'a>>,
}

impl<'a> Resolution<'a> {
    pub fn new_lockstep(problems: Vec<Problem<'a>>) -> Resolution<'a> {
        Resolution { kind: ResolutionKind::Lockstep, problems }
    }

    pub fn new_blessed(problems: Vec<Problem<'a>>) -> Resolution<'a> {
        Resolution { kind: ResolutionKind::Blessed, problems }
    }

    pub fn new_new_locally(problems: Vec<Problem<'a>>) -> Resolution<'a> {
        Resolution { kind: ResolutionKind::NewLocally, problems }
    }

    pub fn has_problems(&self) -> bool {
        !self.problems.is_empty()
    }

    pub fn has_errors(&self) -> bool {
        self.problems().any(|p| !p.is_fixable())
    }

    pub fn problems(&self) -> impl Iterator<Item = &'_ Problem<'a>> + '_ {
        self.problems.iter()
    }

    pub fn kind(&self) -> ResolutionKind {
        self.kind
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionKind {
    /// This is a lockstep API
    Lockstep,
    /// This is a versioned API and this version is blessed
    Blessed,
    /// This version is new to the current workspace (i.e., not present
    /// upstream)
    NewLocally,
}

impl Display for ResolutionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ResolutionKind::Lockstep => "lockstep",
            ResolutionKind::Blessed => "blessed",
            ResolutionKind::NewLocally => "added locally",
        })
    }
}

/// Describes a problem resolving the blessed spec(s), generated spec(s), and
/// local spec files for a particular API
#[derive(Debug, Error)]
pub enum Problem<'a> {
    // This kind of problem is not associated with any *supported* version of an
    // API.  (All the others are.)
    #[error(
        "A local OpenAPI document was found that does not correspond to a \
         supported version of this API: {spec_file_name}.  This is unusual, \
         but it could happen if you're either retiring an older version of \
         this API or if you created this version in this branch and later \
         merged with upstream and had to change your local version number.  \
         In either case, this tool can remove the unused file for you."
    )]
    LocalSpecFileOrphaned { spec_file_name: ApiSpecFileName },

    // All other problems are associated with specific supported versions of an
    // API.
    #[error(
        "This version is blessed, and it's a supported version, but it's \
         missing a local OpenAPI document.  This is unusual.  If you intended \
         to remove this version, you must also update the list of supported \
         versions in Rust.  If you didn't, restore the file from git: \
         {spec_file_name}"
    )]
    BlessedVersionMissingLocal { spec_file_name: ApiSpecFileName },

    #[error(
        "For this blessed version, found an extra OpenAPI document that does \
         not match the blessed (upstream) OpenAPI document: {spec_file_name}.  \
         This can happen if you created this version of the API in this branch, \
         then merged with an upstream commit that also added the same version \
         number.  In that case, you likely already bumped your local version \
         number (when you merged the list of supported versions in Rust) and \
         this file is vestigial. This tool can remove the unused file for you."
    )]
    BlessedVersionExtraLocalSpec { spec_file_name: ApiSpecFileName },

    #[error(
        "OpenAPI document generated from the current code is not compatible \
         with the blessed document (from upstream): {compatibility_issues}"
    )]
    BlessedVersionBroken {
        compatibility_issues: DisplayableVec<OpenApiCompatibilityError>,
    },

    #[error(
        "No local OpenAPI document was found for this lockstep API.  This is \
         only expected if you're adding a new lockstep API.  This tool can \
         generate the file for you."
    )]
    LockstepMissingLocal { generated: &'a GeneratedApiSpecFile },

    #[error(
        "For this lockstep API, OpenAPI document generated from the current \
         code does not match the local file: {:?}.  This tool can update the \
         local file for you.", generated.spec_file_name().path()
    )]
    LockstepStale {
        found: &'a LocalApiSpecFile,
        generated: &'a GeneratedApiSpecFile,
    },

    #[error(
        "No OpenAPI document was found for this locally-added API version.  \
         This is normal if you have added or changed this API version.  \
         This tool can generate the file for you."
    )]
    LocalVersionMissingLocal { generated: &'a GeneratedApiSpecFile },

    #[error(
        "Extra (incorrect) OpenAPI documents were found for locally-added \
         version: {spec_file_names}.  This tool can remove the files for you."
    )]
    LocalVersionExtra { spec_file_names: DisplayableVec<ApiSpecFileName> },

    #[error(
        "For this locally-added version, the OpenAPI document generated \
         from the current code does not match the local file: {}. \
         This tool can update the local file(s) for you.",
        DisplayableVec(
            spec_files.iter().map(|s| s.spec_file_name().to_string()).collect()
        )
    )]
    // For versioned APIs, since the filename has its own hash in it, when the
    // local file is stale, it's not that the file contents will be wrong, but
    // rather that there will be one or more _incorrect_ files and the correct
    // one will be missing.  The fix will be to remove all the incorrect ones
    // and add the correct one.
    LocalVersionStale {
        spec_files: Vec<&'a LocalApiSpecFile>,
        generated: &'a GeneratedApiSpecFile,
    },

    #[error(
        "Generated OpenAPI document for API {api_ident:?} version {version} \
         is not valid"
    )]
    GeneratedValidationError {
        api_ident: ApiIdent,
        version: semver::Version,
        #[source]
        source: anyhow::Error,
    },

    #[error(
        "Additional validated file associated with API {api_ident:?} is \
         stale: {path}"
    )]
    ExtraFileStale {
        api_ident: ApiIdent,
        path: Utf8PathBuf,
        check_stale: CheckStale,
    },

    #[error("\"Latest\" symlink for versioned API {api_ident:?} is missing")]
    LatestLinkMissing { api_ident: ApiIdent, link: &'a ApiSpecFileName },

    #[error(
        "\"Latest\" symlink for versioned API {api_ident:?} is stale: points \
         to {}, but should be {}",
         found.basename(),
         link.basename(),
    )]
    LatestLinkStale {
        api_ident: ApiIdent,
        found: &'a ApiSpecFileName,
        link: &'a ApiSpecFileName,
    },
}

impl<'a> Problem<'a> {
    pub fn is_fixable(&self) -> bool {
        self.fix().is_some()
    }

    pub fn fix(&'a self) -> Option<Fix<'a>> {
        match self {
            Problem::LocalSpecFileOrphaned { spec_file_name } => {
                Some(Fix::DeleteFiles {
                    files: DisplayableVec(vec![spec_file_name.clone()]),
                })
            }
            Problem::BlessedVersionMissingLocal { .. } => None,
            Problem::BlessedVersionExtraLocalSpec { spec_file_name } => {
                Some(Fix::DeleteFiles {
                    files: DisplayableVec(vec![spec_file_name.clone()]),
                })
            }
            Problem::BlessedVersionBroken { .. } => None,
            Problem::LockstepMissingLocal { generated }
            | Problem::LockstepStale { generated, .. } => {
                Some(Fix::FixLockstepFile { generated })
            }
            Problem::LocalVersionMissingLocal { generated } => {
                Some(Fix::FixVersionedFiles {
                    old: DisplayableVec(Vec::new()),
                    generated,
                })
            }
            Problem::LocalVersionExtra { spec_file_names } => {
                Some(Fix::DeleteFiles { files: spec_file_names.clone() })
            }
            Problem::LocalVersionStale { spec_files, generated } => {
                Some(Fix::FixVersionedFiles {
                    old: DisplayableVec(
                        spec_files.iter().map(|s| s.spec_file_name()).collect(),
                    ),
                    generated,
                })
            }
            Problem::GeneratedValidationError { .. } => None,
            Problem::ExtraFileStale { path, check_stale, .. } => {
                Some(Fix::FixExtraFile { path, check_stale })
            }
            Problem::LatestLinkStale { api_ident, link, .. }
            | Problem::LatestLinkMissing { api_ident, link } => {
                Some(Fix::FixSymlink { api_ident, link })
            }
        }
    }
}

pub enum Fix<'a> {
    DeleteFiles {
        files: DisplayableVec<ApiSpecFileName>,
    },
    FixLockstepFile {
        generated: &'a GeneratedApiSpecFile,
    },
    FixVersionedFiles {
        old: DisplayableVec<&'a ApiSpecFileName>,
        generated: &'a GeneratedApiSpecFile,
    },
    FixExtraFile {
        path: &'a Utf8Path,
        check_stale: &'a CheckStale,
    },
    FixSymlink {
        api_ident: &'a ApiIdent,
        link: &'a ApiSpecFileName,
    },
}

impl Display for Fix<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fix::DeleteFiles { files } => {
                writeln!(
                    f,
                    "delete {}: {files}",
                    plural::files(files.0.len())
                )?;
            }
            Fix::FixLockstepFile { generated } => {
                writeln!(
                    f,
                    "rewrite lockstep file {} from generated",
                    generated.spec_file_name().path()
                )?;
            }
            Fix::FixVersionedFiles { old, generated } => {
                if !old.0.is_empty() {
                    writeln!(
                        f,
                        "remove old {}: {old}",
                        plural::files(old.0.len())
                    )?;
                }
                writeln!(
                    f,
                    "write new file {} from generated",
                    generated.spec_file_name().path()
                )?;
            }
            Fix::FixExtraFile { path, check_stale } => {
                let label = match check_stale {
                    CheckStale::Modified { .. } => "rewrite",
                    CheckStale::New { .. } => "write new",
                };
                writeln!(f, "{label} file {path} from generated")?;
            }
            Fix::FixSymlink { link, .. } => {
                writeln!(f, "update symlink to point to {}", link.basename())?;
            }
        };
        Ok(())
    }
}

impl Fix<'_> {
    pub fn execute(&self, env: &ResolvedEnv) -> anyhow::Result<Vec<String>> {
        let root = env.openapi_abs_dir();
        match self {
            Fix::DeleteFiles { files } => {
                let mut rv = Vec::new();
                for f in &files.0 {
                    let path = root.join(f.path());
                    fs_err::remove_file(&path)?;
                    rv.push(format!("removed {}", path));
                }
                Ok(rv)
            }
            Fix::FixLockstepFile { generated } => {
                let path = root.join(generated.spec_file_name().path());
                Ok(vec![format!(
                    "updated {}: {:?}",
                    &path,
                    overwrite_file(&path, generated.contents())?
                )])
            }
            Fix::FixVersionedFiles { old, generated } => {
                let mut rv = Vec::new();
                for f in &old.0 {
                    let path = root.join(f.path());
                    fs_err::remove_file(&path)?;
                    rv.push(format!("removed {}", path));
                }

                let path = root.join(generated.spec_file_name().path());
                rv.push(format!(
                    "created {}: {:?}",
                    &path,
                    overwrite_file(&path, generated.contents())?
                ));
                Ok(rv)
            }
            Fix::FixExtraFile { path, check_stale } => {
                let expected_contents = match check_stale {
                    CheckStale::Modified { expected, .. } => expected,
                    CheckStale::New { expected } => expected,
                };
                Ok(vec![format!(
                    "wrote {}: {:?}",
                    &path,
                    overwrite_file(&path, expected_contents)?
                )])
            }
            Fix::FixSymlink { api_ident, link } => {
                let path = root
                    .join(api_ident.to_string())
                    .join(api_ident.versioned_api_latest_symlink());
                // We want the link to contain a relative path to a file in the
                // same directory so that it's correct no matter where it's
                // resolved from.
                let target = link.basename();
                match fs_err::remove_file(&path) {
                    Ok(_) => (),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                        ()
                    }
                    Err(err) => {
                        return Err(anyhow!(err).context("removing old link"));
                    }
                };
                symlink_file(&target, &path)?;
                Ok(vec![format!("wrote link {} -> {}", path, target)])
            }
        }
    }
}

#[cfg(unix)]
fn symlink_file(target: &str, path: &Utf8Path) -> std::io::Result<()> {
    fs_err::os::unix::fs::symlink(&target, &path)
}

#[cfg(windows)]
fn symlink_file(target: &str, path: &Utf8Path) -> std::io::Result<()> {
    fs_err::os::windows::fs::symlink_file(&target, &path)
}

/// Resolve differences between blessed spec(s), the generated spec, and any
/// local spec files for a given API
pub struct Resolved<'a> {
    notes: Vec<Note>,
    non_version_problems: Vec<Problem<'a>>,
    api_results: BTreeMap<ApiIdent, ApiResolved<'a>>,
    nexpected_documents: usize,
}

impl<'a> Resolved<'a> {
    pub fn new(
        env: &'a ResolvedEnv,
        apis: &'a ManagedApis,
        blessed: &'a BlessedFiles,
        generated: &'a GeneratedFiles,
        local: &'a LocalFiles,
    ) -> Resolved<'a> {
        // First, assemble a list of supported versions for each API, as defined
        // in the Rust list of supported versions.  We'll use this to identify
        // any blessed spec files or local spec files that don't belong at all.
        let supported_versions_by_api: BTreeMap<
            &ApiIdent,
            BTreeSet<&semver::Version>,
        > = apis
            .iter_apis()
            .map(|api| {
                (
                    api.ident(),
                    api.iter_versions_semver().collect::<BTreeSet<_>>(),
                )
            })
            .collect();

        let nexpected_documents = supported_versions_by_api
            .values()
            .map(|v| v.len())
            .fold(0, |sum_so_far, count| sum_so_far + count);

        // Get one easy case out of the way: if there are any blessed API
        // versions that aren't supported any more, note that.
        let notes = resolve_removed_blessed_versions(
            &supported_versions_by_api,
            blessed,
        )
        .map(|(ident, version)| Note::BlessedVersionRemoved {
            api_ident: ident.clone(),
            version: version.clone(),
        })
        .collect();

        // Get the other easy case out of the way: if there are any local spec
        // files for APIs or API versions that aren't supported any more, that's
        // a (fixable) problem.
        let non_version_problems =
            resolve_orphaned_local_specs(&supported_versions_by_api, local)
                .map(|spec_file_name| Problem::LocalSpecFileOrphaned {
                    spec_file_name: spec_file_name.clone(),
                })
                .collect();

        // Now resolve each of the supported API versions.
        let api_results = apis
            .iter_apis()
            .map(|api| {
                let ident = api.ident().clone();
                let api_blessed = blessed.get(&ident);
                // We should have generated an API for every supported version.
                let api_generated = generated.get(&ident).unwrap();
                let api_local = local.get(&ident);
                (
                    api.ident().clone(),
                    resolve_api(
                        env,
                        api,
                        apis.validation(),
                        api_blessed,
                        api_generated,
                        api_local,
                    ),
                )
            })
            .collect();

        Resolved {
            notes,
            non_version_problems,
            api_results,
            nexpected_documents,
        }
    }

    pub fn nexpected_documents(&self) -> usize {
        self.nexpected_documents
    }

    pub fn notes(&self) -> impl Iterator<Item = &Note> + '_ {
        self.notes.iter()
    }

    pub fn general_problems(&self) -> impl Iterator<Item = &Problem<'a>> + '_ {
        self.non_version_problems.iter()
    }

    pub fn resolution_for_api_version(
        &self,
        ident: &ApiIdent,
        version: &semver::Version,
    ) -> Option<&Resolution<'_>> {
        self.api_results.get(ident).and_then(|v| v.by_version.get(version))
    }

    pub fn symlink_problem(&self, ident: &ApiIdent) -> Option<&Problem<'_>> {
        self.api_results.get(ident).and_then(|v| v.symlink.as_ref())
    }

    pub fn has_unfixable_problems(&self) -> bool {
        self.general_problems().any(|p| !p.is_fixable())
            || self.api_results.values().any(|a| a.has_unfixable_problems())
    }
}

struct ApiResolved<'a> {
    by_version: BTreeMap<semver::Version, Resolution<'a>>,
    symlink: Option<Problem<'a>>,
}

impl ApiResolved<'_> {
    fn has_unfixable_problems(&self) -> bool {
        self.symlink.as_ref().map_or(false, |f| !f.is_fixable())
            || self.by_version.values().any(|r| r.has_errors())
    }
}

fn resolve_removed_blessed_versions<'a>(
    supported_versions_by_api: &'a BTreeMap<
        &'a ApiIdent,
        BTreeSet<&'a semver::Version>,
    >,
    blessed: &'a BlessedFiles,
) -> impl Iterator<Item = (&'a ApiIdent, &'a semver::Version)> + 'a {
    blessed.iter().flat_map(|(ident, api_files)| {
        let set = supported_versions_by_api.get(ident);
        api_files.versions().keys().filter_map(move |version| match set {
            Some(set) if set.contains(version) => None,
            _ => Some((ident, version)),
        })
    })
}

fn resolve_orphaned_local_specs<'a>(
    supported_versions_by_api: &'a BTreeMap<
        &'a ApiIdent,
        BTreeSet<&'a semver::Version>,
    >,
    local: &'a LocalFiles,
) -> impl Iterator<Item = &'a ApiSpecFileName> + 'a {
    local.iter().flat_map(|(ident, api_files)| {
        let set = supported_versions_by_api.get(ident);
        api_files
            .versions()
            .iter()
            .filter_map(move |(version, files)| match set {
                Some(set) if !set.contains(version) => {
                    Some(files.iter().map(|f| f.spec_file_name()))
                }
                _ => None,
            })
            .flatten()
    })
}

fn resolve_api<'a>(
    env: &'a ResolvedEnv,
    api: &'a ManagedApi,
    validation: Option<fn(&OpenAPI, ValidationContext<'_>)>,
    api_blessed: Option<&'a ApiFiles<BlessedApiSpecFile>>,
    api_generated: &'a ApiFiles<GeneratedApiSpecFile>,
    api_local: Option<&'a ApiFiles<Vec<LocalApiSpecFile>>>,
) -> ApiResolved<'a> {
    let (by_version, symlink) = if api.is_lockstep() {
        (
            resolve_api_lockstep(
                env,
                api,
                validation,
                api_generated,
                api_local,
            ),
            None,
        )
    } else {
        let by_version = api
            .iter_versions_semver()
            .map(|version| {
                let version = version.clone();
                let blessed =
                    api_blessed.and_then(|b| b.versions().get(&version));
                let generated = api_generated.versions().get(&version).unwrap();
                let local = api_local
                    .and_then(|b| b.versions().get(&version))
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                let resolution = resolve_api_version(
                    env, api, validation, &version, blessed, generated, local,
                );
                (version, resolution)
            })
            .collect();

        // Check the "latest" symlink.
        let latest_generated = api_generated.latest_link().expect(
            "\"generated\" source should always have a \"latest\" link",
        );
        let symlink = match api_local.and_then(|l| l.latest_link()) {
            Some(latest_local) if latest_local == latest_generated => None,
            Some(latest_local) => Some(Problem::LatestLinkStale {
                api_ident: api.ident().clone(),
                link: latest_generated,
                found: latest_local,
            }),
            None => Some(Problem::LatestLinkMissing {
                api_ident: api.ident().clone(),
                link: latest_generated,
            }),
        };

        (by_version, symlink)
    };

    ApiResolved { by_version, symlink }
}

fn resolve_api_lockstep<'a>(
    env: &'a ResolvedEnv,
    api: &'a ManagedApi,
    validation: Option<fn(&OpenAPI, ValidationContext<'_>)>,
    api_generated: &'a ApiFiles<GeneratedApiSpecFile>,
    api_local: Option<&'a ApiFiles<Vec<LocalApiSpecFile>>>,
) -> BTreeMap<semver::Version, Resolution<'a>> {
    assert!(api.is_lockstep());

    // unwrap(): Lockstep APIs by construction always have exactly one version.
    let version = iter_only(api.iter_versions_semver())
        .with_context(|| {
            format!("list of versions for lockstep API {}", api.ident())
        })
        .unwrap();

    let generated = api_generated
        .versions()
        .get(version)
        .expect("generated OpenAPI document for lockstep API");

    // We may or may not have found a local OpenAPI document for this API.
    let local = api_local
        .and_then(|by_version| by_version.versions().get(version))
        .and_then(|list| match &list.as_slice() {
            &[first] => Some(first),
            &[] => None,
            items => {
                // Structurally, it's not possible to have more than one
                // local file for a lockstep API because the file is named
                // by the API itself.
                unreachable!(
                    "unexpectedly found more than one local OpenAPI \
                     document for lockstep API {}: {:?}",
                    api.ident(),
                    items
                );
            }
        });

    let mut problems = Vec::new();

    // Validate the generated API document.
    validate_generated(env, api, validation, version, generated, &mut problems);

    match local {
        Some(local_file) if local_file.contents() == generated.contents() => (),
        Some(found) => {
            problems.push(Problem::LockstepStale { found, generated })
        }
        None => problems.push(Problem::LockstepMissingLocal { generated }),
    };

    BTreeMap::from([(version.clone(), Resolution::new_lockstep(problems))])
}

fn resolve_api_version<'a>(
    env: &'_ ResolvedEnv,
    api: &'_ ManagedApi,
    validation: Option<fn(&OpenAPI, ValidationContext<'_>)>,
    version: &'_ semver::Version,
    blessed: Option<&'a BlessedApiSpecFile>,
    generated: &'a GeneratedApiSpecFile,
    local: &'a [LocalApiSpecFile],
) -> Resolution<'a> {
    match blessed {
        Some(blessed) => resolve_api_version_blessed(
            env, api, validation, version, blessed, generated, local,
        ),
        None => resolve_api_version_local(
            env, api, validation, version, generated, local,
        ),
    }
}

fn resolve_api_version_blessed<'a>(
    env: &'_ ResolvedEnv,
    api: &'_ ManagedApi,
    validation: Option<fn(&OpenAPI, ValidationContext<'_>)>,
    version: &'_ semver::Version,
    blessed: &'a BlessedApiSpecFile,
    generated: &'a GeneratedApiSpecFile,
    local: &'a [LocalApiSpecFile],
) -> Resolution<'a> {
    let mut problems = Vec::new();

    // Validate the generated API document.
    validate_generated(env, api, validation, version, generated, &mut problems);

    // First off, the blessed spec must be a subset of the generated one.
    // If not, someone has made an incompatible change to the API
    // *implementation*, such that the implementation no longer faithfully
    // implements this older, supported version.
    let issues = api_compatible(blessed.openapi(), generated.openapi());
    if !issues.is_empty() {
        problems.push(Problem::BlessedVersionBroken {
            compatibility_issues: DisplayableVec(issues),
        });
    }

    // Now, there should be at least one local spec that exactly matches the
    // blessed one.
    let (matching, non_matching): (Vec<_>, Vec<_>) =
        local.iter().partition(|local| {
            // It should be enough to compare the hashes, since we should have
            // already validated that the hashes are correct for the contents.
            // But while it's cheap enough to do, we may as well compare the
            // contents, too, and make sure we haven't messed something up.
            let contents_match = local.contents() == blessed.contents();
            let local_hash = local.spec_file_name().hash().expect(
                "this should be a versioned file so it should have a hash",
            );
            let blessed_hash = blessed.spec_file_name().hash().expect(
                "this should be a versioned file so it should have a hash",
            );
            let hashes_match = local_hash == blessed_hash;
            // If the hashes are equal, the contents should be equal, and vice
            // versa.
            assert_eq!(hashes_match, contents_match);
            hashes_match
        });
    if matching.is_empty() {
        problems.push(Problem::BlessedVersionMissingLocal {
            spec_file_name: blessed.spec_file_name().clone(),
        })
    } else {
        // The specs are identified by, among other things, their hash.  Thus,
        // to have two matching specs (i.e., having the same contents), we'd
        // have to have a hash collision.  This is conceivable but unlikely
        // enough that this is more likely a logic bug.
        assert_eq!(matching.len(), 1);
    }

    // There shouldn't be any local specs that match the same version but don't
    // match the same contents.
    problems.extend(non_matching.into_iter().map(|s| {
        Problem::BlessedVersionExtraLocalSpec {
            spec_file_name: s.spec_file_name().clone(),
        }
    }));

    Resolution::new_blessed(problems)
}

fn resolve_api_version_local<'a>(
    env: &'_ ResolvedEnv,
    api: &'_ ManagedApi,
    validation: Option<fn(&OpenAPI, ValidationContext<'_>)>,
    version: &'_ semver::Version,
    generated: &'a GeneratedApiSpecFile,
    local: &'a [LocalApiSpecFile],
) -> Resolution<'a> {
    let mut problems = Vec::new();

    // Validate the generated API document.
    validate_generated(env, api, validation, version, generated, &mut problems);

    let (matching, non_matching): (Vec<_>, Vec<_>) = local
        .iter()
        .partition(|local| local.contents() == generated.contents());

    if matching.is_empty() {
        // There was no matching spec.
        if non_matching.is_empty() {
            // There were no non-matching specs, either.
            problems.push(Problem::LocalVersionMissingLocal { generated });
        } else {
            // There were non-matching specs.  This is your basic "stale" case.
            problems.push(Problem::LocalVersionStale {
                spec_files: non_matching,
                generated,
            });
        }
    } else if !non_matching.is_empty() {
        // There was a matching spec, but also some non-matching ones.
        // These are superfluous.  (It's not clear how this could happen.)
        let spec_file_names = DisplayableVec(
            non_matching.iter().map(|s| s.spec_file_name().clone()).collect(),
        );
        problems.push(Problem::LocalVersionExtra { spec_file_names });
    }

    Resolution::new_new_locally(problems)
}

fn validate_generated(
    env: &ResolvedEnv,
    api: &ManagedApi,
    validation: Option<fn(&OpenAPI, ValidationContext<'_>)>,
    version: &semver::Version,
    generated: &GeneratedApiSpecFile,
    problems: &mut Vec<Problem<'_>>,
) {
    match validate(env, api, validation, generated) {
        Err(source) => {
            problems.push(Problem::GeneratedValidationError {
                api_ident: api.ident().clone(),
                version: version.clone(),
                source,
            });
        }
        Ok(extra_files) => {
            for (path, status) in extra_files {
                match status {
                    CheckStatus::Fresh => (),
                    CheckStatus::Stale(check_stale) => {
                        problems.push(Problem::ExtraFileStale {
                            api_ident: api.ident().clone(),
                            path,
                            check_stale,
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::DisplayableVec;

    #[test]
    fn test_displayable_vec() {
        let v = DisplayableVec(Vec::<usize>::new());
        assert_eq!(v.to_string(), "");

        let v = DisplayableVec(vec![8]);
        assert_eq!(v.to_string(), "8");

        let v = DisplayableVec(vec![8, 12, 14]);
        assert_eq!(v.to_string(), "8, 12, 14");
    }
}
