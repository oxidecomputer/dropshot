// Copyright 2025 Oxide Computer Company

use std::fmt;

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
