//! The byte-frame stream the relay protocol runs on, and the meter around it.
//!
//! iroh-relay keeps its own WebSocket adapter private, so this mirrors it on
//! the same `tokio-websockets`. The meter sits on frames the client sends,
//! which is every byte the account pushes through the node.

use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, ready},
};

use bytes::Bytes;
use futures_sink::Sink;
use futures_util::Stream;
use iroh_relay::{ExportKeyingMaterial, protos::streams::StreamError};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_websockets::{Message, Payload, WebSocketStream};

use crate::node::Account;

pub(crate) struct WsFramed<T> {
    pub(crate) io: WebSocketStream<T>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> Stream for WsFramed<T> {
    type Item = Result<Bytes, StreamError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match ready!(Pin::new(&mut self.io).poll_next(cx)) {
                None => return Poll::Ready(None),
                Some(Err(error)) => return Poll::Ready(Some(Err(StreamError::from_std(error)))),
                Some(Ok(message)) if message.is_close() => return Poll::Ready(None),
                Some(Ok(message)) if message.is_binary() => {
                    return Poll::Ready(Some(Ok(message.into_payload().into())));
                }
                // Pings are answered inside tokio-websockets; text is not the protocol.
                Some(Ok(_)) => continue,
            }
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Sink<Bytes> for WsFramed<T> {
    type Error = StreamError;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.io)
            .poll_ready(cx)
            .map_err(StreamError::from_std)
    }

    fn start_send(mut self: Pin<&mut Self>, bytes: Bytes) -> Result<(), Self::Error> {
        Pin::new(&mut self.io)
            .start_send(Message::binary(Payload::from(bytes)))
            .map_err(StreamError::from_std)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.io)
            .poll_flush(cx)
            .map_err(StreamError::from_std)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.io)
            .poll_close(cx)
            .map_err(StreamError::from_std)
    }
}

impl<T: ExportKeyingMaterial + AsyncRead + AsyncWrite + Unpin> ExportKeyingMaterial
    for WsFramed<T>
{
    fn export_keying_material<O: AsMut<[u8]>>(
        &self,
        output: O,
        label: &[u8],
        context: Option<&[u8]>,
    ) -> Option<O> {
        self.io
            .get_ref()
            .export_keying_material(output, label, context)
    }
}

/// Charges every frame the client sends to its account, and stops reading
/// from the client while the account's bucket is empty.
pub(crate) struct Metered<S> {
    inner: S,
    account: Arc<Account>,
    resume: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl<S> Metered<S> {
    pub(crate) fn new(inner: S, account: Arc<Account>) -> Self {
        Self {
            inner,
            account,
            resume: None,
        }
    }
}

impl<S: Stream<Item = Result<Bytes, StreamError>> + Unpin> Stream for Metered<S> {
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(resume) = self.resume.as_mut() {
            ready!(resume.as_mut().poll(cx));
            self.resume = None;
        }
        let item = ready!(Pin::new(&mut self.inner).poll_next(cx));
        if let Some(Ok(frame)) = &item
            && let Err(deadline) = self.account.consume(frame.len())
        {
            self.resume = Some(Box::pin(tokio::time::sleep_until(deadline)));
        }
        Poll::Ready(item)
    }
}

impl<S: Sink<Bytes, Error = StreamError> + Unpin> Sink<Bytes> for Metered<S> {
    type Error = StreamError;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.inner).poll_ready(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, bytes: Bytes) -> Result<(), Self::Error> {
        Pin::new(&mut self.inner).start_send(bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.inner).poll_close(cx)
    }
}
