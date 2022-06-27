//! Websocket specific functions.

use std::{
    borrow::Cow,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{future, ready, FutureExt, Sink, Stream};
use tokio_tungstenite::{tungstenite::protocol, WebSocketStream};

use crate::HttpError;

/// A websocket `Stream` and `Sink`.
///
/// Ping messages sent from the client will be handled internally by replying with a Pong message.
/// Close messages need to be handled explicitly: usually by closing the `Sink` end of the
/// `WebSocket`.
///
/// **Note!**
/// Due to rust futures nature, pings won't be handled until read part of `WebSocket` is polled
pub struct WebSocket {
    inner: WebSocketStream<hyper::upgrade::Upgraded>,
}

impl WebSocket {
    pub async fn from_raw_socket(
        upgraded: hyper::upgrade::Upgraded,
        role: protocol::Role,
        config: Option<protocol::WebSocketConfig>,
    ) -> Self {
        WebSocketStream::from_raw_socket(upgraded, role, config)
            .map(|inner| WebSocket { inner })
            .await
    }

    /// Gracefully close this websocket.
    pub async fn close(mut self) -> Result<(), HttpError> {
        future::poll_fn(|cx| Pin::new(&mut self).poll_close(cx)).await
    }
}

impl Stream for WebSocket {
    type Item = Result<WebSocketMessage, HttpError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        match ready!(Pin::new(&mut self.inner).poll_next(cx)) {
            Some(Ok(item)) => {
                Poll::Ready(Some(Ok(WebSocketMessage { inner: item })))
            }
            Some(Err(e)) => Poll::Ready(Some(Err(e.into()))),
            None => Poll::Ready(None),
        }
    }
}

impl Sink<WebSocketMessage> for WebSocket {
    type Error = HttpError;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), HttpError>> {
        match ready!(Pin::new(&mut self.inner).poll_ready(cx)) {
            Ok(()) => Poll::Ready(Ok(())),
            Err(e) => Poll::Ready(Err(e.into())),
        }
    }

    fn start_send(
        mut self: Pin<&mut Self>,
        item: WebSocketMessage,
    ) -> Result<(), HttpError> {
        match Pin::new(&mut self.inner).start_send(item.inner) {
            Ok(()) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), HttpError>> {
        match ready!(Pin::new(&mut self.inner).poll_flush(cx)) {
            Ok(()) => Poll::Ready(Ok(())),
            Err(e) => Poll::Ready(Err(e.into())),
        }
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), HttpError>> {
        match ready!(Pin::new(&mut self.inner).poll_close(cx)) {
            Ok(()) => Poll::Ready(Ok(())),
            Err(err) => Poll::Ready(Err(err.into())),
        }
    }
}

impl std::fmt::Debug for WebSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocket").finish()
    }
}

/// A WebSocket message.
#[derive(Eq, PartialEq, Clone)]
pub struct WebSocketMessage {
    inner: protocol::Message,
}

impl WebSocketMessage {
    /// Construct a new Text `Message`.
    pub fn text<S: Into<String>>(s: S) -> Self {
        WebSocketMessage { inner: protocol::Message::text(s) }
    }

    /// Construct a new Json `Message`
    pub fn json<T>(j: T) -> Result<Self, HttpError>
    where
        T: serde::Serialize,
    {
        Ok(Self {
            inner: protocol::Message::text(serde_json::to_string(&j).map_err(
                |e| {
                    HttpError::for_bad_request(
                        None,
                        format!(
                            "unable to serialize string as JSON body: {}",
                            e
                        ),
                    )
                },
            )?),
        })
    }

    /// Construct a new Binary `Message`.
    pub fn binary<V: Into<Vec<u8>>>(v: V) -> Self {
        WebSocketMessage { inner: protocol::Message::binary(v) }
    }

    /// Construct a new Ping `Message`.
    pub fn ping<V: Into<Vec<u8>>>(v: V) -> Self {
        WebSocketMessage { inner: protocol::Message::Ping(v.into()) }
    }

    /// Construct a new Pong `Message`.
    ///
    /// Note that one rarely needs to manually construct a Pong message because the underlying tungstenite socket
    /// automatically responds to the Ping messages it receives. Manual construction might still be useful in some cases
    /// like in tests or to send unidirectional heartbeats.
    pub fn pong<V: Into<Vec<u8>>>(v: V) -> Self {
        WebSocketMessage { inner: protocol::Message::Pong(v.into()) }
    }

    /// Construct the default Close `Message`.
    pub fn close() -> Self {
        WebSocketMessage { inner: protocol::Message::Close(None) }
    }

    /// Construct a Close `Message` with a code and reason.
    pub fn close_with(
        code: impl Into<u16>,
        reason: impl Into<Cow<'static, str>>,
    ) -> Self {
        WebSocketMessage {
            inner: protocol::Message::Close(Some(
                protocol::frame::CloseFrame {
                    code: protocol::frame::coding::CloseCode::from(code.into()),
                    reason: reason.into(),
                },
            )),
        }
    }

    /// Returns true if this message is a Text message.
    pub fn is_text(&self) -> bool {
        self.inner.is_text()
    }

    /// Returns true if this message is a Binary message.
    pub fn is_binary(&self) -> bool {
        self.inner.is_binary()
    }

    /// Returns true if this message a is a Close message.
    pub fn is_close(&self) -> bool {
        self.inner.is_close()
    }

    /// Returns true if this message is a Ping message.
    pub fn is_ping(&self) -> bool {
        self.inner.is_ping()
    }

    /// Returns true if this message is a Pong message.
    pub fn is_pong(&self) -> bool {
        self.inner.is_pong()
    }

    /// Try to get the close frame (close code and reason)
    pub fn close_frame(&self) -> Option<(u16, &str)> {
        if let protocol::Message::Close(Some(ref close_frame)) = self.inner {
            Some((close_frame.code.into(), close_frame.reason.as_ref()))
        } else {
            None
        }
    }

    /// Try to get a reference to the string text, if this is a Text message.
    pub fn to_str(&self) -> Result<&str, HttpError> {
        match self.inner {
            protocol::Message::Text(ref s) => Ok(s),
            _ => Err(HttpError::for_internal_error(
                "websocket message is not a text message".to_string(),
            )),
        }
    }

    /// Return the bytes of this message, if the message can contain data.
    pub fn as_bytes(&self) -> &[u8] {
        match self.inner {
            protocol::Message::Text(ref s) => s.as_bytes(),
            protocol::Message::Binary(ref v) => v,
            protocol::Message::Ping(ref v) => v,
            protocol::Message::Pong(ref v) => v,
            protocol::Message::Frame(ref frame) => frame.payload().as_slice(),
            protocol::Message::Close(_) => &[],
        }
    }

    /// Return the type as decoded json.
    pub fn as_json<T>(&self) -> Result<T, HttpError>
    where
        T: serde::de::DeserializeOwned,
    {
        serde_json::from_slice(self.as_bytes()).map_err(|e| {
            HttpError::for_internal_error(format!(
                "deserializing websocket message as JSON failed: {}",
                e
            ))
        })
    }

    /// Destructure this message into binary data.
    pub fn into_bytes(self) -> Vec<u8> {
        self.inner.into_data()
    }
}

impl std::fmt::Debug for WebSocketMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.inner, f)
    }
}

impl From<WebSocketMessage> for Vec<u8> {
    fn from(m: WebSocketMessage) -> Self {
        m.into_bytes()
    }
}
