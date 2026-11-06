//! Signing in without a password.
//!
//! The app opens the console in the browser and waits on a loopback port for
//! the answer (authorization code with PKCE, RFC 8252): the code comes back
//! only to this machine, and only this process holds the verifier that turns
//! it into a session. A device without a screen shows a short code someone
//! confirms on the console from any browser (RFC 8628).

use std::time::Duration;

use rand::RngCore as _;
use sha2::{Digest as _, Sha256};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
};
use yttt_relay_ticket::api::{
    DeviceAuthorization, DeviceAuthorizationRequest, TokenRequest, token_error,
};

use crate::{ControlClient, ControlError, ControlSession};

const MAX_REQUEST_BYTES: usize = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// A browser sign-in in progress: open `authorize_url`, then `finish`.
pub struct BrowserLogin {
    client: ControlClient,
    listener: TcpListener,
    redirect_uri: String,
    state: String,
    verifier: String,
    authorize_url: String,
}

impl ControlClient {
    /// Listens on a loopback port and prepares the console page that asks
    /// the signed-in person to let this app in. `label` names this device on
    /// that page.
    pub async fn begin_browser_login(&self, label: &str) -> Result<BrowserLogin, ControlError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|error| ControlError::Unreachable(format!("a loopback port: {error}")))?;
        let port = listener
            .local_addr()
            .map_err(|error| ControlError::Unreachable(error.to_string()))?
            .port();
        let redirect_uri = format!("http://127.0.0.1:{port}/callback");
        let state = random();
        let verifier = random();
        let challenge = data_encoding::BASE64URL_NOPAD.encode(&Sha256::digest(verifier.as_bytes()));
        let mut url = url::Url::parse(&self.url("/authorize"))
            .map_err(|error| ControlError::Protocol(error.to_string()))?;
        url.query_pairs_mut()
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("state", &state)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("label", label);
        Ok(BrowserLogin {
            client: self.clone(),
            listener,
            redirect_uri,
            state,
            verifier,
            authorize_url: url.into(),
        })
    }

    /// Asks for a short code to confirm on the console.
    pub async fn begin_device_login(&self, label: &str) -> Result<DeviceLogin, ControlError> {
        let authorization: DeviceAuthorization =
            self.send(self.http.post(self.url("/v1/oauth/device")).json(
                &DeviceAuthorizationRequest {
                    label: label.to_owned(),
                },
            ))
            .await?;
        let verification_uri = authorization
            .verification_uri
            .clone()
            .unwrap_or_else(|| self.url("/device"));
        Ok(DeviceLogin {
            client: self.clone(),
            verification_uri_complete: format!(
                "{verification_uri}?code={}",
                authorization.user_code
            ),
            verification_uri,
            user_code: authorization.user_code,
            device_code: authorization.device_code,
            interval: Duration::from_secs(authorization.interval.clamp(1, 60)),
            expires_in: Duration::from_secs(authorization.expires_in),
        })
    }

    async fn exchange(&self, request: &TokenRequest) -> Result<ControlSession, ControlError> {
        let granted: yttt_relay_ticket::api::SessionGranted = self
            .send(self.http.post(self.url("/v1/oauth/token")).json(request))
            .await?;
        Ok(ControlSession {
            control_url: self.base.clone(),
            account_id: granted.account_id,
            token: granted.token,
        })
    }
}

impl BrowserLogin {
    /// The console page to open in the browser.
    pub fn authorize_url(&self) -> &str {
        &self.authorize_url
    }

    /// Waits for the browser to come back, for as long as the caller lets it,
    /// and answers the browser with a page saying how it went.
    pub async fn finish(self) -> Result<ControlSession, ControlError> {
        loop {
            let Ok((mut stream, _)) = self.listener.accept().await else {
                continue;
            };
            let Some(target) = read_request_target(&mut stream).await else {
                continue;
            };
            let url = match url::Url::parse(&format!("http://127.0.0.1{target}")) {
                Ok(url) if url.path() == "/callback" => url,
                // A favicon or anything else the browser asks for.
                _ => {
                    respond(&mut stream, "404 Not Found", "").await;
                    continue;
                }
            };
            let query = |name: &str| {
                url.query_pairs()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.into_owned())
            };
            // Something other than this sign-in's console page; keep waiting.
            if query("state").as_deref() != Some(self.state.as_str()) {
                respond(&mut stream, "400 Bad Request", &page(PAGE_INVALID)).await;
                continue;
            }
            let Some(code) = query("code") else {
                respond(&mut stream, "200 OK", &page(PAGE_DENIED)).await;
                return Err(ControlError::Denied);
            };
            let session = self
                .client
                .exchange(&TokenRequest::AuthorizationCode {
                    code,
                    code_verifier: self.verifier.clone(),
                    redirect_uri: self.redirect_uri.clone(),
                })
                .await;
            let shown = if session.is_ok() {
                PAGE_DONE
            } else {
                PAGE_FAILED
            };
            respond(&mut stream, "200 OK", &page(shown)).await;
            return session;
        }
    }
}

/// A short-code sign-in in progress: show `user_code` and
/// `verification_uri`, then `finish`.
pub struct DeviceLogin {
    client: ControlClient,
    pub user_code: String,
    pub verification_uri: String,
    /// The page with the code filled in, e.g. for a QR code.
    pub verification_uri_complete: String,
    device_code: String,
    interval: Duration,
    expires_in: Duration,
}

impl DeviceLogin {
    /// Polls until someone confirms or denies the code, or it expires.
    pub async fn finish(self) -> Result<ControlSession, ControlError> {
        let deadline = tokio::time::Instant::now() + self.expires_in;
        let mut interval = self.interval;
        let request = TokenRequest::DeviceCode {
            device_code: self.device_code.clone(),
        };
        loop {
            tokio::time::sleep(interval).await;
            if tokio::time::Instant::now() >= deadline {
                return Err(ControlError::Expired);
            }
            match self.client.exchange(&request).await {
                Ok(session) => return Ok(session),
                Err(ControlError::Refused {
                    status: 400,
                    message,
                }) => match message.as_str() {
                    token_error::PENDING => {}
                    token_error::SLOW_DOWN => interval += Duration::from_secs(5),
                    token_error::DENIED => return Err(ControlError::Denied),
                    token_error::EXPIRED => return Err(ControlError::Expired),
                    _ => {
                        return Err(ControlError::Refused {
                            status: 400,
                            message,
                        });
                    }
                },
                // A dropped connection or a restart: try again next time.
                Err(ControlError::Unreachable(error)) => {
                    tracing::debug!(%error, "polling for the device sign-in failed");
                }
                Err(error) => return Err(error),
            }
        }
    }
}

/// The path and query of an HTTP request, from its first line.
async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let read = async {
        while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut chunk).await.ok()?;
            if count == 0 || buffer.len() + count > MAX_REQUEST_BYTES {
                return None;
            }
            buffer.extend_from_slice(&chunk[..count]);
        }
        Some(())
    };
    tokio::time::timeout(READ_TIMEOUT, read).await.ok()??;
    let line = std::str::from_utf8(&buffer).ok()?.lines().next()?;
    let mut parts = line.split(' ');
    (parts.next()? == "GET").then_some(())?;
    let target = parts.next()?;
    target.starts_with('/').then(|| target.to_owned())
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

const PAGE_DONE: (&str, &str) = (
    "已登录 · Signed in",
    "yttt 已登录，可以关闭此页并回到应用。<br>yttt is signed in; you can close this page.",
);
const PAGE_DENIED: (&str, &str) = (
    "已取消 · Cancelled",
    "已拒绝这次登录，可以关闭此页。<br>The sign-in was denied; you can close this page.",
);
const PAGE_FAILED: (&str, &str) = (
    "登录失败 · Sign-in failed",
    "请回到 yttt 应用重试。<br>Go back to yttt and try again.",
);
const PAGE_INVALID: (&str, &str) = (
    "无效的请求 · Invalid request",
    "这个页面不属于当前的登录。<br>This page is not part of the current sign-in.",
);

fn page((title, text): (&str, &str)) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title>\
         <meta name=\"viewport\" content=\"width=device-width\"></head>\
         <body style=\"font-family:system-ui,sans-serif;display:grid;place-items:center;\
         min-height:90vh;margin:0;color:#1c1f23\"><div style=\"text-align:center\">\
         <h2>{title}</h2><p style=\"color:#555;line-height:1.7\">{text}</p></div></body></html>"
    )
}

/// 32 random bytes, base64url: the state and the PKCE verifier.
fn random() -> String {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    data_encoding::BASE64URL_NOPAD.encode(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn get(port: u16, target: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).await.unwrap();
        reply
    }

    #[tokio::test]
    async fn the_loopback_answers_only_its_own_sign_in() {
        let client = ControlClient::new("http://127.0.0.1:9").unwrap();
        let login = client.begin_browser_login("laptop").await.unwrap();
        let url = url::Url::parse(login.authorize_url()).unwrap();
        assert_eq!(url.path(), "/authorize");
        let query = |name: &str| {
            url.query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
                .unwrap()
        };
        assert_eq!(query("code_challenge_method"), "S256");
        assert_eq!(query("label"), "laptop");
        let redirect = url::Url::parse(&query("redirect_uri")).unwrap();
        assert_eq!(redirect.host_str(), Some("127.0.0.1"));
        let port = redirect.port().unwrap();
        let state = query("state");

        let finished = tokio::spawn(login.finish());
        assert!(get(port, "/favicon.ico").await.starts_with("HTTP/1.1 404"));
        assert!(
            get(port, "/callback?code=x&state=someone-else")
                .await
                .starts_with("HTTP/1.1 400")
        );
        let denied = get(
            port,
            &format!("/callback?error=access_denied&state={state}"),
        )
        .await;
        assert!(denied.contains("Cancelled"));
        assert!(matches!(finished.await.unwrap(), Err(ControlError::Denied)));
    }
}
