// Copyright 2025 Oxide Computer Company

use std::fmt;

/// Optional metadata that's part of a `ManagedApiConfig`.
#[derive(Clone, Debug, Default)]
pub struct ManagedApiMetadata {
    /// human-readable description of the API (goes into OpenAPI document)
    pub description: Option<&'static str>,

    /// the contact URL for the API
    pub contact_url: Option<&'static str>,

    /// the contact email for the API
    pub contact_email: Option<&'static str>,
}

/// Whether an API is "internal" or "external" to the system.
///
/// This is not interpreted by the OpenAPI manager itself, but it can be useful
/// for determining what kind of validation to pursue for a given API.
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
