// Copyright 2025 Oxide Computer Company

//! URL-related extractor(s)

use super::metadata::get_metadata;
use crate::api_description::ApiEndpointBodyContentType;
use crate::api_description::ApiEndpointParameterLocation;
use crate::error::HttpError;
use crate::http_util::http_extract_path_params;
use crate::server::ServerContext;
use crate::ExtractorMetadata;
use crate::RequestContext;
use crate::SharedExtractor;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use std::fmt::Debug;

/// `Path<PathType>` is an extractor used to deserialize an instance of
/// `PathType` from an HTTP request's path parameters.  `PathType` may be any
/// struct of yours that implements [serde::Deserialize] and
/// [schemars::JsonSchema].
/// See the top-level crate documentation for more information.
#[derive(Debug)]
pub struct Path<PathType: JsonSchema + Send + Sync> {
    inner: PathType,
}

impl<PathType: JsonSchema + Send + Sync> Path<PathType> {
    // TODO drop this in favor of Deref?  + Display and Debug for convenience?
    pub fn into_inner(self) -> PathType {
        self.inner
    }

    /// Convert this `Path` into one with a different type parameter; this
    /// may be useful when multiple, related endpoints take path parameters that
    /// are similar and convertible into a common type.
    pub fn map<T, F>(self, f: F) -> Path<T>
    where
        T: JsonSchema + Send + Sync,
        F: FnOnce(PathType) -> T,
    {
        Path { inner: f(self.inner) }
    }

    /// Similar to [`Path::map`] but with support for fallibility.
    pub fn try_map<T, E, F>(self, f: F) -> Result<Path<T>, E>
    where
        T: JsonSchema + Send + Sync,
        F: FnOnce(PathType) -> Result<T, E>,
    {
        Ok(Path { inner: f(self.inner)? })
    }
}

// The `SharedExtractor` implementation for Path<PathType> describes how to
// construct an instance of `Path<QueryType>` from an HTTP request: namely, by
// extracting parameters from the query string.
#[async_trait]
impl<PathType> SharedExtractor for Path<PathType>
where
    PathType: DeserializeOwned + JsonSchema + Send + Sync + 'static,
{
    async fn from_request<Context: ServerContext>(
        rqctx: &RequestContext<Context>,
    ) -> Result<Path<PathType>, HttpError> {
        let params: PathType =
            http_extract_path_params(&rqctx.endpoint.variables)?;
        Ok(Path { inner: params })
    }

    fn metadata(
        _body_content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata {
        get_metadata::<PathType>(&ApiEndpointParameterLocation::Path)
    }
}
