// Copyright 2025 Oxide Computer Company

//! Header extractor

use std::collections::BTreeMap;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;

use crate::{
    from_map::from_map, ApiEndpointBodyContentType,
    ApiEndpointParameterLocation, HttpError, RequestContext, RequestInfo,
    ServerContext,
};

use super::{metadata::get_metadata, ExtractorMetadata, SharedExtractor};

/// `Header<HeaderType>` is an extractor used to deserialize an instance of
/// `HeaderType` from an HTTP request's header values. `HeaderType` may be any
/// struct of yours that implements [serde::Deserialize] and 
/// [schemars::JsonSchema], where  primitives and enums have to be wrapped in
/// an outer struct and enums need to be flattened using the 
/// `#[serde(flatten)]` attribute.  While headers are accessible through 
/// [RequestInfo::headers], using this extractor in an entrypoint causes header
/// inputs to be documented in OpenAPI output. 
/// See the crate documentation for more information.
pub struct Header<HeaderType: DeserializeOwned + JsonSchema + Send + Sync> {
    inner: HeaderType,
}

impl<HeaderType: DeserializeOwned + JsonSchema + Send + Sync>
    Header<HeaderType>
{
    pub fn into_inner(self) -> HeaderType {
        self.inner
    }
}

/// Given an HTTP request, pull out the headers and attempt to deserialize
/// it as an instance of `HeaderType`.
fn http_request_load_header<HeaderType>(
    request: &RequestInfo,
) -> Result<Header<HeaderType>, HttpError>
where
    HeaderType: DeserializeOwned + JsonSchema + Send + Sync,
{
    let headers = request
        .headers()
        .iter()
        .map(|(k, v)| Ok((k.to_string(), v.to_str()?.to_string())))
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(|message: http::header::ToStrError| {
            HttpError::for_bad_request(None, message.to_string())
        })?;
    let x: HeaderType = from_map(&headers).unwrap();
    Ok(Header { inner: x })
}

#[async_trait]
impl<HeaderType> SharedExtractor for Header<HeaderType>
where
    HeaderType: JsonSchema + DeserializeOwned + Send + Sync + 'static,
{
    async fn from_request<Context: ServerContext>(
        rqctx: &RequestContext<Context>,
    ) -> Result<Header<HeaderType>, HttpError> {
        http_request_load_header(&rqctx.request)
    }

    fn metadata(
        _body_content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata {
        get_metadata::<HeaderType>(&ApiEndpointParameterLocation::Header)
    }
}
