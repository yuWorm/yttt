//! Signing in to the console with GitHub or Google, and linking those to an
//! account.
//!
//! An identity is the provider's unchanging id for a person, never their
//! email. An identity whose email already belongs to an account is not
//! merged into it: whoever owns that account links the identity after
//! signing in, so nobody takes over an account through a provider's email.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use serde::{Deserialize, Serialize};
use sqlx::Row as _;

use crate::{
    AppState, Failure,
    admin::audit,
    auth::{self, Attempt, ConsoleSession},
    clamp_i64,
    config::ProviderConfig,
    now_unix, sha256, web,
};

type Shared = State<Arc<AppState>>;

const STATE_TTL_SECS: u64 = 10 * 60;
const COOKIE_SECURE: &str = "__Host-yttt_oauth";
const COOKIE_PLAIN: &str = "yttt_oauth";
/// What the console pages show when a sign-in or link through a provider
/// fails, in `?oauth_error=`.
mod reason {
    pub const DENIED: &str = "denied";
    pub const FAILED: &str = "failed";
    pub const UNVERIFIED: &str = "unverified_email";
    pub const EMAIL_TAKEN: &str = "email_taken";
    pub const SIGNUP_CLOSED: &str = "signup_closed";
    pub const ALREADY_LINKED: &str = "already_linked";
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Provider {
    Github,
    Google,
}

impl Provider {
    const ALL: [Self; 2] = [Self::Github, Self::Google];

    fn name(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Google => "google",
        }
    }

    fn parse(name: &str) -> Result<Self, Failure> {
        Self::ALL
            .into_iter()
            .find(|provider| provider.name() == name)
            .ok_or_else(Failure::not_found)
    }

    fn config(self, state: &AppState) -> Option<&ProviderConfig> {
        match self {
            Self::Github => state.config.oauth.github.as_ref(),
            Self::Google => state.config.oauth.google.as_ref(),
        }
    }

    fn configured(self, state: &AppState) -> Result<&ProviderConfig, Failure> {
        self.config(state).ok_or_else(Failure::not_found)
    }

    fn authorize_url(self, config: &ProviderConfig) -> &str {
        config.authorize_url.as_deref().unwrap_or(match self {
            Self::Github => "https://github.com/login/oauth/authorize",
            Self::Google => "https://accounts.google.com/o/oauth2/v2/auth",
        })
    }

    fn token_url(self, config: &ProviderConfig) -> &str {
        config.token_url.as_deref().unwrap_or(match self {
            Self::Github => "https://github.com/login/oauth/access_token",
            Self::Google => "https://oauth2.googleapis.com/token",
        })
    }

    fn api_url(self, config: &ProviderConfig) -> &str {
        config.api_url.as_deref().unwrap_or(match self {
            Self::Github => "https://api.github.com",
            Self::Google => "https://openidconnect.googleapis.com/v1/userinfo",
        })
    }

    fn scope(self) -> &'static str {
        match self {
            Self::Github => "read:user user:email",
            Self::Google => "openid email",
        }
    }
}

/// Refuses a configuration whose providers could not call back: they need
/// the console's public address, and Google takes only HTTPS callbacks.
pub(crate) fn check_config(config: &crate::ControlConfig) -> Result<(), &'static str> {
    if config.oauth.github.is_none() && config.oauth.google.is_none() {
        return Ok(());
    }
    let Some(public_url) = config.public_url.as_deref() else {
        return Err("GitHub and Google sign-in need public_url for their callbacks");
    };
    let local = ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|prefix| public_url.starts_with(prefix));
    if config.oauth.google.is_some() && !public_url.starts_with("https://") && !local {
        return Err("Google sign-in needs an https:// public_url");
    }
    Ok(())
}

fn callback_url(state: &AppState, provider: Provider) -> String {
    let base = state
        .config
        .public_url
        .as_deref()
        .expect("checked at start")
        .trim_end_matches('/');
    format!("{base}/v1/auth/{}/callback", provider.name())
}

/// `GET /v1/auth/providers`: which buttons the console shows.
pub(crate) async fn providers(State(state): Shared) -> Json<Vec<&'static str>> {
    Json(
        Provider::ALL
            .into_iter()
            .filter(|provider| provider.config(&state).is_some())
            .map(Provider::name)
            .collect(),
    )
}

#[derive(Deserialize)]
pub(crate) struct StartQuery {
    #[serde(default)]
    next: String,
}

/// `GET /v1/auth/{provider}/start`: leaves for the provider to sign in.
pub(crate) async fn start(
    State(state): Shared,
    _attempt: Attempt,
    Path(provider): Path<String>,
    Query(query): Query<StartQuery>,
) -> Result<Response, Failure> {
    let provider = Provider::parse(&provider)?;
    let (url, cookie) = begin(&state, provider, &query.next, None).await?;
    let mut response = Redirect::to(&url).into_response();
    set_cookie(&mut response, &cookie);
    Ok(response)
}

#[derive(Deserialize)]
pub(crate) struct LinkRequest {
    #[serde(default)]
    next: String,
}

#[derive(Serialize)]
pub(crate) struct LinkStart {
    redirect: String,
}

/// `POST /v1/auth/{provider}/link`: the signed-in account adds a provider.
/// A console POST, so another site cannot start it.
pub(crate) async fn link(
    State(state): Shared,
    session: ConsoleSession,
    Path(provider): Path<String>,
    Json(request): Json<LinkRequest>,
) -> Result<Response, Failure> {
    let provider = Provider::parse(&provider)?;
    let (redirect, cookie) =
        begin(&state, provider, &request.next, Some(session.account_id)).await?;
    let mut response = Json(LinkStart { redirect }).into_response();
    set_cookie(&mut response, &cookie);
    Ok(response)
}

async fn begin(
    state: &AppState,
    provider: Provider,
    next: &str,
    link_account: Option<i64>,
) -> Result<(String, String), Failure> {
    let config = provider.configured(state)?;
    let login_state = auth::random_token();
    let verifier = auth::random_token();
    let now = now_unix();
    sqlx::query("DELETE FROM login_states WHERE expires_unix <= $1")
        .bind(clamp_i64(now))
        .execute(&state.pool)
        .await?;
    sqlx::query(
        "INSERT INTO login_states (state_hash, provider, verifier, next, link_account, expires_unix)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(sha256(login_state.as_bytes()))
    .bind(provider.name())
    .bind(&verifier)
    .bind(safe_next(next))
    .bind(link_account)
    .bind(clamp_i64(now + STATE_TTL_SECS))
    .execute(&state.pool)
    .await?;
    let challenge = data_encoding::BASE64URL_NOPAD.encode(&sha256(verifier.as_bytes()));
    let mut url = url::Url::parse(provider.authorize_url(config))
        .map_err(|_| Failure::unavailable("the provider's address is invalid"))?;
    url.query_pairs_mut()
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &callback_url(state, provider))
        .append_pair("response_type", "code")
        .append_pair("scope", provider.scope())
        .append_pair("state", &login_state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    if provider == Provider::Google {
        url.query_pairs_mut()
            .append_pair("prompt", "select_account");
    }
    Ok((url.into(), state_cookie(state, Some(&login_state))))
}

#[derive(Deserialize)]
pub(crate) struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

/// `GET /v1/auth/{provider}/callback`: back from the provider. Its state is
/// 256 random bits bound to the browser's cookie, so it spends no guess.
pub(crate) async fn callback(
    State(state): Shared,
    Path(provider): Path<String>,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Result<Response, Failure> {
    let provider = Provider::parse(&provider)?;
    let config = provider.configured(&state)?;
    // The state must be the one this browser left with.
    let carried = cookie(&headers);
    let login_state = query
        .state
        .as_deref()
        .filter(|login_state| carried.as_deref() == Some(*login_state))
        .ok_or(Failure::bad_request("the sign-in expired; start again"))?;
    let row = sqlx::query(
        "DELETE FROM login_states WHERE state_hash = $1 AND provider = $2
         RETURNING verifier, next, link_account, expires_unix",
    )
    .bind(sha256(login_state.as_bytes()))
    .bind(provider.name())
    .fetch_optional(&state.pool)
    .await?
    .filter(|row| row.get::<i64, _>("expires_unix") > clamp_i64(now_unix()))
    .ok_or(Failure::bad_request("the sign-in expired; start again"))?;
    let next: String = row.get("next");
    let link_account: Option<i64> = row.get("link_account");
    let back = if link_account.is_some() {
        "/account"
    } else {
        "/login"
    };
    let outcome = match query.code {
        None => Err(reason::DENIED),
        Some(code) => {
            match identify(
                &state,
                provider,
                config,
                &code,
                &row.get::<String, _>("verifier"),
            )
            .await
            {
                Ok(person) => match link_account {
                    Some(account_id) => link_identity(&state, provider, &person, account_id).await,
                    None => sign_in(&state, provider, &person).await,
                },
                Err(reason) => Err(reason),
            }
        }
    };
    let mut response = match outcome {
        Ok(Done::Linked) => page(&next),
        Ok(Done::SignedIn { token }) => {
            let mut response = page(&next);
            let cookie =
                auth::session_cookie(&state, Some(&token), state.config.session_ttl().as_secs());
            response.headers_mut().append(
                header::SET_COOKIE,
                HeaderValue::from_str(&cookie).expect("a cookie is a valid header"),
            );
            response
        }
        Err(reason) => page(&format!(
            "{back}?oauth_error={reason}&provider={}",
            provider.name()
        )),
    };
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&state_cookie(&state, None)).expect("a cookie is a valid header"),
    );
    Ok(response)
}

enum Done {
    SignedIn { token: String },
    Linked,
}

/// Who the provider says signed in.
struct Person {
    subject: String,
    /// Verified, normalized.
    email: String,
}

async fn identify(
    state: &AppState,
    provider: Provider,
    config: &ProviderConfig,
    code: &str,
    verifier: &str,
) -> Result<Person, &'static str> {
    let failed = |error: reqwest::Error| {
        tracing::warn!(provider = provider.name(), %error, "sign-in with the provider failed");
        reason::FAILED
    };
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("yttt-control")
        .build()
        .map_err(failed)?;
    let callback = callback_url(state, provider);
    let mut form = vec![
        ("client_id", config.client_id.as_str()),
        ("client_secret", config.client_secret.as_str()),
        ("code", code),
        ("code_verifier", verifier),
        ("redirect_uri", callback.as_str()),
    ];
    if provider == Provider::Google {
        form.push(("grant_type", "authorization_code"));
    }
    #[derive(Deserialize)]
    struct Token {
        access_token: Option<String>,
    }
    // GitHub answers in form encoding unless asked for JSON.
    let token: Token = http
        .post(provider.token_url(config))
        .header(header::ACCEPT, "application/json")
        .form(&form)
        .send()
        .await
        .map_err(failed)?
        .error_for_status()
        .map_err(failed)?
        .json()
        .await
        .map_err(failed)?;
    let access = token.access_token.ok_or(reason::FAILED)?;
    let api = provider.api_url(config).trim_end_matches('/');
    let get = |url: String| {
        http.get(url)
            .bearer_auth(&access)
            .header(header::ACCEPT, "application/json")
            .send()
    };
    let (subject, email) = match provider {
        Provider::Github => {
            #[derive(Deserialize)]
            struct User {
                id: u64,
            }
            #[derive(Deserialize)]
            struct Email {
                email: String,
                primary: bool,
                verified: bool,
            }
            let user: User = get(format!("{api}/user"))
                .await
                .map_err(failed)?
                .error_for_status()
                .map_err(failed)?
                .json()
                .await
                .map_err(failed)?;
            let emails: Vec<Email> = get(format!("{api}/user/emails"))
                .await
                .map_err(failed)?
                .error_for_status()
                .map_err(failed)?
                .json()
                .await
                .map_err(failed)?;
            let email = emails
                .into_iter()
                .find(|email| email.primary && email.verified)
                .map(|email| email.email);
            (user.id.to_string(), email)
        }
        Provider::Google => {
            #[derive(Deserialize)]
            struct UserInfo {
                sub: String,
                email: Option<String>,
                #[serde(default)]
                email_verified: bool,
            }
            let info: UserInfo = get(api.to_string())
                .await
                .map_err(failed)?
                .error_for_status()
                .map_err(failed)?
                .json()
                .await
                .map_err(failed)?;
            (info.sub, info.email.filter(|_| info.email_verified))
        }
    };
    let email = email
        .and_then(|email| web::normalize_email(&email).ok())
        .ok_or(reason::UNVERIFIED)?;
    Ok(Person { subject, email })
}

async fn sign_in(
    state: &AppState,
    provider: Provider,
    person: &Person,
) -> Result<Done, &'static str> {
    let internal = |_| reason::FAILED;
    let linked: Option<i64> = sqlx::query_scalar(
        "SELECT account_id FROM identities WHERE provider = $1 AND subject = $2",
    )
    .bind(provider.name())
    .bind(&person.subject)
    .fetch_optional(&state.pool)
    .await
    .map_err(|error| {
        tracing::error!(%error, "database request failed");
        reason::FAILED
    })?;
    let account_id = match linked {
        Some(account_id) => account_id,
        None => {
            let account_id = web::create_account(state, &person.email, None)
                .await
                .map_err(|failure| match failure.status() {
                    StatusCode::CONFLICT => reason::EMAIL_TAKEN,
                    StatusCode::NOT_FOUND | StatusCode::SERVICE_UNAVAILABLE => {
                        reason::SIGNUP_CLOSED
                    }
                    _ => reason::FAILED,
                })?;
            insert_identity(state, provider, person, account_id).await?;
            account_id
        }
    };
    let (token, _) = auth::open_session(state, account_id, auth::Scope::Web)
        .await
        .map_err(internal)?;
    Ok(Done::SignedIn { token })
}

async fn link_identity(
    state: &AppState,
    provider: Provider,
    person: &Person,
    account_id: i64,
) -> Result<Done, &'static str> {
    insert_identity(state, provider, person, account_id).await?;
    record(state, account_id, "identity.link", provider)
        .await
        .map_err(|_| reason::FAILED)?;
    Ok(Done::Linked)
}

async fn insert_identity(
    state: &AppState,
    provider: Provider,
    person: &Person,
    account_id: i64,
) -> Result<(), &'static str> {
    let inserted = sqlx::query(
        "INSERT INTO identities (provider, subject, account_id, email, created_unix)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(provider.name())
    .bind(&person.subject)
    .bind(account_id)
    .bind(&person.email)
    .bind(clamp_i64(now_unix()))
    .execute(&state.pool)
    .await;
    match inserted {
        Ok(_) => Ok(()),
        // The identity is another account's, or this one has the provider.
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            Err(reason::ALREADY_LINKED)
        }
        Err(error) => {
            tracing::error!(%error, "database request failed");
            Err(reason::FAILED)
        }
    }
}

async fn record(
    state: &AppState,
    account_id: i64,
    action: &str,
    provider: Provider,
) -> Result<(), Failure> {
    let mut tx = state.pool.begin().await?;
    audit(
        &mut tx,
        account_id,
        action,
        Some(account_id),
        None,
        provider.name(),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Serialize)]
pub(crate) struct SignInMethods {
    has_password: bool,
    /// Whether this session signed in recently enough to set a password.
    may_set_password: bool,
    identities: Vec<IdentityView>,
    providers: Vec<&'static str>,
}

#[derive(Serialize)]
pub(crate) struct IdentityView {
    provider: String,
    email: String,
    created_unix: u64,
}

/// `GET /v1/accounts/me/sign-in`: how this account signs in.
pub(crate) async fn methods(
    State(state): Shared,
    session: ConsoleSession,
) -> Result<Json<SignInMethods>, Failure> {
    let has_password: bool =
        sqlx::query_scalar("SELECT password_hash IS NOT NULL FROM accounts WHERE id = $1")
            .bind(session.account_id)
            .fetch_one(&state.pool)
            .await?;
    let identities = sqlx::query(
        "SELECT provider, email, created_unix FROM identities WHERE account_id = $1
         ORDER BY provider",
    )
    .bind(session.account_id)
    .fetch_all(&state.pool)
    .await?
    .iter()
    .map(|row| IdentityView {
        provider: row.get("provider"),
        email: row.get("email"),
        created_unix: row.get::<i64, _>("created_unix").max(0) as u64,
    })
    .collect();
    Ok(Json(SignInMethods {
        has_password,
        may_set_password: web::may_set_password(&session),
        identities,
        providers: providers(State(state)).await.0,
    }))
}

/// `DELETE /v1/accounts/me/identities/{provider}`: unlinks a provider, unless
/// it is the only way left to sign in. Other sessions end with it.
pub(crate) async fn unlink(
    State(state): Shared,
    session: ConsoleSession,
    Path(provider): Path<String>,
) -> Result<StatusCode, Failure> {
    let provider = Provider::parse(&provider)?;
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query(
        "SELECT a.password_hash IS NOT NULL AS has_password,
                (SELECT COUNT(*) FROM identities i WHERE i.account_id = a.id) AS identities
         FROM accounts a WHERE a.id = $1 FOR UPDATE",
    )
    .bind(session.account_id)
    .fetch_one(&mut *tx)
    .await?;
    if !row.get::<bool, _>("has_password") && row.get::<i64, _>("identities") <= 1 {
        return Err(Failure::conflict(
            "set a password or link another provider before unlinking the last one",
        ));
    }
    let removed = sqlx::query("DELETE FROM identities WHERE account_id = $1 AND provider = $2")
        .bind(session.account_id)
        .bind(provider.name())
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(Failure::not_found());
    }
    audit(
        &mut tx,
        session.account_id,
        "identity.unlink",
        Some(session.account_id),
        None,
        provider.name(),
    )
    .await?;
    tx.commit().await?;
    auth::end_sessions(&state, session.account_id, Some(&session.token_hash)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A path on this site to go back to: one leading slash, nothing that a
/// browser could read as another host or a script.
pub(crate) fn safe_next(next: &str) -> String {
    let safe = next.starts_with('/')
        && !next.starts_with("//")
        && !next.contains('\\')
        && next.len() <= 512
        && next
            .chars()
            .all(|character| !character.is_control() && !"\"<>'` ".contains(character));
    if safe { next.to_string() } else { "/".into() }
}

/// Goes to `next` from a page on this site, so the console loads as a
/// same-site navigation that carries its session cookie; a redirect straight
/// from the provider's callback would not.
fn page(next: &str) -> Response {
    let next = safe_next(next);
    let html = format!(
        "<!doctype html><meta charset=\"utf-8\"><meta http-equiv=\"refresh\" content=\"0;url={next}\">\
         <title>yttt</title><p><a href=\"{next}\">Continue</a></p>"
    );
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
}

fn state_cookie(state: &AppState, value: Option<&str>) -> String {
    let secure = state.config.secure_cookies();
    let name = if secure { COOKIE_SECURE } else { COOKIE_PLAIN };
    let (value, max_age) = match value {
        Some(value) => (value, STATE_TTL_SECS),
        None => ("", 0),
    };
    // Lax: the provider sends the browser back from its own site.
    format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{}",
        if secure { "; Secure" } else { "" }
    )
}

fn set_cookie(response: &mut Response, cookie: &str) {
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(cookie).expect("a cookie is a valid header"),
    );
}

fn cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE_SECURE || *name == COOKIE_PLAIN)
        .map(|(_, value)| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_need_an_address_to_call_back_to() {
        let config = |public_url: &str, provider: &str| {
            crate::ControlConfig::from_toml(&format!(
                r#"
                listen = "127.0.0.1:0"
                {public_url}
                database_url = "postgres://localhost/x"
                ticket_secret = "00"
                plans = []
                [oauth.{provider}]
                client_id = "id"
                client_secret = "secret"
                "#
            ))
            .unwrap()
        };
        assert!(check_config(&config("", "github")).is_err());
        assert!(
            check_config(&config(
                r#"public_url = "http://control.example""#,
                "github"
            ))
            .is_ok()
        );
        assert!(
            check_config(&config(
                r#"public_url = "http://control.example""#,
                "google"
            ))
            .is_err()
        );
        assert!(
            check_config(&config(
                r#"public_url = "https://control.example""#,
                "google"
            ))
            .is_ok()
        );
        assert!(check_config(&config(r#"public_url = "http://127.0.0.1:8080""#, "google")).is_ok());
    }

    #[test]
    fn only_paths_on_this_site_are_followed() {
        assert_eq!(
            safe_next("/authorize?redirect_uri=http%3A%2F%2F127.0.0.1%3A5%2Fcb&label=x"),
            "/authorize?redirect_uri=http%3A%2F%2F127.0.0.1%3A5%2Fcb&label=x"
        );
        assert_eq!(safe_next("/device"), "/device");
        for bad in [
            "//evil.example",
            "/\\evil.example",
            "https://evil.example",
            "",
            "/a\"b",
        ] {
            assert_eq!(safe_next(bad), "/", "{bad}");
        }
        assert_eq!(safe_next("/login?x=<script>"), "/");
    }
}
