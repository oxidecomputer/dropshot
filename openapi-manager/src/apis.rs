// Copyright 2025 Oxide Computer Company

use anyhow::{bail, Context};
use dropshot::{ApiDescription, ApiDescriptionBuildErrors, StubContext};
use itertools::Either;
use openapi_manager_types::{
    SupportedVersion, SupportedVersions, ValidationContext,
};
use openapiv3::OpenAPI;
use std::{collections::BTreeMap, fmt};

/// Describes an API managed by the openapi-manager crate and CLI tool
///
/// This struct exactly matches how we want developers to configure the list of
/// APIs managed by this tool.
pub struct ManagedApiConfig {
    /// The API-specific part of the filename that's used for API descriptions
    ///
    /// This string is sometimes used as an identifier for developers.
    pub ident: &'static str,

    /// how this API is versioned
    pub versions: Versions,

    /// title of the API (goes into OpenAPI spec)
    pub title: &'static str,

    /// human-readable description of the API (goes into OpenAPI spec)
    pub description: &'static str,

    /// whether this API is internal or external
    ///
    /// This affects some of the validation applied to the OpenAPI spec.
    pub boundary: ApiBoundary,

    /// The API description function, typically a reference to
    /// `stub_api_description`
    ///
    /// This is used to generate the OpenAPI spec that matches the current
    /// server implementation.
    pub api_description:
        fn() -> Result<ApiDescription<StubContext>, ApiDescriptionBuildErrors>,

    /// Extra validation to perform on the OpenAPI spec, if any.
    pub extra_validation: Option<fn(&OpenAPI, ValidationContext<'_>)>,
}

/// Used internally to describe an API managed by this tool
#[derive(Debug)]
pub struct ManagedApi {
    /// The API-specific part of the filename that's used for API descriptions
    ///
    /// This string is sometimes used as an identifier for developers.
    ident: ApiIdent,

    /// how this API is versioned
    versions: Versions,

    /// title of the API (goes into OpenAPI spec)
    title: &'static str,

    /// human-readable description of the API (goes into OpenAPI spec)
    description: &'static str,

    /// whether this API is internal or external
    ///
    /// This affects some of the validation applied to the OpenAPI spec.
    boundary: ApiBoundary,

    /// The API description function, typically a reference to
    /// `stub_api_description`
    ///
    /// This is used to generate the OpenAPI spec that matches the current
    /// server implementation.
    api_description:
        fn() -> Result<ApiDescription<StubContext>, ApiDescriptionBuildErrors>,

    /// Extra validation to perform on the OpenAPI spec, if any.
    extra_validation: Option<fn(&OpenAPI, ValidationContext<'_>)>,
}

impl From<ManagedApiConfig> for ManagedApi {
    fn from(value: ManagedApiConfig) -> Self {
        ManagedApi {
            ident: ApiIdent::from(value.ident.to_owned()),
            versions: value.versions,
            title: value.title,
            description: value.description,
            boundary: value.boundary,
            api_description: value.api_description,
            extra_validation: value.extra_validation,
        }
    }
}

impl ManagedApi {
    pub fn ident(&self) -> &ApiIdent {
        &self.ident
    }

    pub fn title(&self) -> &str {
        self.title
    }

    pub fn description(&self) -> &str {
        self.description
    }

    pub fn is_lockstep(&self) -> bool {
        self.versions.is_lockstep()
    }

    pub fn is_versioned(&self) -> bool {
        self.versions.is_versioned()
    }

    pub fn iter_versioned_versions(
        &self,
    ) -> Option<impl Iterator<Item = &SupportedVersion> + '_> {
        self.versions.iter_versioned_versions()
    }

    pub fn iter_versions_semver(
        &self,
    ) -> impl Iterator<Item = &semver::Version> + '_ {
        self.versions.iter_versions_semvers()
    }

    pub fn generate_openapi_doc(
        &self,
        version: &semver::Version,
    ) -> anyhow::Result<OpenAPI> {
        // It's a bit weird to first convert to bytes and then back to OpenAPI,
        // but this is the easiest way to do so (currently, Dropshot doesn't
        // return the OpenAPI type directly). It is also consistent with the
        // other code paths.
        let contents = self.generate_spec_bytes(version)?;
        serde_json::from_slice(&contents)
            .context("generated document is not valid OpenAPI")
    }

    pub fn generate_spec_bytes(
        &self,
        version: &semver::Version,
    ) -> anyhow::Result<Vec<u8>> {
        let description = (self.api_description)().map_err(|error| {
            // ApiDescriptionBuildError is actually a list of errors so it
            // doesn't implement std::error::Error itself. Its Display
            // impl formats the errors appropriately.
            anyhow::anyhow!("{}", error)
        })?;
        let mut openapi_def = description.openapi(&self.title, version.clone());
        openapi_def
            .description(&self.description)
            .contact_url("https://oxide.computer")
            .contact_email("api@oxide.computer");

        // Use write because it's the most reliable way to get the canonical
        // JSON order. The `json` method returns a serde_json::Value which may
        // or may not have preserve_order enabled.
        let mut contents = Vec::new();
        openapi_def.write(&mut contents)?;
        Ok(contents)
    }

    pub fn boundary(&self) -> ApiBoundary {
        self.boundary
    }

    pub fn extra_validation(
        &self,
        openapi: &OpenAPI,
        validation_context: ValidationContext<'_>,
    ) {
        if let Some(extra_validation) = self.extra_validation {
            extra_validation(openapi, validation_context);
        }
    }
}

/// Describes the Rust-defined configuration for all of the APIs managed by this
/// tool.
///
/// This is repo-specific state that's passed into the OpenAPI manager.
#[derive(Debug)]
pub struct ManagedApis {
    apis: BTreeMap<ApiIdent, ManagedApi>,
}

impl ManagedApis {
    pub fn new(api_list: Vec<ManagedApi>) -> anyhow::Result<ManagedApis> {
        let mut apis = BTreeMap::new();
        for api in api_list {
            if api.extra_validation.is_some() && api.is_versioned() {
                // Extra validation is not yet supported for versioned APIs.
                // The reason is that extra validation can instruct this tool to
                // check the contents of additional files (e.g.,
                // nexus_tags.txt).  We'd need to figure out if we want to
                // maintain expected output for each supported version, only
                // check the latest version, or what.  (Since this is currenty
                // only used for nexus_tags, it would probably be okay to just
                // check the latest version.)  Rather than deal with any of
                // this, we punt for now.  We can revisit this if/when it comes
                // up.
                bail!("extra validation is not supported for versioned APIs");
            }

            if let Some(old) = apis.insert(api.ident.clone(), api) {
                bail!("API is defined twice: {:?}", &old.ident);
            }
        }

        Ok(ManagedApis { apis })
    }

    pub fn len(&self) -> usize {
        self.apis.len()
    }

    pub fn iter_apis(&self) -> impl Iterator<Item = &'_ ManagedApi> + '_ {
        self.apis.values()
    }

    pub fn api(&self, ident: &ApiIdent) -> Option<&ManagedApi> {
        self.apis.get(ident)
    }
}

/// Newtype for API identifiers

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct ApiIdent(String);
NewtypeDebug! { () pub struct ApiIdent(String); }
NewtypeDeref! { () pub struct ApiIdent(String); }
NewtypeDisplay! { () pub struct ApiIdent(String); }
NewtypeFrom! { () pub struct ApiIdent(String); }

impl ApiIdent {
    /// Given an API identifier, return the basename of its "latest" symlink
    pub fn versioned_api_latest_symlink(&self) -> String {
        format!("{self}-latest.json")
    }

    /// Given an API identifier and a file name, determine if we're looking at
    /// this API's "latest" symlink
    pub fn versioned_api_is_latest_symlink(&self, base_name: &str) -> bool {
        base_name == self.versioned_api_latest_symlink()
    }
}

/// Whether an API is exposed externally from the Oxide system
///
/// This affects the kind of validation that's done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiBoundary {
    Internal,
    External,
}

impl fmt::Display for ApiBoundary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiBoundary::Internal => write!(f, "internal"),
            ApiBoundary::External => write!(f, "external"),
        }
    }
}

/// Describes how an API is versioned
#[derive(Debug)]
pub enum Versions {
    /// There is only ever one version of this API
    ///
    /// Clients and servers are updated at runtime in lockstep.
    Lockstep { version: semver::Version },

    /// There are multiple supported versions of this API
    ///
    /// Clients and servers may be updated independently of each other.  Other
    /// parts of the system may constrain things so that either clients or
    /// servers are always updated first, but this tool does not assume that.
    Versioned { supported_versions: SupportedVersions },
}

impl Versions {
    /// Constructor for a lockstep API
    pub fn new_lockstep(version: semver::Version) -> Versions {
        Versions::Lockstep { version }
    }

    /// Constructor for a versioned API
    pub fn new_versioned(supported_versions: SupportedVersions) -> Versions {
        Versions::Versioned { supported_versions }
    }

    /// Returns whether this API is versioned (as opposed to lockstep)
    pub fn is_versioned(&self) -> bool {
        match self {
            Versions::Lockstep { .. } => false,
            Versions::Versioned { .. } => true,
        }
    }

    /// Returns whether this API is lockstep (as opposed to versioned)
    pub fn is_lockstep(&self) -> bool {
        match self {
            Versions::Lockstep { .. } => true,
            Versions::Versioned { .. } => false,
        }
    }

    /// Iterate over the semver versions of an API that are supported
    pub fn iter_versions_semvers(
        &self,
    ) -> impl Iterator<Item = &semver::Version> + '_ {
        match self {
            Versions::Lockstep { version } => {
                Either::Left(std::iter::once(version))
            }
            Versions::Versioned { supported_versions } => {
                Either::Right(supported_versions.iter().map(|v| v.semver()))
            }
        }
    }

    /// For versioned APIs only, iterate over the SupportedVersions
    pub fn iter_versioned_versions(
        &self,
    ) -> Option<impl Iterator<Item = &SupportedVersion> + '_> {
        match self {
            Versions::Lockstep { .. } => None,
            Versions::Versioned { supported_versions } => {
                Some(supported_versions.iter())
            }
        }
    }
}
