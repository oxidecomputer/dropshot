// Copyright 2023 Oxide Computer Company

use crate::api_description::ApiEndpointParameter;
use crate::api_description::{ApiEndpointBodyContentType, ExtensionMode};
use crate::error::HttpError;
use crate::server::ServerContext;
use crate::RequestContext;

use async_trait::async_trait;

/// Metadata associated with an extractor including parameters and whether or not
/// the associated endpoint is paginated.
pub struct ExtractorMetadata {
    pub extension_mode: ExtensionMode,
    pub parameters: Vec<ApiEndpointParameter>,
}

/// Extractors that require exclusive access to the underyling `hyper::Request`
///
/// These extractors usually need to read the body of the request or else modify
/// how the server treats the rest of it (e.g., websocket upgrade).  There may
/// be at most one of these associated with any request.
#[async_trait]
pub trait ExclusiveExtractor: Send + Sync + Sized {
    /// Construct an instance of this type from a `RequestContext`.
    async fn from_request<Context: ServerContext>(
        rqctx: &RequestContext<Context>,
        request: hyper::Request<crate::Body>,
    ) -> Result<Self, HttpError>;

    fn metadata(
        body_content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata;
}

/// Extractors that do _not_ require exclusive access to the underyling
/// `hyper::Request`
///
/// These extractors usually look at immutable properties of the request that
/// are known up front, like the URL.  There may be any number of these
/// associated with any request.
#[async_trait]
pub trait SharedExtractor: Send + Sync + Sized {
    /// Construct an instance of this type from a `RequestContext`.
    async fn from_request<Context: ServerContext>(
        rqctx: &RequestContext<Context>,
    ) -> Result<Self, HttpError>;

    fn metadata(
        body_content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata;
}

// A `SharedExtractor` can always be treated like an `ExclusiveExtractor`.
#[async_trait]
impl<S: SharedExtractor> ExclusiveExtractor for S {
    async fn from_request<Context: ServerContext>(
        rqctx: &RequestContext<Context>,
        _request: hyper::Request<crate::Body>,
    ) -> Result<Self, HttpError> {
        <S as SharedExtractor>::from_request(rqctx).await
    }

    fn metadata(
        body_content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata {
        <S as SharedExtractor>::metadata(body_content_type)
    }
}

/// Top-level extractor for a given request
///
/// During request handling, we must find and invoke the appropriate
/// consumer-defined handler function.  While each of these functions takes a
/// fixed number of arguments, different handler functions may take a different
/// number of arguments.  The arguments that can vary between handler functions
/// are all extractors, meaning that they impl `SharedExtractor` or
/// `ExclusiveExtractor`.
///
/// This trait helps us invoke various handler functions uniformly, despite them
/// accepting different arguments.  To achieve this, we impl this trait for all
/// supported _tuples_ of argument types, which is essentially 0 or more
/// `SharedExtractor`s followed by at most one `ExclusiveExtractor`.  This impl
/// essentially does the same thing as any other extractor, and it does it by
/// delegating to the impls of each tuple member.
///
/// In practice, the trait `RequestExtractor` is identical to
/// `ExclusiveExtractor` and we could use `ExclusiveExtractor` directly.  But
/// it's clearer to use distinct types, since they're used differently.  To
/// summarize: `RequestExtractor` is private, only implemented on tuple types,
/// and only used to kick off extraction from the top level.
/// `ExclusiveExtractor` s public, implementing types can be consumer-defined,
/// and it would generally not be implemented on tuple types.
#[async_trait]
pub trait RequestExtractor: Send + Sync + Sized {
    /// Construct an instance of this type from a `RequestContext`.
    async fn from_request<Context: ServerContext>(
        rqctx: &RequestContext<Context>,
        request: hyper::Request<crate::Body>,
    ) -> Result<Self, HttpError>;

    fn metadata(
        body_content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata;
}

// Impl for zero-element tuple (used for request handlers with no extractors)
#[async_trait]
impl RequestExtractor for () {
    async fn from_request<Context: ServerContext>(
        _rqctx: &RequestContext<Context>,
        _request: hyper::Request<crate::Body>,
    ) -> Result<Self, HttpError> {
        Ok(())
    }

    fn metadata(
        _body_content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata {
        ExtractorMetadata {
            extension_mode: ExtensionMode::None,
            parameters: vec![],
        }
    }
}

// Impl for one-element tuple with an exclusive extractor
#[async_trait]
impl<X: ExclusiveExtractor + 'static> RequestExtractor for (X,) {
    async fn from_request<Context: ServerContext>(
        rqctx: &RequestContext<Context>,
        request: hyper::Request<crate::Body>,
    ) -> Result<Self, HttpError> {
        Ok((X::from_request(rqctx, request).await?,))
    }

    fn metadata(
        body_content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata {
        X::metadata(body_content_type)
    }
}

/// Defines implementations of `RequestExtractor` for tuples of one or more
/// `SharedExtractor` followed by an `ExclusiveExtractor`
///
/// As an example, `impl_rqextractor_for_tuple!(S1, S2)` defines an impl of
/// `RequestExtractor` for tuple `(S1, S2, X)` where `S1: SharedExtractor`,
/// `S2: SharedExtractor`, and `X: ExclusiveExtractor`.  Note that any
/// `SharedExtractor` also impls `ExclusiveExtractor`, so it's not necessary to
/// impl this separately for `(S1, S2, S3)` (and indeed that would not be
/// possible, since it would overlap with the definition for `(S1, S2, X)`, even
/// if `SharedExtractor` did not impl `ExclusiveExtractor`).
macro_rules! impl_rqextractor_for_tuple {
    ($( $S:ident),+) => {

    // impl RequestExtractor for a tuple of shared extractors with an exclusive extractor
    #[async_trait]
    impl< X: ExclusiveExtractor + 'static, $($S: SharedExtractor + 'static,)+ >
        RequestExtractor
        for ($($S,)+ X)
    {
        async fn from_request<Context: ServerContext>(
            rqctx: &RequestContext<Context>,
            request: hyper::Request<crate::Body>
        ) -> Result<( $($S,)+ X ), HttpError>
        {
            futures::try_join!(
                $($S::from_request(rqctx),)+
                X::from_request(rqctx, request)
            )
        }

        fn metadata(_body_content_type: ApiEndpointBodyContentType) -> ExtractorMetadata {
            #[allow(unused_mut)]
            let mut extension_mode = ExtensionMode::None;
            #[allow(unused_mut)]
            let mut parameters = vec![];
            $(
                let mut metadata = $S::metadata(_body_content_type.clone());
                extension_mode = match (extension_mode, metadata.extension_mode) {
                    (ExtensionMode::None, x) | (x, ExtensionMode::None) => x,
                    (x, y) if x != y => {
                        panic!("incompatible extension modes in tuple: {:?} != {:?}", x, y);
                    }
                    (_, x) => x,
                };
                parameters.append(&mut metadata.parameters);
            )+

            let mut metadata = X::metadata(_body_content_type.clone());
            extension_mode = match (extension_mode, metadata.extension_mode) {
                (ExtensionMode::None, x) | (x, ExtensionMode::None) => x,
                (x, y) if x != y => {
                    panic!("incompatible extension modes in tuple: {:?} != {:?}", x, y);
                }
                (_, x) => x,
            };
            parameters.append(&mut metadata.parameters);

            ExtractorMetadata { extension_mode, parameters }
        }
    }
}}

// Implement `RequestExtractor` for any tuple consisting of 0-2 shared
// extractors and exactly one exclusive extractor.
impl_rqextractor_for_tuple!(S1);
impl_rqextractor_for_tuple!(S1, S2);
