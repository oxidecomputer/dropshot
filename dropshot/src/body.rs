// Copyright 2024 Oxide Computer Company

use std::pin::Pin;
use std::task::{Context, Poll};

use http_body_util::BodyExt;
use http_body_util::combinators::BoxBody;
use hyper::body::{Body as HttpBody, Bytes, Frame};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A body type for both requests and responses in Dropshot.
#[derive(Debug)]
pub struct Body {
    inner: BoxBody<Bytes, BoxError>,
}

#[derive(Debug)]
pub(crate) struct DataStream(Body);

impl Body {
    /// Create an empty body.
    pub fn empty() -> Self {
        let inner = http_body_util::Empty::new()
            .map_err(|never| match never {})
            .boxed();
        Body { inner }
    }

    /// Create a body with content from a specific buffer.
    pub fn with_content(buf: impl Into<Bytes>) -> Self {
        let inner = http_body_util::Full::new(buf.into())
            .map_err(|never| match never {})
            .boxed();
        Body { inner }
    }

    /// Wrap any body as a dropshot Body.
    pub fn wrap<B>(under: B) -> Self
    where
        B: HttpBody<Data = Bytes> + Send + Sync + 'static,
        B::Error: Into<BoxError>,
    {
        let inner = under.map_err(Into::into).boxed();
        Body { inner }
    }

    /// Converts this body into an `impl Stream` of only the data frames.
    pub(crate) fn into_data_stream(self) -> DataStream {
        DataStream(self)
    }
}

impl Default for Body {
    fn default() -> Body {
        Body::empty()
    }
}

impl From<Bytes> for Body {
    fn from(b: Bytes) -> Body {
        Body::with_content(b)
    }
}

impl From<Vec<u8>> for Body {
    fn from(s: Vec<u8>) -> Body {
        Body::with_content(s)
    }
}

impl From<String> for Body {
    fn from(s: String) -> Body {
        Body::with_content(s)
    }
}

impl From<&'static str> for Body {
    fn from(s: &'static str) -> Body {
        Body::with_content(s)
    }
}

impl HttpBody for Body {
    type Data = Bytes;
    type Error = BoxError;

    #[inline]
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.inner).poll_frame(cx)
    }

    #[inline]
    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner.size_hint()
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
}

impl futures::Stream for DataStream {
    type Item = Result<Bytes, BoxError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        loop {
            match futures::ready!(Pin::new(&mut self.0).poll_frame(cx)?) {
                Some(frame) => match frame.into_data() {
                    Ok(data) => return Poll::Ready(Some(Ok(data))),
                    Err(_frame) => {}
                },
                None => return Poll::Ready(None),
            }
        }
    }
}
