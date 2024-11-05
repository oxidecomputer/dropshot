// Copyright 2023 Oxide Computer Company

//! Extractors: traits and impls
//!
//! See top-level crate documentation for details

mod common;
pub use common::ExclusiveExtractor;
pub use common::ExtractorMetadata;
pub use common::RequestExtractor;
pub use common::SharedExtractor;

mod body;
pub use body::MultipartBody;
pub use body::MultipartBodyError;
pub use body::StreamingBody;
pub use body::StreamingBodyError;
pub use body::TypedBody;
pub use body::TypedBodyError;
pub use body::UntypedBody;

mod metadata;

mod path;
pub use path::Path;

mod query;
pub use query::Query;
pub use query::QueryError;

mod raw_request;
pub use raw_request::RawRequest;

use crate::error::HttpError;
pub use crate::http_util::PathError;

/// Errors returned by extractors.
///
/// Because extractors can be combined (a tuple of types which are extractors is
/// itself an extractor), all extractors must return the same error type. Thus,
/// this is an enum of all the possible errors that can be returned by any
/// extractor.
#[derive(Debug, thiserror::Error)]
pub enum ExtractorError {
    /// Errors returned by the [`MultipartBody`] extractor.
    #[error(transparent)]
    MultipartBody(#[from] MultipartBodyError),
    /// Errors returned by the [`StreamingBody`] extractor.
    #[error(transparent)]
    StreamingBody(#[from] StreamingBodyError),
    /// Errors returned by the [`TypedBody`] extractor.
    #[error(transparent)]
    TypedBody(#[from] TypedBodyError),
    /// Errors returned by the [`Path`] extractor.
    #[error(transparent)]
    PathParams(#[from] PathError),
    /// Errors returned by the [`Query`] extractor.
    #[error(transparent)]
    QueryParams(#[from] QueryError),
}

impl From<ExtractorError> for HttpError {
    fn from(error: ExtractorError) -> Self {
        match error {
            ExtractorError::MultipartBody(e) => e.into(),
            ExtractorError::StreamingBody(e) => e.into(),
            ExtractorError::TypedBody(e) => e.into(),
            ExtractorError::PathParams(e) => e.into(),
            ExtractorError::QueryParams(e) => e.into(),
        }
    }
}
