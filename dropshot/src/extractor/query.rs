// Copyright 2023 Oxide Computer Company

//! Querystring-related extractor(s)

use super::metadata::get_metadata;
use super::ExtractorError;
use crate::api_description::ApiEndpointBodyContentType;
use crate::api_description::ApiEndpointParameterLocation;
use crate::error::HttpError;
use crate::server::ServerContext;
use crate::ExtractorMetadata;
use crate::RequestContext;
use crate::RequestInfo;
use crate::SharedExtractor;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use std::fmt::Debug;

/// `Query<QueryType>` is an extractor used to deserialize an instance of
/// `QueryType` from an HTTP request's query string.  `QueryType` is any
/// structure of yours that implements `serde::Deserialize`.  See this module's
/// documentation for more information.
#[derive(Debug)]
pub struct Query<QueryType: DeserializeOwned + JsonSchema + Send + Sync> {
    inner: QueryType,
}

impl<QueryType: DeserializeOwned + JsonSchema + Send + Sync> Query<QueryType> {
    // TODO drop this in favor of Deref?  + Display and Debug for convenience?
    pub fn into_inner(self) -> QueryType {
        self.inner
    }
}

/// Errors returned by the [`Query`] extractor.
#[derive(Debug, thiserror::Error)]
#[error("unable to parse query string: {0}")]
pub struct QueryError(#[from] serde_urlencoded::de::Error);

impl From<QueryError> for HttpError {
    fn from(error: QueryError) -> Self {
        HttpError::for_client_error(
            None,
            error.recommended_status_code(),
            error.to_string(),
        )
    }
}

impl QueryError {
    /// Returns the recommended status code for this error.
    ///
    /// This can be used when constructing a HTTP response for this error. These
    /// are the status codes used by the `From<QueryError>` implementation
    /// for [`HttpError`].
    pub fn recommended_status_code(&self) -> http::StatusCode {
        http::StatusCode::BAD_REQUEST
    }
}

/// Given an HTTP request, pull out the query string and attempt to deserialize
/// it as an instance of `QueryType`.
fn http_request_load_query<QueryType>(
    request: &RequestInfo,
) -> Result<Query<QueryType>, QueryError>
where
    QueryType: DeserializeOwned + JsonSchema + Send + Sync,
{
    let raw_query_string = request.uri().query().unwrap_or("");
    // TODO-correctness: are query strings defined to be urlencoded in this way?
    let q = serde_urlencoded::from_str(raw_query_string)?;
    Ok(Query { inner: q })
}

// The `SharedExtractor` implementation for Query<QueryType> describes how to
// construct an instance of `Query<QueryType>` from an HTTP request: namely, by
// parsing the query string to an instance of `QueryType`.
// TODO-cleanup We shouldn't have to use the "'static" bound on `QueryType`
// here.  It seems like we ought to be able to use 'async_trait, but that
// doesn't seem to be defined.
#[async_trait]
impl<QueryType> SharedExtractor for Query<QueryType>
where
    QueryType: JsonSchema + DeserializeOwned + Send + Sync + 'static,
{
    async fn from_request<Context: ServerContext>(
        rqctx: &RequestContext<Context>,
    ) -> Result<Query<QueryType>, ExtractorError> {
        http_request_load_query(&rqctx.request).map_err(Into::into)
    }

    fn metadata(
        _body_content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata {
        get_metadata::<QueryType>(&ApiEndpointParameterLocation::Query)
    }
}
