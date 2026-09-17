//! Noticing that the caller of a tool call has gone away (ADR-0031).
//!
//! A call that waits, `claim` or `task_pull` with `wait_secs`, must stop
//! waiting when nobody can receive its result, or it grants a lease or
//! assigns a task to an agent that has left. rmcp reports two of the ways a
//! caller goes: an MCP `notifications/cancelled` cancels the request's
//! token at once, and a closed session cancels it after rmcp's drain of up
//! to five seconds. It reports nothing when the HTTP connection carrying the
//! call drops. Every client that sends `initialize` gets a session, and a
//! session answers each request with a server-sent event stream that the
//! client could resume, so rmcp keeps the handler running.
//!
//! [`layer`] closes that gap without a heartbeat. It gives every `/mcp`
//! request a [`Hangup`] and ties the sending half to the response body.
//! Hyper drops the body when the client's connection closes, and rmcp ends
//! it when the session closes; either way the hangup fires. A reply sent in
//! full ends the body as well, but by then the handler has returned and
//! nothing is listening. [`caller_gone`] combines the hangup with the
//! request's cancellation token for the tool handlers.

use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::{Body, Bytes, HttpBody};
use axum::extract::Request;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::Response;
use http_body::{Frame, SizeHint};
use rmcp::service::{RequestContext, RoleServer};
use tokio::sync::watch;

/// Fires when the HTTP request that carried an MCP request is over: its
/// response body was dropped or ended, or no response was ever produced.
#[derive(Debug, Clone)]
pub(crate) struct Hangup(watch::Receiver<()>);

impl Hangup {
    /// Completes once the request is over.
    async fn wait(mut self) {
        // Nothing is ever sent; `changed` fails once the sender is dropped.
        while self.0.changed().await.is_ok() {}
    }

    /// Whether the request is already over.
    fn is_over(&self) -> bool {
        self.0.has_changed().is_err()
    }

    /// The hangup of the HTTP request behind `context`, when it came
    /// through [`layer`].
    fn of(context: &RequestContext<RoleServer>) -> Option<Self> {
        context
            .extensions
            .get::<Parts>()
            .and_then(|parts| parts.extensions.get::<Self>())
            .cloned()
    }
}

/// Middleware for the MCP endpoint: adds a [`Hangup`] to the request and
/// ties it to the response body. If this future is dropped before the
/// response exists, the hangup fires too.
pub(crate) async fn layer(mut request: Request, next: Next) -> Response {
    let (hangup, receiver) = watch::channel(());
    request.extensions_mut().insert(Hangup(receiver));
    let response = next.run(request).await;
    response.map(|body| {
        Body::new(Tied {
            body,
            hangup: Some(hangup),
        })
    })
}

/// Completes when the caller of this tool call can no longer receive its
/// result: the client cancelled the request, its MCP session closed, or
/// the HTTP request that carried the call is over.
pub(crate) fn caller_gone(
    context: &RequestContext<RoleServer>,
) -> impl Future<Output = ()> + Send + 'static {
    let cancelled = context.ct.clone();
    let hangup = Hangup::of(context);
    async move {
        match hangup {
            Some(hangup) => tokio::select! {
                () = cancelled.cancelled() => {}
                () = hangup.wait() => {}
            },
            None => cancelled.cancelled().await,
        }
    }
}

/// Whether [`caller_gone`] would complete at once.
pub(crate) fn is_caller_gone(context: &RequestContext<RoleServer>) -> bool {
    context.ct.is_cancelled() || Hangup::of(context).is_some_and(|hangup| hangup.is_over())
}

/// A response body holding the sending half of a [`Hangup`], so the hangup
/// fires when the body ends or is dropped.
struct Tied {
    body: Body,
    hangup: Option<watch::Sender<()>>,
}

impl HttpBody for Tied {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        let polled = Pin::new(&mut self.body).poll_frame(cx);
        if matches!(polled, Poll::Ready(None | Some(Err(_)))) {
            // The reply is over; fire now rather than when hyper drops us.
            self.hangup.take();
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hangup fires once the body holding its sender is dropped, and not
    /// before; the body keeps the size of the one it wraps.
    #[tokio::test]
    async fn a_hangup_fires_when_its_body_is_dropped() {
        let (sender, receiver) = watch::channel(());
        let hangup = Hangup(receiver);
        let body = Tied {
            body: Body::from("hello"),
            hangup: Some(sender),
        };
        assert!(!hangup.is_over(), "the body is still alive");
        assert_eq!(body.size_hint().exact(), Some(5));
        assert!(!body.is_end_stream());
        drop(body);
        assert!(hangup.is_over());
        tokio::time::timeout(std::time::Duration::from_secs(1), hangup.wait())
            .await
            .unwrap();
    }

    /// A reply read to its end fires the hangup even while hyper still
    /// holds the body.
    #[tokio::test]
    async fn a_hangup_fires_when_its_body_ends() {
        let (sender, receiver) = watch::channel(());
        let hangup = Hangup(receiver);
        let mut body = Tied {
            body: Body::from("hello"),
            hangup: Some(sender),
        };
        let first = std::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await;
        assert!(first.is_some_and(|frame| frame.is_ok()));
        assert!(!hangup.is_over(), "the data frame is not the end");
        let end = std::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await;
        assert!(end.is_none());
        assert!(hangup.is_over(), "ended, not yet dropped");
        drop(body);
    }
}
