// Copyright 2025 Oxide Computer Company

//! Determine if one OpenAPI spec is a subset of another

use openapiv3::OpenAPI;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("all changes are considered incompatible right now")]
pub struct OpenApiCompatibilityError {}

pub fn api_compatible(
    spec1: &OpenAPI,
    spec2: &OpenAPI,
) -> Vec<OpenApiCompatibilityError> {
    if *spec1 != *spec2 {
        vec![OpenApiCompatibilityError {}]
    } else {
        Vec::new()
    }
}
