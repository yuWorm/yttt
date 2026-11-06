//! The node's HTTP surface: the `/relay` WebSocket upgrade plus the probes
//! iroh endpoints send to measure a relay.

use std::{convert::Infallible, sync::Arc, time::Duration};

use bytes::Bytes;
use http_body_util::Full;
use hyper::{
    Method, Request, Response, StatusCode,
    body::Incoming,
    header::{self, HeaderValue},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use iroh_relay::{
    http::{CLIENT_AUTH_HEADER, ProtocolVersion, RELAY_PATH, RELAY_PROBE_PATH},
    protos::handshake,
    server::{
        ClientRequest, DynAccessControl, client::Config, streams::MaybeTlsStream,
        streams::RelayedStream,
    },
};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tokio_rustls_acme::AcmeAcceptor;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::{
    framing::{Metered, WsFramed},
    node::{Gate, Node},
};

/// Time a connection gets to finish TLS, HTTP and the relay handshake.
const ESTABLISH_TIMEOUT: Duration = Duration::from_secs(15);
/// The largest WebSocket message the relay protocol uses.
const MAX_FRAME_SIZE: usize = 1024 * 1024;
const WEBSOCKET_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const CHALLENGE_HEADER: &str = "X-Iroh-Challenge";
const CHALLENGE_RESPONSE_HEADER: &str = "X-Iroh-Response";

type Body = Full<Bytes>;

/// How the relay listener speaks TLS, when it does.
#[derive(Clone)]
pub(crate) enum Tls {
    Manual(TlsAcceptor),
    /// Certificates from Let's Encrypt; also answers its TLS-ALPN-01
    /// validation connections.
    Acme(AcmeAcceptor, Arc<rustls::ServerConfig>),
}

pub(crate) async fn serve(
    listener: TcpListener,
    tls: Option<Tls>,
    node: Arc<Node>,
    cancel: CancellationToken,
) {
    loop {
        let (stream, peer) = tokio::select! {
            () = cancel.cancelled() => return,
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    warn!(%error, "relay accept failed");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            },
        };
        let tls = tls.clone();
        let node = node.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = cancel.cancelled() => {}
                () = serve_connection(stream, tls, node) => {}
            }
            debug!(%peer, "relay HTTP connection ended");
        });
    }
}

async fn serve_connection(stream: TcpStream, tls: Option<Tls>, node: Arc<Node>) {
    // The relay protocol sends many small frames; never hold them back.
    let _ = stream.set_nodelay(true);
    let accepted = match tls {
        None => Ok(Ok(Some(MaybeTlsStream::Plain(stream)))),
        Some(Tls::Manual(acceptor)) => {
            tokio::time::timeout(ESTABLISH_TIMEOUT, acceptor.accept(stream))
                .await
                .map(|accepted| accepted.map(|stream| Some(MaybeTlsStream::Tls(stream))))
        }
        Some(Tls::Acme(acceptor, config)) => {
            tokio::time::timeout(ESTABLISH_TIMEOUT, async move {
                match acceptor.accept(stream).await? {
                    // A certificate authority's validation, answered already.
                    None => Ok(None),
                    Some(start) => start
                        .into_stream(config)
                        .await
                        .map(|stream| Some(MaybeTlsStream::Tls(stream))),
                }
            })
            .await
        }
    };
    let io = match accepted {
        Ok(Ok(Some(io))) => io,
        Ok(Ok(None)) => return,
        Ok(Err(error)) => return debug!(%error, "TLS handshake failed"),
        Err(_) => return debug!("TLS handshake timed out"),
    };
    let plain = matches!(io, MaybeTlsStream::Plain(_));
    let service = service_fn(move |request| {
        let node = node.clone();
        async move { Ok::<_, Infallible>(route(node, request, plain)) }
    });
    if let Err(error) = http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(ESTABLISH_TIMEOUT)
        .serve_connection(TokioIo::new(io), service)
        .with_upgrades()
        .await
    {
        debug!(%error, "relay HTTP connection failed");
    }
}

/// Serves only the captive-portal probe, on plain HTTP beside an HTTPS node.
pub(crate) async fn serve_captive_portal(listener: TcpListener, cancel: CancellationToken) {
    loop {
        let stream = tokio::select! {
            () = cancel.cancelled() => return,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(_) => continue,
            },
        };
        tokio::spawn(async move {
            let service = service_fn(|request: Request<Incoming>| async move {
                Ok::<_, Infallible>(match (request.method(), request.uri().path()) {
                    (&Method::GET, "/generate_204") => no_content(&request),
                    _ => text(StatusCode::NOT_FOUND, "Not Found"),
                })
            });
            let _ = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(ESTABLISH_TIMEOUT)
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

fn route(node: Arc<Node>, request: Request<Incoming>, plain: bool) -> Response<Body> {
    match (request.method(), request.uri().path()) {
        (&Method::GET, RELAY_PATH) => upgrade(node, request),
        (&Method::GET, RELAY_PROBE_PATH) => Response::builder()
            .status(StatusCode::OK)
            .header("Access-Control-Allow-Origin", "*")
            .body(Body::default())
            .expect("static response"),
        (&Method::GET, "/generate_204") if plain => no_content(&request),
        (&Method::GET, "/healthz") => text(StatusCode::OK, "ok"),
        (&Method::GET, "/robots.txt") => text(StatusCode::OK, "User-agent: *\nDisallow: /\n"),
        (&Method::GET, "/") => text(StatusCode::OK, "yttt relay"),
        _ => text(StatusCode::NOT_FOUND, "Not Found"),
    }
}

fn upgrade(node: Arc<Node>, mut request: Request<Incoming>) -> Response<Body> {
    let headers = request.headers();
    let is_websocket = headers
        .get(header::UPGRADE)
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"websocket"));
    let Some(key) = headers.get(header::SEC_WEBSOCKET_KEY).cloned() else {
        return text(StatusCode::BAD_REQUEST, "missing Sec-WebSocket-Key");
    };
    if !is_websocket {
        return text(StatusCode::BAD_REQUEST, "expected a WebSocket upgrade");
    }
    if headers
        .get(header::SEC_WEBSOCKET_VERSION)
        .is_none_or(|version| version.as_bytes() != b"13")
    {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header(header::SEC_WEBSOCKET_VERSION, "13")
            .body(Body::from("unsupported WebSocket version"))
            .expect("static response");
    }
    let Some(version) = headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .split(',')
                .filter_map(|protocol| ProtocolVersion::match_from_str(protocol.trim()))
                .max()
        })
    else {
        return text(
            StatusCode::BAD_REQUEST,
            &format!(
                "supported relay protocols: {}",
                ProtocolVersion::all_joined()
            ),
        );
    };

    tokio::spawn(async move {
        let upgraded = match hyper::upgrade::on(&mut request).await {
            Ok(upgraded) => upgraded,
            Err(error) => return debug!(%error, "relay upgrade failed"),
        };
        let Ok(parts) = upgraded.downcast::<TokioIo<MaybeTlsStream>>() else {
            return warn!("relay upgrade carried an unexpected stream");
        };
        if !parts.read_buf.is_empty() {
            return debug!("client sent data before the upgrade finished");
        }
        let (request_parts, _) = request.into_parts();
        let io = parts.io.into_inner();
        match tokio::time::timeout(ESTABLISH_TIMEOUT, accept(&node, io, request_parts, version))
            .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => debug!(%error, "relay client refused"),
            Err(_) => debug!("relay handshake timed out"),
        }
    });

    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::UPGRADE, HeaderValue::from_static("websocket"))
        .header(header::CONNECTION, HeaderValue::from_static("upgrade"))
        .header(header::SEC_WEBSOCKET_ACCEPT, accept_key(&key))
        .header(header::SEC_WEBSOCKET_PROTOCOL, version.to_header_value())
        .body(Body::default())
        .expect("valid upgrade response")
}

/// Proves the client's EndpointId, checks its ticket, and hands the metered
/// stream to iroh's relay core.
async fn accept(
    node: &Arc<Node>,
    io: MaybeTlsStream,
    parts: hyper::http::request::Parts,
    version: ProtocolVersion,
) -> Result<(), handshake::Error> {
    let websocket = tokio_websockets::ServerBuilder::new()
        .limits(tokio_websockets::Limits::default().max_payload_len(Some(MAX_FRAME_SIZE)))
        .serve(io);
    let mut framed = WsFramed { io: websocket };
    let auth_header = parts.headers.get(CLIENT_AUTH_HEADER).cloned();
    let authentication = handshake::serverside(&mut framed, auth_header).await?;
    let request = ClientRequest::new(authentication.client_key, version, parts);
    let access: Arc<dyn DynAccessControl> = Arc::new(Gate(node.clone()));
    let guard = authentication
        .authorize_with(&request, &access, &mut framed)
        .await?;
    let Some(account) = node.account_of(guard.connection_id()) else {
        // Suspended or expired between admission and here; the guard's drop
        // already cleaned up.
        return Ok(());
    };
    let stream = RelayedStream::new(Metered::new(framed, account), node.key_cache.clone());
    node.clients
        .register(Config::new(guard, stream, version), node.metrics.clone());
    Ok(())
}

fn accept_key(key: &HeaderValue) -> String {
    use sha1::Digest as _;
    let mut sha1 = sha1::Sha1::new();
    sha1.update(key.as_bytes());
    sha1.update(WEBSOCKET_GUID);
    data_encoding::BASE64.encode(&sha1.finalize())
}

fn no_content(request: &Request<Incoming>) -> Response<Body> {
    let mut response = Response::builder().status(StatusCode::NO_CONTENT);
    if let Some(challenge) = request
        .headers()
        .get(CHALLENGE_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() < 64
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        })
    {
        response = response.header(CHALLENGE_RESPONSE_HEADER, format!("response {challenge}"));
    }
    response.body(Body::default()).expect("static response")
}

fn text(status: StatusCode, body: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(body.to_owned()))
        .expect("static response")
}
