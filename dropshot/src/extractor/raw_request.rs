// Copyright 2023 Oxide Computer Company

//! Raw request extractor

use crate::api_description::{ApiEndpointBodyContentType, ExtensionMode};
use crate::error::HttpError;
use crate::server::ServerContext;
use crate::{ExclusiveExtractor, ExtractorMetadata, RequestContext};
use async_trait::async_trait;
use std::fmt::Debug;

/// `RawRequest` is an extractor providing access to the raw underlying
/// [`hyper::Request`].
#[derive(Debug)]
pub struct RawRequest {
    request: hyper::Request<crate::Body>,
}

impl RawRequest {
    pub fn into_inner(self) -> hyper::Request<crate::Body> {
        self.request
    }
}

#[async_trait]
impl ExclusiveExtractor for RawRequest {
    async fn from_request<Context: ServerContext>(
        _rqctx: &RequestContext<Context>,
        request: hyper::Request<crate::Body>,
    ) -> Result<RawRequest, HttpError> {
        Ok(RawRequest { request })
    }

    fn metadata(
        _content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata {
        ExtractorMetadata {
            parameters: vec![],
            extension_mode: ExtensionMode::None,
        }
    }
}
