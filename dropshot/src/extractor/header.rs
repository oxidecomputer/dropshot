// Copyright 2026 Oxide Computer Company

//! Header extractor

use std::collections::BTreeMap;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;

use crate::{
    from_map::from_map_insensitive, ApiEndpointBodyContentType,
    ApiEndpointParameterLocation, HttpError, RequestContext, RequestInfo,
    ServerContext,
};

use super::{metadata::get_metadata, ExtractorMetadata, SharedExtractor};

/// `Header<HeaderType>` is an extractor used to deserialize an instance of
/// `HeaderType` from an HTTP request's header values. `HeaderType` may be any
/// structure that implements [serde::Deserialize] and [schemars::JsonSchema].
/// While headers are always accessible through [RequestInfo::headers] even
/// without this extractor, using this extractor causes header inputs to be
/// documented in OpenAPI output.
/// See the top-level crate documentation for more information.
///
/// Note that (unlike the [`Query`] and [`Path`] extractors) headers are case-
/// insensitive. You may rename fields with mixed casing (e.g. by using
/// #[serde(rename = "X-Header-Foo")]) and that casing will appear in the
/// OpenAPI document output. Name conflicts (including names differentiated by
/// casing since headers are case-insensitive) may lead to unexpected behavior,
/// and should be avoided. For example, only one of the conflicting fields may
/// be deserialized, and therefore deserialization may fail if any conflicting
/// field is required (i.e. not an `Option<T>` type)
#[derive(Debug)]
pub struct Header<HeaderType: DeserializeOwned + JsonSchema + Send + Sync> {
    inner: HeaderType,
}

impl<HeaderType: DeserializeOwned + JsonSchema + Send + Sync>
    Header<HeaderType>
{
    pub fn into_inner(self) -> HeaderType {
        self.inner
    }

    /// Convert this `Header` into one with a different type parameter; this
    /// may be useful when multiple, related endpoints take header parameters
    /// that are similar and convertible into a common type.
    pub fn map<T, F>(self, f: F) -> Header<T>
    where
        T: DeserializeOwned + JsonSchema + Send + Sync,
        F: FnOnce(HeaderType) -> T,
    {
        Header { inner: f(self.inner) }
    }

    /// Similar to [`Header::map`] but with support for fallibility.
    pub fn try_map<T, E, F>(self, f: F) -> Result<Header<T>, E>
    where
        T: DeserializeOwned + JsonSchema + Send + Sync,
        F: FnOnce(HeaderType) -> Result<T, E>,
    {
        Ok(Header { inner: f(self.inner)? })
    }
}

impl<HeaderType: DeserializeOwned + JsonSchema + Send + Sync> From<HeaderType>
    for Header<HeaderType>
{
    fn from(value: HeaderType) -> Self {
        Self { inner: value }
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
    let inner = from_map_insensitive(&headers).map_err(|message| {
        HttpError::for_bad_request(
            None,
            format!("error processing headers: {message}"),
        )
    })?;
    Ok(Header { inner })
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

#[cfg(test)]
mod tests {
    use schemars::JsonSchema;
    use serde::Deserialize;

    use crate::{extractor::header::http_request_load_header, RequestInfo};

    #[test]
    fn test_header_parsing() {
        #[allow(dead_code)]
        #[derive(Debug, Deserialize, JsonSchema)]
        pub struct TestHeaders {
            header_a: String,
            #[serde(rename = "X-Header-B")]
            header_b: String,
        }

        let addr = std::net::SocketAddr::new(
            std::net::Ipv4Addr::LOCALHOST.into(),
            8080,
        );

        let request =
            hyper::Request::builder().uri("http://localhost").body(()).unwrap();
        let info = RequestInfo::new(&request, addr);

        let parsed = http_request_load_header::<TestHeaders>(&info);
        assert!(parsed.is_err());

        let request = hyper::Request::builder()
            .header("header_a", "header_a value")
            .header("X-Header-B", "header_b value")
            .uri("http://localhost")
            .body(())
            .unwrap();
        let info = RequestInfo::new(&request, addr);

        let parsed = http_request_load_header::<TestHeaders>(&info);

        match parsed {
            Ok(headers) => {
                assert_eq!(headers.inner.header_a, "header_a value");
                assert_eq!(headers.inner.header_b, "header_b value");
            }
            Err(e) => {
                panic!("unexpected error: {}", e);
            }
        }

        let request = hyper::Request::builder()
            .header("header_a", "header_a value")
            .header("X-hEaDEr-b", "header_b value")
            .uri("http://localhost")
            .body(())
            .unwrap();
        let info = RequestInfo::new(&request, addr);

        let parsed = http_request_load_header::<TestHeaders>(&info);

        match parsed {
            Ok(headers) => {
                assert_eq!(headers.inner.header_a, "header_a value");
                assert_eq!(headers.inner.header_b, "header_b value");
            }
            Err(e) => {
                panic!("unexpected error: {}", e);
            }
        }
    }
}
