// Copyright 2025 Oxide Computer Company

//! Types used by trait-based API definitions to define the versions that they
//! support.

use std::collections::BTreeMap;

#[derive(Debug)]
pub struct SupportedVersion {
    semver: semver::Version,
    label: &'static str,
}

impl SupportedVersion {
    pub const fn new(
        semver: semver::Version,
        label: &'static str,
    ) -> SupportedVersion {
        SupportedVersion { semver, label }
    }

    pub fn semver(&self) -> &semver::Version {
        &self.semver
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

#[derive(Debug)]
pub struct SupportedVersions {
    versions: Vec<SupportedVersion>,
}

impl SupportedVersions {
    #[track_caller]
    pub fn new(versions: Vec<SupportedVersion>) -> SupportedVersions {
        assert!(
            !versions.is_empty(),
            "at least one version of an API must be supported"
        );

        // We require that the list of supported versions for an API be sorted
        // because this helps ensure a git conflict when two people attempt to
        // add or modify the same version in different branches.
        assert!(
            versions.iter().map(|v| v.semver()).is_sorted(),
            "list of supported versions for an API must be sorted"
        );

        // Each semver and each label must be unique.
        let mut unique_versions = BTreeMap::new();
        let mut unique_labels = BTreeMap::new();
        for v in &versions {
            if let Some(previous) =
                unique_versions.insert(v.semver(), v.label())
            {
                panic!(
                    "version {} appears multiple times (labels: {:?}, {:?})",
                    v.semver(),
                    previous,
                    v.label()
                );
            }

            if let Some(previous) = unique_labels.insert(v.label(), v.semver())
            {
                panic!(
                    "label {:?} appears multiple times (versions: {}, {})",
                    v.label(),
                    previous,
                    v.semver()
                );
            }
        }

        SupportedVersions { versions }
    }

    pub fn iter(&self) -> impl Iterator<Item = &'_ SupportedVersion> + '_ {
        self.versions.iter()
    }
}

/// Helper macro used to define API versions
///
/// ```
/// use openapi_manager_types::{
///     api_versions,
///     SupportedVersion,
///     SupportedVersions
/// };
///
/// api_versions!([
///     (2, ADD_FOOBAR_OPERATION),
///     (1, INITIAL),
/// ]);
/// ```
///
/// This example says that there are two API versions: `1.0.0` (the initial
/// version) and `2.0.0` (which adds an operation called "foobar").  This macro
/// invocation defines symbolic constants of type `semver::Version` for each of
/// these, equivalent to:
///
/// ```
///     pub const VERSION_ADD_FOOBAR_OPERATION: semver::Version =
///         semver::Version::new(2, 0, 0);
///     pub const VERSION_INITIAL: semver::Version =
///         semver::Version::new(1, 0, 0);
/// ```
///
/// It also defines a function called `pub fn supported_versions() ->
/// SupportedVersions` that, as the name suggests, returns a
/// [`SupportedVersions`] that describes these two supported API versions.
// Design constraints:
// - For each new API version, we need a developer-chosen semver and label that
//   can be used to construct an identifier.
// - We want to produce:
//   - a symbolic constant for each version that won't change if the developer
//     needs to change the semver value for this API version
//   - a list of supported API versions
// - Critically, we want to ensure that if two developers both add new API
//   versions in separate branches, whether or not they choose the same value,
//   there must be a git conflict that requires manual resolution.
//   - To achieve this, we put the list of versions in a list.
// - We want to make it hard to do this merge wrong without noticing.
//   - We want to require that the list be sorted (so that someone hasn't put
//     something in the wrong order).
//   - The list should have no duplicates.
// - We want to minimize boilerplate.
//
// That's how we've landed on defining API versions using this macro where:
// - each API definition is simple and fits on a single line
// - there will necessarily be a conflict if two people try to add a line in the
//   same spot of the file, even if they overlap, assuming they choose different
//   labels for their API version
// - the consumer of this value will be able to do those checks that help make
//   sure there wasn't a mismerge.
#[macro_export]
macro_rules! api_versions {
    ( [ $( (
        $major:literal,
        $name:ident
    ) ),* $(,)? ] ) => {
        openapi_manager_types::paste! {
            $(
                pub const [<VERSION_ $name>]: semver::Version =
                    semver::Version::new($major, 0, 0);
            )*

            pub fn supported_versions() -> SupportedVersions {
                let mut literal_versions = vec![
                    $( SupportedVersion::new([<VERSION_ $name>], stringify!($name)) ),*
                ];
                literal_versions.reverse();
                SupportedVersions::new(literal_versions)
            }
        }
    };
}

/// "picky" version of `api_versions` that lets you specify the minor and patch
/// numbers, too
///
/// It is not yet clear why we'd ever need this.  Our approach to versioning is
/// oriented around not having to care whether a change is a major bump or not
/// so we can just always bump the major number.
#[macro_export]
macro_rules! api_versions_picky {
    ( [ $( (
        $major:literal,
        $minor:literal,
        $patch:literal,
        $name:ident
    ) ),* $(,)? ] ) => {
        openapi_manager_types::paste! {
            $(
                pub const [<VERSION_ $name>]: semver::Version =
                    semver::Version::new($major, $minor, $patch);
            )*

            #[track_caller]
            pub fn supported_versions() -> SupportedVersions {
                let mut literal_versions = vec![
                    $( SupportedVersion::new([<VERSION_ $name>], $desc) ),*
                ];
                literal_versions.reverse();
                SupportedVersions::new(literal_versions)
            }
        }
    };
}
