// Copyright 2025 Oxide Computer Company

use std::{fmt, ops::Deref};

use camino::Utf8PathBuf;

use crate::{ManagedApiMetadata, Versions};

/// Context for validation of OpenAPI specifications.
pub struct ValidationContext<'a> {
    backend: &'a mut dyn ValidationBackend,
}

impl<'a> ValidationContext<'a> {
    /// Not part of the public API -- only called by the OpenAPI manager.
    #[doc(hidden)]
    pub fn new(backend: &'a mut dyn ValidationBackend) -> Self {
        Self { backend }
    }

    /// Retrieves the identifier of the API being validated.
    ///
    /// This identifier is set via the OpenAPI manager's `ManagedApiConfig`
    /// type.
    pub fn ident(&self) -> &ApiIdent {
        self.backend.ident()
    }

    /// Returns a descriptor for the API's file name.
    pub fn file_name(&self) -> &ApiSpecFileName {
        self.backend.file_name()
    }

    /// Retrieves the versioning strategy for this API.
    pub fn versions(&self) -> &Versions {
        self.backend.versions()
    }

    /// Retrieves the title of the API being validated.
    pub fn title(&self) -> &str {
        self.backend.title()
    }

    /// Retrieves optional metadata for the API being validated.
    pub fn metadata(&self) -> &ManagedApiMetadata {
        self.backend.metadata()
    }

    /// Reports a validation error.
    pub fn report_error(&mut self, error: anyhow::Error) {
        self.backend.report_error(error);
    }

    /// Records that the file has the given contents.
    ///
    /// In check mode, if the files differ, an error is logged.
    ///
    /// In generate mode, the file is overwritten with the given contents.
    ///
    /// The path is treated as relative to the root of the repository.
    pub fn record_file_contents(
        &mut self,
        path: impl Into<Utf8PathBuf>,
        contents: Vec<u8>,
    ) {
        self.backend.record_file_contents(path.into(), contents);
    }
}

/// The backend for validation.
///
/// Not part of the public API -- only implemented by the OpenAPI manager.
#[doc(hidden)]
pub trait ValidationBackend {
    fn ident(&self) -> &ApiIdent;
    fn file_name(&self) -> &ApiSpecFileName;
    fn versions(&self) -> &Versions;
    fn title(&self) -> &str;
    fn metadata(&self) -> &ManagedApiMetadata;
    fn report_error(&mut self, error: anyhow::Error);
    fn record_file_contents(&mut self, path: Utf8PathBuf, contents: Vec<u8>);
}

/// Describes the path to an OpenAPI document file, relative to some root where
/// similar documents are found
#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub struct ApiSpecFileName {
    ident: ApiIdent,
    kind: ApiSpecFileNameKind,
}

impl fmt::Display for ApiSpecFileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.path().as_str())
    }
}

impl ApiSpecFileName {
    // Only used by the OpenAPI manager -- not part of the public API.
    #[doc(hidden)]
    pub fn new(ident: ApiIdent, kind: ApiSpecFileNameKind) -> ApiSpecFileName {
        ApiSpecFileName { ident, kind }
    }

    pub fn ident(&self) -> &ApiIdent {
        &self.ident
    }

    pub fn kind(&self) -> &ApiSpecFileNameKind {
        &self.kind
    }

    /// Returns the path of this file relative to the root of the OpenAPI
    /// documents
    pub fn path(&self) -> Utf8PathBuf {
        match &self.kind {
            ApiSpecFileNameKind::Lockstep => {
                Utf8PathBuf::from_iter([self.basename()])
            }
            ApiSpecFileNameKind::Versioned { .. } => Utf8PathBuf::from_iter([
                self.ident.deref().clone(),
                self.basename(),
            ]),
        }
    }

    /// Returns the base name of this file path
    pub fn basename(&self) -> String {
        match &self.kind {
            ApiSpecFileNameKind::Lockstep => format!("{}.json", self.ident),
            ApiSpecFileNameKind::Versioned { version, hash } => {
                format!("{}-{}-{}.json", self.ident, version, hash)
            }
        }
    }

    /// For versioned APIs, returns the hash part of the filename
    pub fn hash(&self) -> Option<&str> {
        match &self.kind {
            ApiSpecFileNameKind::Lockstep => None,
            ApiSpecFileNameKind::Versioned { hash, .. } => Some(hash),
        }
    }
}

/// Describes how a particular OpenAPI document is named.
#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum ApiSpecFileNameKind {
    /// The file's path implies a lockstep API.
    Lockstep,
    /// The file's path implies a versioned API.
    Versioned {
        /// The version of the API this document describes.
        version: semver::Version,
        /// The hash of the file contents.
        hash: String,
    },
}

/// Newtype for API identifiers
#[derive(Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct ApiIdent(String);

impl fmt::Debug for ApiIdent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Deref for ApiIdent {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for ApiIdent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<String> for ApiIdent {
    fn from(value: String) -> Self {
        Self(value)
    }
}

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
