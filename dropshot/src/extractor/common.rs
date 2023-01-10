// Copyright 2022 Oxide Computer Company

// XXX-dap TODO-cleanup should the metadata into a separate, shared trait?

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
/// During request handling, we wind up needing to call a function with a
/// variable number of arguments whose types are all extractors (either
/// `SharedExtractor` or `ExclusiveExtractor`).  We achieve this with a separate
/// type called `RequestExtractor` that looks just like `ExclusiveExtractor`.
/// We can impl this trait on a tuple of any number of types that themselves
/// impl `SharedExtractor` or `ExclusiveExtractor` by delegating to each type's
/// extractor implementation.  There may be at most one `ExclusiveExtractor` in
/// the tuple.  We require it to be the last argument just to avoid having to
/// define the power set of impls.
///
/// In practice, `RequestExtractor` is identical to `ExclusiveExtractor`.  But
/// we use them in different ways.  `RequestExtractor` is private, only
/// implemented on tuple types, and only used to kick off extraction.
/// `ExclusiveExtractor` can be consumer-defined and would generally not be
/// implemented on tuple types.
#[async_trait]
pub trait RequestExtractor: Send + Sync + Sized {
    /// Construct an instance of this type from a `RequestContext`.
    async fn from_request<Context: ServerContext>(
        rqctx: &RequestContext<Context>,
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
    ) -> Result<Self, HttpError> {
        Ok((X::from_request(rqctx).await?,))
    }

    fn metadata(
        body_content_type: ApiEndpointBodyContentType,
    ) -> ExtractorMetadata {
        X::metadata(body_content_type)
    }
}

// XXX-dap TODO-doc update comment based on the change that uses the fact that
// SharedExtractor impls ExclusiveExtractor such that the last item in the
// tuple *must* be an exclusive extractor
/// Defines implementations of `RequestExtractor` for tuples of one or more
/// `SharedExtractor` followed by an `ExclusiveExtractor`
///
/// As an example, `impl_rqextractor_for_tuple!(S1, S2)` defines an impl of
/// `RequestExtractor` for tuple `(S1, S2, X)` where `S1: SharedExtractor`,
/// `S2: SharedExtractor`, and `X: ExclusiveExtractor`, as well as a similar
/// impl for just `(S1, S2)`.
macro_rules! impl_rqextractor_for_tuple {
    ($( $S:ident),+) => {

    // impl RequestExtractor for a tuple of shared extractors with an exclusive extractor
    #[async_trait]
    impl< X: ExclusiveExtractor + 'static, $($S: SharedExtractor + 'static,)+ >
        RequestExtractor
        for ($($S,)+ X)
    {
        async fn from_request<Context: ServerContext>(rqctx: &RequestContext<Context>)
            -> Result<( $($S,)+ X ), HttpError>
        {
            futures::try_join!(
                $($S::from_request(rqctx),)+
                X::from_request(rqctx)
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
