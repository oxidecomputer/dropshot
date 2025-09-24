// Copyright 2025 Oxide Computer Company

use anyhow::{bail, Context};
use dropshot::{ApiDescription, ApiDescriptionBuildErrors, StubContext};
use dropshot_api_manager_types::{
    ApiIdent, ManagedApiMetadata, SupportedVersion, ValidationContext, Versions,
};
use openapiv3::OpenAPI;
use std::collections::BTreeMap;

/// Describes an API managed by the dropshot-api-manager CLI tool.
#[derive(Clone, Debug)]
pub struct ManagedApiConfig {
    /// The API-specific part of the filename that's used for API descriptions
    ///
    /// This string is sometimes used as an identifier for developers.
    pub ident: &'static str,

    /// how this API is versioned
    pub versions: Versions,

    /// title of the API (goes into OpenAPI spec)
    pub title: &'static str,

    /// metadata about the API
    pub metadata: ManagedApiMetadata,

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

/// Used internally to describe an API managed by this tool.
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

    /// metadata about the API
    metadata: ManagedApiMetadata,

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
            metadata: value.metadata,
            api_description: value.api_description,
            extra_validation: value.extra_validation,
        }
    }
}

impl ManagedApi {
    pub fn ident(&self) -> &ApiIdent {
        &self.ident
    }

    pub fn versions(&self) -> &Versions {
        &self.versions
    }

    pub fn title(&self) -> &'static str {
        self.title
    }

    pub fn metadata(&self) -> &ManagedApiMetadata {
        &self.metadata
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
        let mut openapi_def = description.openapi(self.title, version.clone());
        if let Some(description) = self.metadata.description {
            openapi_def.description(description);
        }
        if let Some(contact_url) = self.metadata.contact_url {
            openapi_def.contact_url(contact_url);
        }
        if let Some(contact_email) = self.metadata.contact_email {
            openapi_def.contact_email(contact_email);
        }

        // Use write because it's the most reliable way to get the canonical
        // JSON order. The `json` method returns a serde_json::Value which may
        // or may not have preserve_order enabled.
        let mut contents = Vec::new();
        openapi_def.write(&mut contents)?;
        Ok(contents)
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
    validation: Option<fn(&OpenAPI, ValidationContext<'_>)>,
}

impl ManagedApis {
    pub fn new(api_list: Vec<ManagedApiConfig>) -> anyhow::Result<ManagedApis> {
        let mut apis = BTreeMap::new();
        for api in api_list {
            let api = ManagedApi::from(api);
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

        Ok(ManagedApis { apis, validation: None })
    }

    /// Sets a validation function to be used for all APIs.
    ///
    /// This function will be called for each API document. The
    /// [`ValidationContext`] can be used to report errors, as well as extra
    /// files for which the contents need to be compared with those on disk.
    pub fn with_validation(
        mut self,
        validation: fn(&OpenAPI, ValidationContext<'_>),
    ) -> Self {
        self.validation = Some(validation);
        self
    }

    /// Returns the validation function for all APIs.
    pub fn validation(&self) -> Option<fn(&OpenAPI, ValidationContext<'_>)> {
        self.validation
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
