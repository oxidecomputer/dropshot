// Copyright 2025 Oxide Computer Company

/// Optional metadata that's part of a `ManagedApiConfig`.
#[derive(Clone, Debug, Default)]
pub struct ManagedApiMetadata {
    /// human-readable description of the API (goes into OpenAPI document)
    pub description: Option<&'static str>,

    /// the contact URL for the API (goes into OpenAPI document)
    pub contact_url: Option<&'static str>,

    /// the contact email for the API (goes into OpenAPI document)
    pub contact_email: Option<&'static str>,

    /// extra, dynamically-typed metadata for internal use
    pub extra: serde_json::Value,
}
