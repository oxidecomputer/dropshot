// Copyright 2026 Oxide Computer Company

//! Querystring-related extractor(s)

use super::metadata::get_metadata;
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
/// `QueryType` from an HTTP request's query string.  `QueryType` may be any
/// struct of yours that implements [serde::Deserialize] and
/// [schemars::JsonSchema].
/// See the top-level crate documentation for more information.
#[derive(Debug)]
pub struct Query<QueryType: DeserializeOwned + JsonSchema + Send + Sync> {
    inner: QueryType,
}

impl<QueryType: DeserializeOwned + JsonSchema + Send + Sync> Query<QueryType> {
    // TODO drop this in favor of Deref?  + Display and Debug for convenience?
    pub fn into_inner(self) -> QueryType {
        self.inner
    }

    /// Convert this `Query` into one with a different type parameter; this
    /// may be useful when multiple, related endpoints take query parameters
    /// that are similar and convertible into a common type.
    pub fn map<T, F>(self, f: F) -> Query<T>
    where
        T: DeserializeOwned + JsonSchema + Send + Sync,
        F: FnOnce(QueryType) -> T,
    {
        Query { inner: f(self.inner) }
    }

    /// Similar to [`Query::map`] but with support for fallibility.
    pub fn try_map<T, E, F>(self, f: F) -> Result<Query<T>, E>
    where
        T: DeserializeOwned + JsonSchema + Send + Sync,
        F: FnOnce(QueryType) -> Result<T, E>,
    {
        Ok(Query { inner: f(self.inner)? })
    }
}

impl<QueryType: DeserializeOwned + JsonSchema + Send + Sync> From<QueryType>
    for Query<QueryType>
{
    fn from(value: QueryType) -> Self {
        Self { inner: value }
    }
}

/// Given an HTTP request, pull out the query string and attempt to deserialize
/// it as an instance of `QueryType`.
fn http_request_load_query<QueryType>(
    request: &RequestInfo,
) -> Result<Query<QueryType>, HttpError>
where
    QueryType: DeserializeOwned + JsonSchema + Send + Sync,
{
    let raw_query_string = request.uri().query().unwrap_or("");
    // TODO-correctness: are query strings defined to be urlencoded in this way?
    match serde_urlencoded::from_str(raw_query_string) {
        Ok(q) => Ok(Query { inner: q }),
        Err(e) => Err(HttpError::for_bad_request(
            None,
            format!("unable to parse query string: {}", e),
        )),
    }
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
    ) -> Result<Query<QueryType>, HttpError> {
        http_request_load_query(&rqctx.request)
    }

    fn metadata(
        _body_content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata {
        get_metadata::<QueryType>(&ApiEndpointParameterLocation::Query)
    }
}
