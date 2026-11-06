//! The control plane against a real PostgreSQL. Each test gets its own
//! database on the server `YTTT_TEST_DATABASE_URL` names, e.g.
//! `postgres://postgres:yttt@127.0.0.1:55433/postgres`
//! (see packaging/relay-dev/docker-compose.full.yml).

use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::StatusCode;
use serde::{Serialize, de::DeserializeOwned};
use yttt_control::{Control, ControlConfig, NodeEntry, PlanConfig};
use yttt_relay_ticket::{
    DeviceProof, ProofPurpose, TicketVerifier,
    api::{
        AccountCreated, AccountStanding, AccountUsage, AccountView, CreateAccount, DeviceList,
        DeviceProofBody, DeviceRole, Login, NodeChoice, RegisterDevice, SessionGranted,
        SetSuspended, Standing, SwitchNode, TicketGranted, TicketRequest, UsageReply, UsageReport,
    },
    public_key,
};

const NODE: &str = "local-1";
const NODE_TOKEN: &str = "node-token";
const MONTHLY_BYTES: u64 = 10_000;
const THROTTLED: u32 = 1_000;
const PLAN_RATE: Standing = Standing::Active {
    rate_bps: 1_000_000,
    burst_bytes: 250_000,
};
const LAPTOP: [u8; 32] = [21; 32];
const PHONE: [u8; 32] = [22; 32];
const TABLET: [u8; 32] = [23; 32];

/// Sends a request as the console does: its cookie and its header.
trait AsConsole {
    fn console(self, cookie: &str) -> Self;
}

impl AsConsole for reqwest::RequestBuilder {
    fn console(self, cookie: &str) -> Self {
        self.header("cookie", cookie).header("x-yttt-web", "1")
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

struct Harness {
    control: Control,
    config: ControlConfig,
    http: reqwest::Client,
    base: String,
}

impl Harness {
    async fn start() -> Self {
        Self::start_with_nodes(&[(NODE, NODE_TOKEN)]).await
    }

    async fn start_with_nodes(nodes: &[(&str, &str)]) -> Self {
        Self::start_with(nodes, |_| {}).await
    }

    async fn start_with(nodes: &[(&str, &str)], edit: impl FnOnce(&mut ControlConfig)) -> Self {
        let server = std::env::var("YTTT_TEST_DATABASE_URL")
            .expect("YTTT_TEST_DATABASE_URL names a PostgreSQL server");
        let name = format!("yttt_control_test_{:016x}", rand_suffix());
        let admin = sqlx_admin(&server).await;
        sqlx_execute(&admin, &format!("CREATE DATABASE {name}")).await;
        let (prefix, _) = server.rsplit_once('/').unwrap();
        let config = ControlConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            public_url: Some("http://127.0.0.1".into()),
            web_root: None,
            relay_binaries_dir: None,
            trusted_proxy: false,
            signup: Some(yttt_control::SignupConfig {
                enabled: true,
                plan: "basic".into(),
                node: NODE.into(),
            }),
            database_url: format!("{prefix}/{name}"),
            ticket_secret: yttt_relay_ticket::hex(&[1; 32]),
            ticket_ttl_secs: 30 * 24 * 3600,
            session_ttl_secs: 3600,
            dev_mode: true,
            oauth: Default::default(),
            plans: vec![PlanConfig {
                id: "basic".into(),
                rate_bps: 1_000_000,
                burst_bytes: 250_000,
                monthly_bytes: MONTHLY_BYTES,
                throttled_rate_bps: THROTTLED,
                max_devices: 2,
            }],
            nodes: nodes
                .iter()
                .map(|(id, token)| NodeEntry {
                    id: (*id).into(),
                    url: "http://127.0.0.1:3340".into(),
                    token: (*token).into(),
                })
                .collect(),
        };
        let mut config = config;
        edit(&mut config);
        let control = Control::spawn(config.clone()).await.unwrap();
        let base = format!("http://{}", control.addr());
        Self {
            control,
            config,
            http: reqwest::Client::new(),
            base,
        }
    }

    async fn call<B: Serialize, R: DeserializeOwned>(
        &self,
        path: &str,
        token: Option<&str>,
        body: &B,
    ) -> Result<R, StatusCode> {
        let mut request = self.http.post(format!("{}{path}", self.base)).json(body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.unwrap();
        if !response.status().is_success() {
            return Err(response.status());
        }
        Ok(response.json().await.unwrap())
    }

    async fn get<R: DeserializeOwned>(&self, path: &str, token: &str) -> Result<R, StatusCode> {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        if !response.status().is_success() {
            return Err(response.status());
        }
        Ok(response.json().await.unwrap())
    }

    async fn status<B: Serialize>(&self, path: &str, token: Option<&str>, body: &B) -> StatusCode {
        let mut request = self.http.post(format!("{}{path}", self.base)).json(body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send().await.unwrap().status()
    }

    async fn account(&self, email: &str) -> (u64, String) {
        let created: AccountCreated = self
            .call(
                "/v1/dev/accounts",
                None,
                &CreateAccount {
                    email: email.into(),
                    password: "correct horse".into(),
                    plan: "basic".into(),
                    node: NODE.into(),
                },
            )
            .await
            .unwrap();
        let session: SessionGranted = self
            .call(
                "/v1/sessions",
                None,
                &Login {
                    email: email.into(),
                    password: "correct horse".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(session.account_id, created.account_id);
        (created.account_id, session.token)
    }

    /// Signs in on the console and returns its cookie: administration and
    /// sign-in settings take nothing else.
    async fn console(&self, email: &str) -> String {
        let response = self
            .http
            .post(format!("{}/v1/web/session", self.base))
            .header("x-yttt-web", "1")
            .json(&serde_json::json!({"email": email, "password": "correct horse"}))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    async fn register(&self, session: &str, account: u64, device: &[u8; 32]) -> StatusCode {
        self.status(
            "/v1/devices",
            Some(session),
            &RegisterDevice {
                role: None,
                follows_host: None,
                label: "laptop".into(),
                proof: proof(device, ProofPurpose::RegisterDevice, account),
            },
        )
        .await
    }

    async fn ticket(
        &self,
        session: &str,
        account: u64,
        device: &[u8; 32],
    ) -> Result<TicketGranted, StatusCode> {
        self.call(
            "/v1/tickets",
            Some(session),
            &TicketRequest {
                host: None,
                proof: proof(device, ProofPurpose::IssueTicket, account),
            },
        )
        .await
    }

    async fn report(&self, token: &str, usage: &[(u64, u64)]) -> Result<UsageReply, StatusCode> {
        self.report_devices(
            token,
            &usage
                .iter()
                .map(|(account, bytes)| (*account, *bytes, Vec::new()))
                .collect::<Vec<_>>(),
        )
        .await
    }

    async fn report_devices(
        &self,
        token: &str,
        usage: &[(u64, u64, Vec<[u8; 32]>)],
    ) -> Result<UsageReply, StatusCode> {
        self.call(
            &format!("/v1/nodes/{NODE}/usage"),
            Some(token),
            &UsageReport {
                revocations_after: None,
                version: None,
                accounts: usage
                    .iter()
                    .map(|(account_id, upstream_bytes, devices)| AccountUsage {
                        account_id: *account_id,
                        upstream_bytes: *upstream_bytes,
                        endpoints: devices
                            .iter()
                            .map(|device| yttt_relay_ticket::hex(&public_key(device)))
                            .collect(),
                    })
                    .collect(),
            },
        )
        .await
    }
}

fn proof(device: &[u8; 32], purpose: ProofPurpose, account: u64) -> DeviceProofBody {
    DeviceProofBody::from(&DeviceProof::sign(device, purpose, account, now()))
}

fn rand_suffix() -> u64 {
    use std::hash::{BuildHasher as _, RandomState};
    RandomState::new().hash_one(std::time::Instant::now())
}

async fn sqlx_admin(url: &str) -> sqlx::PgPool {
    sqlx::PgPool::connect(url).await.unwrap()
}

async fn sqlx_execute(pool: &sqlx::PgPool, statement: &str) {
    sqlx::raw_sql(statement).execute(pool).await.unwrap();
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn only_the_right_password_opens_a_session() {
    let harness = Harness::start().await;
    harness.account("alice@example.com").await;
    let wrong = Login {
        email: "alice@example.com".into(),
        password: "wrong horse".into(),
    };
    let unknown = Login {
        email: "nobody@example.com".into(),
        password: "correct horse".into(),
    };
    assert_eq!(
        harness.status("/v1/sessions", None, &wrong).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        harness.status("/v1/sessions", None, &unknown).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        harness
            .get::<AccountView>("/v1/accounts/me", "forged")
            .await,
        Err(StatusCode::UNAUTHORIZED)
    );
    let duplicate = CreateAccount {
        email: "Alice@Example.com".into(),
        password: "another password".into(),
        plan: "basic".into(),
        node: NODE.into(),
    };
    assert_eq!(
        harness.status("/v1/dev/accounts", None, &duplicate).await,
        StatusCode::CONFLICT
    );
    harness.control.shutdown().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn a_device_registers_only_with_proof_of_its_key_and_within_the_plan() {
    let harness = Harness::start().await;
    let (alice, alice_session) = harness.account("alice@example.com").await;
    let (bob, bob_session) = harness.account("bob@example.com").await;

    // A proof for another purpose, another account, or another key fails.
    let stolen = RegisterDevice {
        role: None,
        follows_host: None,
        label: "laptop".into(),
        proof: proof(&LAPTOP, ProofPurpose::IssueTicket, alice),
    };
    assert_eq!(
        harness
            .status("/v1/devices", Some(&alice_session), &stolen)
            .await,
        StatusCode::FORBIDDEN
    );
    let replayed = RegisterDevice {
        role: None,
        follows_host: None,
        label: "laptop".into(),
        proof: proof(&LAPTOP, ProofPurpose::RegisterDevice, bob),
    };
    assert_eq!(
        harness
            .status("/v1/devices", Some(&alice_session), &replayed)
            .await,
        StatusCode::FORBIDDEN
    );
    let mut claimed = proof(&LAPTOP, ProofPurpose::RegisterDevice, alice);
    claimed.endpoint_id = yttt_relay_ticket::hex(&public_key(&PHONE));
    let claimed = RegisterDevice {
        role: None,
        follows_host: None,
        label: "phone".into(),
        proof: claimed,
    };
    assert_eq!(
        harness
            .status("/v1/devices", Some(&alice_session), &claimed)
            .await,
        StatusCode::FORBIDDEN
    );

    assert_eq!(
        harness.register(&alice_session, alice, &LAPTOP).await,
        StatusCode::CREATED
    );
    assert_eq!(
        harness.register(&alice_session, alice, &LAPTOP).await,
        StatusCode::OK
    );
    assert_eq!(
        harness.register(&bob_session, bob, &LAPTOP).await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        harness.register(&alice_session, alice, &PHONE).await,
        StatusCode::CREATED
    );
    assert_eq!(
        harness.register(&alice_session, alice, &TABLET).await,
        StatusCode::FORBIDDEN
    );

    let devices: DeviceList = harness.get("/v1/devices", &alice_session).await.unwrap();
    assert_eq!(devices.devices.len(), 2);
    let removed = harness
        .http
        .delete(format!(
            "{}/v1/devices/{}",
            harness.base,
            yttt_relay_ticket::hex(&public_key(&PHONE))
        ))
        .bearer_auth(&alice_session)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(removed, StatusCode::NO_CONTENT);
    assert_eq!(
        harness.register(&alice_session, alice, &TABLET).await,
        StatusCode::CREATED
    );
    harness.control.shutdown().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn a_ticket_is_issued_only_to_a_registered_device_and_verifies_on_the_node() {
    let harness = Harness::start().await;
    let (alice, session) = harness.account("alice@example.com").await;
    assert_eq!(
        harness.ticket(&session, alice, &LAPTOP).await.unwrap_err(),
        StatusCode::FORBIDDEN
    );
    harness.register(&session, alice, &LAPTOP).await;

    let granted = harness.ticket(&session, alice, &LAPTOP).await.unwrap();
    assert_eq!(granted.node_id, NODE);
    assert_eq!(granted.relay_url, "http://127.0.0.1:3340");
    let verifier = TicketVerifier::new(&[harness.control.ticket_key()]).unwrap();
    let ticket = verifier
        .verify(&granted.token, &public_key(&LAPTOP), NODE, now())
        .unwrap();
    assert_eq!(ticket.account_id, alice);
    assert_eq!(ticket.rate_bps, 1_000_000);
    assert!(
        verifier
            .verify(&granted.token, &public_key(&PHONE), NODE, now())
            .is_err()
    );

    harness
        .status(
            &format!("/v1/dev/accounts/{alice}/suspension"),
            None,
            &SetSuspended { suspended: true },
        )
        .await;
    // Suspension ends the account's sessions, so the device cannot even ask.
    assert_eq!(
        harness.ticket(&session, alice, &LAPTOP).await.unwrap_err(),
        StatusCode::UNAUTHORIZED
    );
    harness.control.shutdown().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn usage_accumulates_and_decides_each_accounts_standing() {
    let harness = Harness::start().await;
    let (alice, alice_session) = harness.account("alice@example.com").await;
    let (bob, _) = harness.account("bob@example.com").await;
    harness.register(&alice_session, alice, &LAPTOP).await;

    assert_eq!(
        harness.report("wrong", &[(alice, 1)]).await.unwrap_err(),
        StatusCode::UNAUTHORIZED
    );

    let reply = harness
        .report(
            NODE_TOKEN,
            &[(alice, MONTHLY_BYTES / 2), (bob, 0), (999_999, 10)],
        )
        .await
        .unwrap();
    assert_eq!(
        reply.accounts,
        vec![
            AccountStanding {
                account_id: alice,
                standing: PLAN_RATE,
                revoked_endpoints: Vec::new(),
            },
            AccountStanding {
                account_id: bob,
                standing: PLAN_RATE,
                revoked_endpoints: Vec::new(),
            },
            AccountStanding {
                account_id: 999_999,
                standing: Standing::Suspended,
                revoked_endpoints: Vec::new(),
            },
        ]
    );

    let reply = harness
        .report(NODE_TOKEN, &[(alice, MONTHLY_BYTES / 2)])
        .await
        .unwrap();
    assert_eq!(
        reply.accounts[0].standing,
        Standing::Throttled {
            rate_bps: THROTTLED,
            burst_bytes: THROTTLED
        }
    );
    let me: AccountView = harness
        .get("/v1/accounts/me", &alice_session)
        .await
        .unwrap();
    assert_eq!(me.month_upstream_bytes, MONTHLY_BYTES);
    assert_eq!(me.devices, 1);
    // A ticket issued over quota already carries the throttled rate.
    let granted = harness
        .ticket(&alice_session, alice, &LAPTOP)
        .await
        .unwrap();
    let ticket = TicketVerifier::new(&[harness.control.ticket_key()])
        .unwrap()
        .verify(&granted.token, &public_key(&LAPTOP), NODE, now())
        .unwrap();
    assert_eq!(ticket.rate_bps, THROTTLED);

    harness
        .status(
            &format!("/v1/dev/accounts/{bob}/suspension"),
            None,
            &SetSuspended { suspended: true },
        )
        .await;
    let reply = harness.report(NODE_TOKEN, &[(bob, 0)]).await.unwrap();
    assert_eq!(reply.accounts[0].standing, Standing::Suspended);
    harness.control.shutdown().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn a_node_learns_which_connected_devices_left_their_account() {
    let harness = Harness::start().await;
    let (alice, alice_session) = harness.account("alice@example.com").await;
    let (bob, bob_session) = harness.account("bob@example.com").await;
    harness.register(&alice_session, alice, &LAPTOP).await;
    harness.register(&alice_session, alice, &PHONE).await;
    harness.register(&bob_session, bob, &TABLET).await;

    // The phone is removed; the tablet is someone else's device presented
    // under Alice's account.
    let removed = harness
        .http
        .delete(format!(
            "{}/v1/devices/{}",
            harness.base,
            yttt_relay_ticket::hex(&public_key(&PHONE))
        ))
        .bearer_auth(&alice_session)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(removed, StatusCode::NO_CONTENT);
    let reply = harness
        .report_devices(
            NODE_TOKEN,
            &[
                (alice, 0, vec![LAPTOP, PHONE, TABLET]),
                (bob, 0, vec![TABLET]),
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        reply.accounts[0].revoked_endpoints,
        vec![
            yttt_relay_ticket::hex(&public_key(&PHONE)),
            yttt_relay_ticket::hex(&public_key(&TABLET)),
        ]
    );
    assert!(reply.accounts[1].revoked_endpoints.is_empty());

    // A ticket is long-lived and records when it was issued.
    let granted = harness
        .ticket(&alice_session, alice, &LAPTOP)
        .await
        .unwrap();
    let ticket = TicketVerifier::new(&[harness.control.ticket_key()])
        .unwrap()
        .verify(&granted.token, &public_key(&LAPTOP), NODE, now())
        .unwrap();
    assert!(ticket.issued_unix.abs_diff(now()) < 60);
    assert!(ticket.expires_unix >= now() + 29 * 24 * 3600);
    harness.control.shutdown().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn a_node_cannot_bill_or_probe_accounts_of_another_node() {
    let harness =
        Harness::start_with_nodes(&[(NODE, NODE_TOKEN), ("other-1", "other-token")]).await;
    let (alice, alice_session) = harness.account("alice@example.com").await;
    harness.register(&alice_session, alice, &LAPTOP).await;

    // Alice lives on NODE; "other-1" reports her as if she were its own.
    let reply: UsageReply = harness
        .call(
            "/v1/nodes/other-1/usage",
            Some("other-token"),
            &UsageReport {
                revocations_after: None,
                version: None,
                accounts: vec![AccountUsage {
                    account_id: alice,
                    upstream_bytes: MONTHLY_BYTES * 10,
                    endpoints: vec![
                        yttt_relay_ticket::hex(&public_key(&LAPTOP)),
                        yttt_relay_ticket::hex(&public_key(&PHONE)),
                    ],
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(reply.accounts[0].standing, Standing::Suspended);
    assert!(reply.accounts[0].revoked_endpoints.is_empty());
    let me: AccountView = harness
        .get("/v1/accounts/me", &alice_session)
        .await
        .unwrap();
    assert_eq!(
        me.month_upstream_bytes, 0,
        "another node's report billed nothing"
    );
    harness.control.shutdown().await;
}

/// A browser: keeps the session cookie and may send the console header.
struct Browser<'a> {
    harness: &'a Harness,
    cookie: Option<String>,
}

impl Browser<'_> {
    async fn send(
        &mut self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
        console_header: bool,
    ) -> (StatusCode, serde_json::Value) {
        let mut request = self
            .harness
            .http
            .request(method, format!("{}{path}", self.harness.base));
        if let Some(cookie) = &self.cookie {
            request = request.header("cookie", cookie);
        }
        if console_header {
            request = request.header("x-yttt-web", "1");
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        if let Some(set) = response.headers().get("set-cookie") {
            let pair = set.to_str().unwrap().split(';').next().unwrap().to_string();
            self.cookie = (!pair.ends_with('=')).then_some(pair);
        }
        let status = response.status();
        let body = response.json().await.unwrap_or(serde_json::Value::Null);
        (status, body)
    }
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn a_browser_signs_up_and_keeps_a_cookie_session_that_forgery_cannot_use() {
    let harness = Harness::start().await;
    let mut browser = Browser {
        harness: &harness,
        cookie: None,
    };
    let signup = serde_json::json!({"email": "web@example.com", "password": "correct horse"});
    let (status, _) = browser
        .send(
            reqwest::Method::POST,
            "/v1/accounts",
            Some(signup.clone()),
            false,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "sign-up needs the console header"
    );
    let (status, body) = browser
        .send(reqwest::Method::POST, "/v1/accounts", Some(signup), true)
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["role"], "user");
    let cookie = browser.cookie.clone().expect("signing up opens a session");
    assert!(cookie.starts_with("yttt_session="));

    let (status, me) = browser
        .send(reqwest::Method::GET, "/v1/accounts/me", None, false)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["plan"], "basic");
    let (status, usage) = browser
        .send(reqwest::Method::GET, "/v1/accounts/me/usage", None, false)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(usage.as_array().unwrap().len(), 1);

    // A cross-site page can make the browser send the cookie, not the header.
    let change = serde_json::json!({"current": "correct horse", "new": "battery staple"});
    let (status, _) = browser
        .send(
            reqwest::Method::POST,
            "/v1/accounts/me/password",
            Some(change.clone()),
            false,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = browser
        .send(
            reqwest::Method::POST,
            "/v1/accounts/me/password",
            Some(change),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = browser
        .send(reqwest::Method::DELETE, "/v1/web/session", None, true)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let mut stale = Browser {
        harness: &harness,
        cookie: Some(cookie),
    };
    let (status, _) = stale
        .send(reqwest::Method::GET, "/v1/accounts/me", None, false)
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "signing out ends the session"
    );
    let login = serde_json::json!({"email": "web@example.com", "password": "battery staple"});
    let (status, _) = browser
        .send(reqwest::Method::POST, "/v1/web/session", Some(login), true)
        .await;
    assert_eq!(status, StatusCode::OK);
    harness.control.shutdown().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn guessing_passwords_is_slowed_for_every_login_door() {
    let harness = Harness::start().await;
    harness.account("victim@example.com").await;
    let mut statuses = Vec::new();
    for _ in 0..14 {
        statuses.push(
            harness
                .status(
                    "/v1/sessions",
                    None,
                    &Login {
                        email: "victim@example.com".into(),
                        password: "wrong horse".into(),
                    },
                )
                .await,
        );
    }
    assert!(statuses.contains(&StatusCode::UNAUTHORIZED));
    assert_eq!(statuses.last(), Some(&StatusCode::TOO_MANY_REQUESTS));
    harness.control.shutdown().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn only_administrators_manage_accounts_and_every_change_is_audited() {
    let harness = Harness::start().await;
    let (_, admin_token) = harness.account("admin@example.com").await;
    let (bob, bob_token) = harness.account("bob@example.com").await;
    let forbidden = harness
        .http
        .get(format!("{}/v1/admin/overview", harness.base))
        .bearer_auth(&admin_token)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(forbidden, StatusCode::FORBIDDEN);
    assert!(
        yttt_control::promote_admin(&harness.config, "Admin@Example.com")
            .await
            .unwrap()
    );
    let admin_cookie = harness.console("admin@example.com").await;
    let admin = |method: reqwest::Method, path: &str| {
        harness
            .http
            .request(method, format!("{}{path}", harness.base))
            .console(&admin_cookie)
    };
    let overview: serde_json::Value = admin(reqwest::Method::GET, "/v1/admin/overview")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(overview["accounts"], 2);
    assert_eq!(overview["nodes"][0]["id"], NODE);
    let page: serde_json::Value = admin(reqwest::Method::GET, "/v1/admin/accounts?q=BOB")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page["total"], 1);
    assert_eq!(page["accounts"][0]["email"], "bob@example.com");

    let me: AccountView = harness.get("/v1/accounts/me", &admin_token).await.unwrap();
    let demote_self = admin(
        reqwest::Method::PATCH,
        &format!("/v1/admin/accounts/{}", me.account_id),
    )
    .json(&serde_json::json!({"role": "user"}))
    .send()
    .await
    .unwrap()
    .status();
    assert_eq!(demote_self, StatusCode::FORBIDDEN);

    let suspended = admin(reqwest::Method::PATCH, &format!("/v1/admin/accounts/{bob}"))
        .json(&serde_json::json!({"suspended": true}))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(suspended, StatusCode::NO_CONTENT);
    assert_eq!(
        harness
            .get::<AccountView>("/v1/accounts/me", &bob_token)
            .await
            .unwrap_err(),
        StatusCode::UNAUTHORIZED,
        "suspension ends the account's sessions"
    );
    let audit: serde_json::Value = admin(reqwest::Method::GET, "/v1/admin/audit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(audit[0]["action"], "suspend");
    assert_eq!(audit[0]["admin_email"], "admin@example.com");
    harness.control.shutdown().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn a_disabled_node_drops_its_devices_and_restarts_keep_managed_nodes() {
    let harness = Harness::start().await;
    let (alice, _) = harness.account("alice@example.com").await;
    let pool = sqlx::PgPool::connect(&harness.config.database_url)
        .await
        .unwrap();
    sqlx::raw_sql(&format!(
        "UPDATE nodes SET state = 'disabled' WHERE id = '{NODE}'"
    ))
    .execute(&pool)
    .await
    .unwrap();
    let reply = harness.report(NODE_TOKEN, &[(alice, 0)]).await.unwrap();
    assert_eq!(reply.accounts[0].standing, Standing::Suspended);

    // The token rotates in the database; restarting with the old seed keeps it.
    sqlx::query("UPDATE nodes SET token_hash = $1, state = 'active'")
        .bind(sha2_hex("rotated-token"))
        .execute(&pool)
        .await
        .unwrap();
    let mut config = harness.config.clone();
    config.listen = "127.0.0.1:0".parse().unwrap();
    let restarted = Control::spawn(config).await.unwrap();
    assert_eq!(
        harness.report(NODE_TOKEN, &[(alice, 0)]).await.unwrap_err(),
        StatusCode::UNAUTHORIZED
    );
    assert!(harness.report("rotated-token", &[(alice, 0)]).await.is_ok());
    restarted.shutdown().await;
    harness.control.shutdown().await;
}

fn sha2_hex(text: &str) -> Vec<u8> {
    use sha2::Digest as _;
    sha2::Sha256::digest(text.as_bytes()).to_vec()
}

fn enrollment_token(command: &str) -> String {
    let start = command.find("YTTT_ENROLL='").unwrap() + "YTTT_ENROLL='".len();
    command[start..].split('\'').next().unwrap().to_string()
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn a_node_deploys_from_a_one_time_command_and_comes_online() {
    let harness = Harness::start().await;
    harness.account("admin@example.com").await;
    yttt_control::promote_admin(&harness.config, "admin@example.com")
        .await
        .unwrap();
    let admin_cookie = harness.console("admin@example.com").await;
    let admin = |method: reqwest::Method, path: &str| {
        harness
            .http
            .request(method, format!("{}{path}", harness.base))
            .console(&admin_cookie)
    };
    let created: serde_json::Value = admin(reqwest::Method::POST, "/v1/admin/nodes")
        .json(&serde_json::json!({"id": "osaka-1", "name": "Osaka"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(created["node"]["status"], "pending");
    let command = created["command"].as_str().unwrap().to_string();
    let first = enrollment_token(&command);

    // The command pins the installer it was generated with.
    let script = harness
        .http
        .get(format!("{}/install/relay.sh", harness.base))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(command.contains(&sha2_hex_string(&script)));
    assert!(script.contains("CONTROL='http://127.0.0.1'"));

    let enroll = |token: &str| {
        harness
            .http
            .post(format!("{}/v1/nodes/enroll", harness.base))
            .json(&serde_json::json!({"token": token, "port": 4433}))
            .send()
    };
    let enrolled: serde_json::Value = enroll(&first).await.unwrap().json().await.unwrap();
    assert_eq!(enrolled["node_id"], "osaka-1");
    assert_eq!(enrolled["url"], "http://127.0.0.1:4433");
    assert_eq!(
        enrolled["ticket_keys"][0],
        yttt_relay_ticket::hex(&harness.control.ticket_key())
    );
    let first_node_token = enrolled["node_token"].as_str().unwrap().to_string();
    assert_eq!(
        enroll(&first).await.unwrap().status(),
        StatusCode::FORBIDDEN,
        "single use"
    );

    // A regenerated command voids the old unused one and rotates the node token on use.
    let again: serde_json::Value =
        admin(reqwest::Method::POST, "/v1/admin/nodes/osaka-1/enrollment")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    let unused: serde_json::Value =
        admin(reqwest::Method::POST, "/v1/admin/nodes/osaka-1/enrollment")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    assert_eq!(
        enroll(&enrollment_token(again["command"].as_str().unwrap()))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let rotated: serde_json::Value = enroll(&enrollment_token(unused["command"].as_str().unwrap()))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let node_token = rotated["node_token"].as_str().unwrap().to_string();
    let report = |token: String| {
        harness
            .http
            .post(format!("{}/v1/nodes/osaka-1/usage", harness.base))
            .bearer_auth(token)
            .json(&UsageReport {
                revocations_after: None,
                version: Some("0.3.21".into()),
                accounts: Vec::new(),
            })
            .send()
    };
    assert_eq!(
        report(first_node_token).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(
        report(node_token.clone())
            .await
            .unwrap()
            .status()
            .is_success()
    );
    let nodes: serde_json::Value = admin(reqwest::Method::GET, "/v1/admin/nodes")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let osaka = nodes
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == "osaka-1")
        .unwrap();
    assert_eq!(osaka["status"], "online");
    assert_eq!(osaka["version"], "0.3.21");

    // Accounts move there, then the node is disabled: they are suspended on it.
    let (carol, _) = harness.account("carol@example.com").await;
    let moved = admin(
        reqwest::Method::PATCH,
        &format!("/v1/admin/accounts/{carol}"),
    )
    .json(&serde_json::json!({"node": "osaka-1"}))
    .send()
    .await
    .unwrap()
    .status();
    assert_eq!(moved, StatusCode::NO_CONTENT);
    assert_eq!(
        admin(reqwest::Method::DELETE, "/v1/admin/nodes/osaka-1")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    admin(reqwest::Method::PATCH, "/v1/admin/nodes/osaka-1")
        .json(&serde_json::json!({"disabled": true}))
        .send()
        .await
        .unwrap();
    let reply: UsageReply = harness
        .http
        .post(format!("{}/v1/nodes/osaka-1/usage", harness.base))
        .bearer_auth(&node_token)
        .json(&UsageReport {
            revocations_after: None,
            version: None,
            accounts: vec![AccountUsage {
                account_id: carol,
                upstream_bytes: 0,
                endpoints: Vec::new(),
            }],
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reply.accounts[0].standing, Standing::Suspended);
    let audit: serde_json::Value = admin(reqwest::Method::GET, "/v1/admin/audit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(audit[0]["action"], "disable_node");
    harness.control.shutdown().await;
}

fn sha2_hex_string(text: &str) -> String {
    sha2_hex(text)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn a_fresh_deployment_gets_its_first_administrator_before_any_node() {
    let harness = Harness::start_with_nodes(&[]).await;
    let sign_up = || {
        harness
            .http
            .post(format!("{}/v1/accounts", harness.base))
            .header("x-yttt-web", "1")
            .json(&serde_json::json!({"email": "web@example.com", "password": "correct horse"}))
            .send()
    };
    assert_eq!(
        sign_up().await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "no node is ready"
    );

    yttt_control::create_admin(&harness.config, "Admin@Example.com", "correct horse")
        .await
        .unwrap();
    assert!(
        yttt_control::create_admin(&harness.config, "admin@example.com", "correct horse")
            .await
            .is_err(),
        "the email is taken"
    );
    let session: SessionGranted = harness
        .call(
            "/v1/sessions",
            None,
            &Login {
                email: "admin@example.com".into(),
                password: "correct horse".into(),
            },
        )
        .await
        .unwrap();
    let me: AccountView = harness
        .get("/v1/accounts/me", &session.token)
        .await
        .unwrap();
    assert_eq!(me.role, "admin");

    // The sign-up node waits to be deployed from the console.
    let cookie = harness.console("admin@example.com").await;
    let nodes: serde_json::Value = harness
        .http
        .get(format!("{}/v1/admin/nodes", harness.base))
        .console(&cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(nodes[0]["id"], NODE);
    assert_eq!(nodes[0]["status"], "pending");
    let deploy: serde_json::Value = harness
        .http
        .post(format!("{}/v1/admin/nodes/{NODE}/enrollment", harness.base))
        .console(&cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let enrolled = harness
        .http
        .post(format!("{}/v1/nodes/enroll", harness.base))
        .json(&serde_json::json!({
            "token": enrollment_token(deploy["command"].as_str().unwrap()),
            "port": 3341,
        }))
        .send()
        .await
        .unwrap();
    assert!(enrolled.status().is_success());
    assert_eq!(sign_up().await.unwrap().status(), StatusCode::CREATED);
}

/// An administrator session and a request builder for it.
/// A console cookie for a new administrator.
async fn admin_session(harness: &Harness, email: &str) -> String {
    harness.account(email).await;
    assert!(
        yttt_control::promote_admin(&harness.config, email)
            .await
            .unwrap()
    );
    harness.console(email).await
}

async fn send(
    harness: &Harness,
    cookie: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut request = harness
        .http
        .request(method, format!("{}{path}", harness.base))
        .console(cookie);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    let body = response.json().await.unwrap_or(serde_json::Value::Null);
    (status, body)
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn plans_are_run_from_the_console_and_reach_nodes_on_their_next_report() {
    let harness = Harness::start().await;
    let admin = admin_session(&harness, "admin@example.com").await;
    let (alice, _) = harness.account("alice@example.com").await;
    let patch = reqwest::Method::PATCH;

    let (status, _) = send(
        &harness,
        &admin,
        reqwest::Method::POST,
        "/v1/admin/plans",
        Some(serde_json::json!({
            "id": "pro", "name": "Pro", "description": "For teams",
            "rate_bps": 8_000_000, "burst_bytes": 1_000_000,
            "monthly_bytes": 500_000_000_000u64, "throttled_rate_bps": 256_000,
            "max_devices": 20,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = send(
        &harness,
        &admin,
        reqwest::Method::POST,
        "/v1/admin/plans",
        Some(serde_json::json!({
            "id": "Bad ID", "name": "x", "rate_bps": 1, "burst_bytes": 1,
            "monthly_bytes": 1, "throttled_rate_bps": 1, "max_devices": 1,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A faster plan reaches the node with its next report.
    let (status, _) = send(
        &harness,
        &admin,
        patch.clone(),
        "/v1/admin/plans/basic",
        Some(serde_json::json!({"rate_bps": 3_000_000, "name": "Basic"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let reply = harness.report(NODE_TOKEN, &[(alice, 1)]).await.unwrap();
    assert_eq!(
        reply.accounts[0].standing,
        Standing::Active {
            rate_bps: 3_000_000,
            burst_bytes: 250_000
        }
    );

    // The sign-up plan cannot be archived or deleted; a plan in use cannot be deleted.
    let (status, _) = send(
        &harness,
        &admin,
        patch.clone(),
        "/v1/admin/plans/basic",
        Some(serde_json::json!({"archived": true})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = send(
        &harness,
        &admin,
        patch.clone(),
        "/v1/admin/settings",
        Some(serde_json::json!({"signup_plan": "pro"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send(
        &harness,
        &admin,
        patch.clone(),
        "/v1/admin/plans/basic",
        Some(serde_json::json!({"archived": true})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send(
        &harness,
        &admin,
        reqwest::Method::DELETE,
        "/v1/admin/plans/basic",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "accounts still use it");
    let (status, _) = send(
        &harness,
        &admin,
        patch.clone(),
        &format!("/v1/admin/accounts/{alice}"),
        Some(serde_json::json!({"plan": "basic"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an archived plan takes no one new"
    );

    // New sign-ups land on the new plan, and the account sees its names.
    let signup = harness
        .http
        .post(format!("{}/v1/accounts", harness.base))
        .header("x-yttt-web", "1")
        .json(&serde_json::json!({"email": "new@example.com", "password": "correct horse"}))
        .send()
        .await
        .unwrap();
    assert_eq!(signup.status(), StatusCode::CREATED);
    let session: SessionGranted = harness
        .call(
            "/v1/sessions",
            None,
            &Login {
                email: "new@example.com".into(),
                password: "correct horse".into(),
            },
        )
        .await
        .unwrap();
    let me: AccountView = harness
        .get("/v1/accounts/me", &session.token)
        .await
        .unwrap();
    assert_eq!((me.plan.as_str(), me.plan_name.as_str()), ("pro", "Pro"));
    assert_eq!(me.plan_description, "For teams");
    assert_eq!(me.rate_bps, 8_000_000);

    let (status, _) = send(
        &harness,
        &admin,
        patch.clone(),
        "/v1/admin/settings",
        Some(serde_json::json!({"signup_enabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let closed = harness
        .http
        .post(format!("{}/v1/accounts", harness.base))
        .header("x-yttt-web", "1")
        .json(&serde_json::json!({"email": "late@example.com", "password": "correct horse"}))
        .send()
        .await
        .unwrap();
    assert_eq!(closed.status(), StatusCode::NOT_FOUND);

    // A restart with the original configuration keeps the console's plans.
    let config = harness.config.clone();
    harness.control.shutdown().await;
    let control = Control::spawn(config).await.unwrap();
    let base = format!("http://{}", control.addr());
    let plans: serde_json::Value = harness
        .http
        .get(format!("{base}/v1/admin/plans"))
        .console(&admin)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let basic = plans
        .as_array()
        .unwrap()
        .iter()
        .find(|plan| plan["id"] == "basic")
        .unwrap();
    assert_eq!(basic["rate_bps"], 3_000_000);
    assert_eq!(basic["archived"], true);
    let audit: serde_json::Value = harness
        .http
        .get(format!("{base}/v1/admin/audit"))
        .console(&admin)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(audit.as_array().unwrap().iter().any(|entry| {
        entry["action"] == "change_plan"
            && entry["detail"]
                .as_str()
                .unwrap()
                .contains("rate_bps: 1000000 -> 3000000")
    }));
    control.shutdown().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn administrators_open_accounts_reset_passwords_and_see_daily_traffic() {
    let harness = Harness::start().await;
    let admin = admin_session(&harness, "admin@example.com").await;
    let (status, created) = send(
        &harness,
        &admin,
        reqwest::Method::POST,
        "/v1/admin/accounts",
        Some(serde_json::json!({
            "email": "Carol@Example.com", "password": "first password",
            "plan": "basic", "node": NODE,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let carol = created["account_id"].as_u64().unwrap();
    let login = |password: &str| Login {
        email: "carol@example.com".into(),
        password: password.into(),
    };
    let session: SessionGranted = harness
        .call("/v1/sessions", None, &login("first password"))
        .await
        .unwrap();

    let (status, _) = send(
        &harness,
        &admin,
        reqwest::Method::POST,
        &format!("/v1/admin/accounts/{carol}/password"),
        Some(serde_json::json!({"password": "second password"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        harness
            .get::<AccountView>("/v1/accounts/me", &session.token)
            .await
            .unwrap_err(),
        StatusCode::UNAUTHORIZED,
        "a reset signs the account out"
    );
    assert!(
        harness
            .call::<_, SessionGranted>("/v1/sessions", None, &login("first password"))
            .await
            .is_err()
    );
    let session: SessionGranted = harness
        .call("/v1/sessions", None, &login("second password"))
        .await
        .unwrap();

    harness.report(NODE_TOKEN, &[(carol, 700)]).await.unwrap();
    harness.report(NODE_TOKEN, &[(carol, 300)]).await.unwrap();
    let daily: serde_json::Value = harness
        .get("/v1/accounts/me/usage/daily", &session.token)
        .await
        .unwrap();
    let days = daily.as_array().unwrap();
    assert_eq!(days.len(), 30);
    assert_eq!(days.last().unwrap()["upstream_bytes"], 1000);
    assert_eq!(days[0]["upstream_bytes"], 0);
    let (_, site) = send(
        &harness,
        &admin,
        reqwest::Method::GET,
        "/v1/admin/usage/daily?days=7",
        None,
    )
    .await;
    assert_eq!(site.as_array().unwrap().len(), 7);
    assert_eq!(site[6]["upstream_bytes"], 1000);
    let (_, overview) = send(
        &harness,
        &admin,
        reqwest::Method::GET,
        "/v1/admin/overview",
        None,
    )
    .await;
    assert_eq!(overview["today_upstream_bytes"], 1000);
    assert_eq!(overview["accounts_today"], 2);
    assert_eq!(overview["nodes"][0]["month_bytes"], 1000);
    let (_, page) = send(
        &harness,
        &admin,
        reqwest::Method::GET,
        "/v1/admin/accounts?status=admin",
        None,
    )
    .await;
    assert_eq!(page["total"], 1);
    harness.control.shutdown().await;
}

/// RFC 7636 Appendix B.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const REDIRECT: &str = "http://127.0.0.1:53682/callback";

fn query_value(url: &str, name: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.to_string())
    })
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn the_app_signs_in_through_the_browser_and_gets_only_a_device_session() {
    let harness = Harness::start().await;
    let cookie = admin_session(&harness, "admin@example.com").await;
    let authorize = |body: serde_json::Value| {
        send(
            &harness,
            &cookie,
            reqwest::Method::POST,
            "/v1/oauth/authorize",
            Some(body),
        )
    };
    let request = |allow: Option<bool>, redirect: &str| {
        serde_json::json!({
            "redirect_uri": redirect,
            "state": "app-state",
            "code_challenge": CHALLENGE,
            "code_challenge_method": "S256",
            "label": "laptop",
            "allow": allow,
        })
    };

    // The page checks the request first and never follows a foreign redirect.
    let (status, checked) = authorize(request(None, REDIRECT)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(checked["label"], "laptop");
    assert!(checked.get("redirect").is_none());
    let (status, _) = authorize(request(Some(true), "http://127.0.0.1:1@evil.example/cb")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let mut plain = request(Some(true), REDIRECT);
    plain["code_challenge_method"] = "plain".into();
    assert_eq!(authorize(plain).await.0, StatusCode::BAD_REQUEST);

    // Denying goes back to the app with the state.
    let (_, denied) = authorize(request(Some(false), REDIRECT)).await;
    let denied = denied["redirect"].as_str().unwrap();
    assert!(denied.starts_with(REDIRECT));
    assert_eq!(
        query_value(denied, "error").as_deref(),
        Some("access_denied")
    );
    assert_eq!(query_value(denied, "state").as_deref(), Some("app-state"));

    let exchange = |code: &str, verifier: &str| {
        harness
            .http
            .post(format!("{}/v1/oauth/token", harness.base))
            .json(&serde_json::json!({
                "grant_type": "authorization_code",
                "code": code,
                "code_verifier": verifier,
                "redirect_uri": REDIRECT,
            }))
            .send()
    };
    let code_from = |answer: serde_json::Value| {
        let redirect = answer["redirect"].as_str().unwrap().to_string();
        assert_eq!(
            query_value(&redirect, "state").as_deref(),
            Some("app-state")
        );
        query_value(&redirect, "code").unwrap()
    };

    // A code without its verifier is worthless, and burns.
    let (_, allowed) = authorize(request(Some(true), REDIRECT)).await;
    let stolen = code_from(allowed);
    let wrong = exchange(&stolen, &"x".repeat(43)).await.unwrap();
    assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        exchange(&stolen, VERIFIER).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );

    let (_, allowed) = authorize(request(Some(true), REDIRECT)).await;
    let code = code_from(allowed);
    let granted: SessionGranted = exchange(&code, VERIFIER)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let again = exchange(&code, VERIFIER).await.unwrap();
    assert_eq!(again.status(), StatusCode::BAD_REQUEST, "a code works once");

    // The session does what a device does …
    let me: AccountView = harness
        .get("/v1/accounts/me", &granted.token)
        .await
        .unwrap();
    assert_eq!(me.email, "admin@example.com");
    assert_eq!(
        harness
            .register(&granted.token, granted.account_id, &LAPTOP)
            .await,
        StatusCode::CREATED
    );
    // … and nothing a console session does; in a cookie it is no session.
    let in_cookie = harness
        .http
        .get(format!("{}/v1/accounts/me", harness.base))
        .header("cookie", format!("yttt_session={}", granted.token))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(in_cookie, StatusCode::UNAUTHORIZED);
    let as_cookie = format!("yttt_session={}", granted.token);
    for (method, path) in [
        (reqwest::Method::GET, "/v1/admin/overview"),
        (reqwest::Method::POST, "/v1/accounts/me/password"),
        (reqwest::Method::POST, "/v1/oauth/authorize"),
        (reqwest::Method::POST, "/v1/oauth/device/confirm"),
    ] {
        let body = serde_json::json!({"current": "correct horse", "new": "another horse", "user_code": "ABCD-EFGH"});
        for request in [
            harness
                .http
                .request(method.clone(), format!("{}{path}", harness.base))
                .bearer_auth(&granted.token),
            harness
                .http
                .request(method.clone(), format!("{}{path}", harness.base))
                .console(&as_cookie),
        ] {
            let status = request.json(&body).send().await.unwrap().status();
            assert!(
                [StatusCode::FORBIDDEN, StatusCode::UNAUTHORIZED].contains(&status),
                "{method} {path}: {status}"
            );
        }
    }
    harness.control.shutdown().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn a_headless_device_signs_in_once_someone_confirms_its_short_code() {
    let harness = Harness::start().await;
    harness.account("alice@example.com").await;
    let cookie = harness.console("alice@example.com").await;
    let start = || async {
        let response = harness
            .http
            .post(format!("{}/v1/oauth/device", harness.base))
            .json(&serde_json::json!({"label": "nas"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        response.json::<serde_json::Value>().await.unwrap()
    };
    let shared = &harness;
    let poll = |device_code: String| async move {
        let response = shared
            .http
            .post(format!("{}/v1/oauth/token", shared.base))
            .json(&serde_json::json!({"grant_type": "device_code", "device_code": device_code}))
            .send()
            .await
            .unwrap();
        let status = response.status();
        (status, response.json::<serde_json::Value>().await.unwrap())
    };
    let confirm = |body: serde_json::Value| {
        send(
            &harness,
            &cookie,
            reqwest::Method::POST,
            "/v1/oauth/device/confirm",
            Some(body),
        )
    };

    let grant = start().await;
    let device_code = grant["device_code"].as_str().unwrap().to_string();
    let user_code = grant["user_code"].as_str().unwrap().to_string();
    assert_eq!(user_code.len(), 9);
    assert_eq!(grant["verification_uri"], "http://127.0.0.1/device");
    assert_eq!(grant["interval"], 5);

    let (status, body) = poll(device_code.clone()).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("authorization_pending"))
    );
    let (_, body) = poll(device_code.clone()).await;
    assert_eq!(body["error"], "slow_down");

    // Whoever confirms sees what asked, typed loosely.
    let (status, _) = confirm(serde_json::json!({"user_code": "ZZZZ-ZZZZ"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, seen) =
        confirm(serde_json::json!({"user_code": user_code.to_lowercase().replace('-', " ")})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(seen["label"], "nas");
    assert_eq!(seen["requester_ip"], "127.0.0.1");
    let (status, _) = confirm(serde_json::json!({"user_code": user_code, "allow": true})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = confirm(serde_json::json!({"user_code": user_code, "allow": true})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a code is confirmed once");

    let (status, granted) = poll(device_code.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let token = granted["token"].as_str().unwrap();
    let me: AccountView = harness.get("/v1/accounts/me", token).await.unwrap();
    assert_eq!(me.email, "alice@example.com");
    let (_, body) = poll(device_code).await;
    assert_eq!(
        body["error"], "invalid_grant",
        "the session is handed out once"
    );

    // Denied stays denied.
    let grant = start().await;
    let device_code = grant["device_code"].as_str().unwrap().to_string();
    confirm(serde_json::json!({"user_code": grant["user_code"], "allow": false})).await;
    let (_, body) = poll(device_code.clone()).await;
    assert_eq!(body["error"], "access_denied");
    let (_, body) = poll(device_code).await;
    assert_eq!(body["error"], "invalid_grant");
    harness.control.shutdown().await;
}

/// A stand-in for GitHub. The authorization code names the person it signs
/// in as `id:email:verified`, and doubles as the access token.
async fn fake_github() -> String {
    use axum::{
        Form, Json, Router,
        http::HeaderMap,
        routing::{get, post},
    };
    fn person(headers: &HeaderMap) -> (u64, String, bool) {
        let token = headers["authorization"]
            .to_str()
            .unwrap()
            .trim_start_matches("Bearer ");
        let mut parts = token.split(':');
        (
            parts.next().unwrap().parse().unwrap(),
            parts.next().unwrap().to_string(),
            parts.next().unwrap() == "true",
        )
    }
    let router = Router::new()
        .route(
            "/token",
            post(|headers: HeaderMap, Form(form): Form<std::collections::HashMap<String, String>>| async move {
                assert_eq!(headers["accept"], "application/json");
                assert_eq!(form["client_secret"], "github-secret");
                assert_eq!(form["redirect_uri"], "http://127.0.0.1/v1/auth/github/callback");
                assert!(form["code_verifier"].len() >= 43);
                Json(serde_json::json!({"access_token": form["code"]}))
            }),
        )
        .route(
            "/user",
            get(|headers: HeaderMap| async move { Json(serde_json::json!({"id": person(&headers).0})) }),
        )
        .route(
            "/user/emails",
            get(|headers: HeaderMap| async move {
                let (_, email, verified) = person(&headers);
                Json(serde_json::json!([
                    {"email": "old@example.com", "primary": false, "verified": true},
                    {"email": email, "primary": true, "verified": verified},
                ]))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

/// A browser that does not follow redirects, so the test plays GitHub.
struct OAuthBrowser<'a> {
    harness: &'a Harness,
    http: reqwest::Client,
}

impl OAuthBrowser<'_> {
    fn new(harness: &Harness) -> OAuthBrowser<'_> {
        OAuthBrowser {
            harness,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        }
    }

    /// Leaves for GitHub and returns the state and the cookie it set.
    async fn start(&self, next: &str) -> (String, String) {
        let response = self
            .http
            .get(format!(
                "{}/v1/auth/github/start?next={next}",
                self.harness.base
            ))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_redirection());
        let location = response.headers()["location"].to_str().unwrap().to_string();
        assert!(location.contains("scope=read%3Auser+user%3Aemail"));
        assert!(location.contains("code_challenge_method=S256"));
        (
            query_value(&location, "state").unwrap(),
            set_cookie(&response, "yttt_oauth"),
        )
    }

    /// Comes back from GitHub as `person`; returns where the page goes next
    /// and the session cookie, if one was set.
    async fn back(
        &self,
        state: &str,
        cookie: &str,
        person: &str,
    ) -> (StatusCode, String, Option<String>) {
        let response = self
            .http
            .get(format!(
                "{}/v1/auth/github/callback?code={person}&state={state}",
                self.harness.base
            ))
            .header("cookie", cookie)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let session = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .map(|value| {
                value
                    .to_str()
                    .unwrap()
                    .split(';')
                    .next()
                    .unwrap()
                    .to_string()
            })
            .find(|pair| pair.starts_with("yttt_session=") && !pair.ends_with('='));
        let body = response.text().await.unwrap();
        let next = body
            .split("url=")
            .nth(1)
            .map(|rest| rest.split('"').next().unwrap().replace("&amp;", "&"))
            .unwrap_or_default();
        (status, next, session)
    }

    async fn sign_in(&self, person: &str) -> (String, Option<String>) {
        let (state, cookie) = self.start("/devices").await;
        let (status, next, session) = self.back(&state, &cookie, person).await;
        assert_eq!(status, StatusCode::OK);
        (next, session)
    }
}

fn set_cookie(response: &reqwest::Response, name: &str) -> String {
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|value| {
            value
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_string()
        })
        .find(|pair| pair.starts_with(&format!("{name}=")))
        .unwrap()
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn the_console_signs_in_with_github_and_links_it_only_to_its_owner() {
    let github = fake_github().await;
    let harness = Harness::start_with(&[(NODE, NODE_TOKEN)], |config| {
        config.oauth.github = Some(yttt_control::ProviderConfig {
            client_id: "github-client".into(),
            client_secret: "github-secret".into(),
            authorize_url: Some(format!("{github}/authorize")),
            token_url: Some(format!("{github}/token")),
            api_url: Some(github.clone()),
        });
    })
    .await;
    let browser = OAuthBrowser::new(&harness);
    let providers: serde_json::Value = harness
        .http
        .get(format!("{}/v1/auth/providers", harness.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(providers, serde_json::json!(["github"]));

    // A new person gets an account without a password.
    let (next, session) = browser.sign_in("42:Carol@Example.com:true").await;
    assert_eq!(next, "/devices");
    let carol = session.unwrap();
    let (_, methods) = send(
        &harness,
        &carol,
        reqwest::Method::GET,
        "/v1/accounts/me/sign-in",
        None,
    )
    .await;
    assert_eq!(methods["has_password"], false);
    assert_eq!(methods["may_set_password"], true);
    assert_eq!(methods["identities"][0]["provider"], "github");
    assert_eq!(methods["identities"][0]["email"], "carol@example.com");
    // Signing in again finds the same account.
    let (_, again) = browser.sign_in("42:renamed@example.com:true").await;
    let (_, me) = send(
        &harness,
        &again.unwrap(),
        reqwest::Method::GET,
        "/v1/accounts/me",
        None,
    )
    .await;
    assert_eq!(me["email"], "carol@example.com");
    // Its only way in cannot be unlinked; right after signing in it may set
    // a first password without knowing one.
    let (status, _) = send(
        &harness,
        &carol,
        reqwest::Method::DELETE,
        "/v1/accounts/me/identities/github",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = send(
        &harness,
        &carol,
        reqwest::Method::POST,
        "/v1/accounts/me/password",
        Some(serde_json::json!({"new": "carol's horse"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The state must come back with the browser that left, once.
    let (state, cookie) = browser.start("/").await;
    let (status, _, _) = browser
        .back(&state, "yttt_oauth=forged", "43:x@example.com:true")
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = browser.back(&state, &cookie, "43:x@example.com:true").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = browser.back(&state, &cookie, "43:x@example.com:true").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // An email that already has an account is not taken over …
    harness.account("alice@example.com").await;
    let (next, session) = browser.sign_in("77:alice@example.com:true").await;
    assert_eq!(next, "/login?oauth_error=email_taken&provider=github");
    assert!(session.is_none());
    let (next, _) = browser.sign_in("78:dave@example.com:false").await;
    assert_eq!(next, "/login?oauth_error=unverified_email&provider=github");

    // … its owner links it after signing in.
    let alice = harness.console("alice@example.com").await;
    let linking = browser
        .http
        .post(format!("{}/v1/auth/github/link", harness.base))
        .console(&alice)
        .json(&serde_json::json!({"next": "/account"}))
        .send()
        .await
        .unwrap();
    assert_eq!(linking.status(), StatusCode::OK);
    let state_cookie = set_cookie(&linking, "yttt_oauth");
    let redirect: serde_json::Value = linking.json().await.unwrap();
    let state = query_value(redirect["redirect"].as_str().unwrap(), "state").unwrap();
    let (_, next, session) = browser
        .back(&state, &state_cookie, "77:alice@example.com:true")
        .await;
    assert_eq!(next, "/account");
    assert!(session.is_none(), "linking keeps the current session");
    let (_, session) = browser.sign_in("77:alice@example.com:true").await;
    let (_, me) = send(
        &harness,
        &session.unwrap(),
        reqwest::Method::GET,
        "/v1/accounts/me",
        None,
    )
    .await;
    assert_eq!(me["email"], "alice@example.com");
    // Nobody else can link the same GitHub account.
    let bob_cookie = {
        harness.account("bob@example.com").await;
        harness.console("bob@example.com").await
    };
    let linking = browser
        .http
        .post(format!("{}/v1/auth/github/link", harness.base))
        .console(&bob_cookie)
        .json(&serde_json::json!({"next": "/account"}))
        .send()
        .await
        .unwrap();
    let state_cookie = set_cookie(&linking, "yttt_oauth");
    let redirect: serde_json::Value = linking.json().await.unwrap();
    let state = query_value(redirect["redirect"].as_str().unwrap(), "state").unwrap();
    let (_, next, _) = browser
        .back(&state, &state_cookie, "77:alice@example.com:true")
        .await;
    assert_eq!(next, "/account?oauth_error=already_linked&provider=github");
    // With a password left, Alice may unlink it.
    let (status, _) = send(
        &harness,
        &alice,
        reqwest::Method::DELETE,
        "/v1/accounts/me/identities/github",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    harness.control.shutdown().await;
}

const HOST: [u8; 32] = [31; 32];
const HOST_CLIENT: [u8; 32] = [32; 32];
const SECOND_HOST: [u8; 32] = [33; 32];
const SECOND_CLIENT: [u8; 32] = [34; 32];

fn key_hex(device: &[u8; 32]) -> String {
    yttt_relay_ticket::hex(&public_key(device))
}

/// Registers `device` as a Host, or as a Client of `follows`.
async fn place(
    harness: &Harness,
    session: &str,
    account: u64,
    device: &[u8; 32],
    follows: Option<&[u8; 32]>,
) -> StatusCode {
    harness
        .status(
            "/v1/devices",
            Some(session),
            &RegisterDevice {
                label: "device".into(),
                proof: proof(device, ProofPurpose::RegisterDevice, account),
                role: Some(if follows.is_some() {
                    DeviceRole::Client
                } else {
                    DeviceRole::Host
                }),
                follows_host: follows.map(key_hex),
            },
        )
        .await
}

async fn ticket_to(
    harness: &Harness,
    session: &str,
    account: u64,
    device: &[u8; 32],
    host: Option<&[u8; 32]>,
) -> Result<TicketGranted, StatusCode> {
    harness
        .call(
            "/v1/tickets",
            Some(session),
            &TicketRequest {
                proof: proof(device, ProofPurpose::IssueTicket, account),
                host: host.map(key_hex),
            },
        )
        .await
}

/// Asks to move `host`, proving with `signer`'s key, as the app does.
async fn switch(
    harness: &Harness,
    session: &str,
    account: u64,
    host: &[u8; 32],
    signer: &[u8; 32],
    node: &str,
) -> StatusCode {
    harness
        .http
        .put(format!(
            "{}/v1/devices/{}/node",
            harness.base,
            key_hex(host)
        ))
        .bearer_auth(session)
        .json(&SwitchNode {
            node_id: node.into(),
            proof: Some(proof(signer, ProofPurpose::SwitchNode, account)),
        })
        .send()
        .await
        .unwrap()
        .status()
}

async fn report_on(
    harness: &Harness,
    node: &str,
    token: &str,
    usage: &[(u64, u64, &[[u8; 32]])],
    revocations_after: Option<u64>,
) -> UsageReply {
    harness
        .call(
            &format!("/v1/nodes/{node}/usage"),
            Some(token),
            &UsageReport {
                revocations_after,
                version: None,
                accounts: usage
                    .iter()
                    .map(|(account_id, upstream_bytes, devices)| AccountUsage {
                        account_id: *account_id,
                        upstream_bytes: *upstream_bytes,
                        endpoints: devices.iter().map(key_hex).collect(),
                    })
                    .collect(),
            },
        )
        .await
        .unwrap()
}

async fn with_room(nodes: &[(&str, &str)]) -> Harness {
    Harness::start_with(nodes, |config| config.plans[0].max_devices = 10).await
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn a_client_relays_through_its_hosts_node_and_follows_it_when_the_account_moves() {
    let harness = with_room(&[(NODE, NODE_TOKEN), ("other-1", "other-token")]).await;
    let (alice, session) = harness.account("alice@example.com").await;
    assert_eq!(
        place(&harness, &session, alice, &HOST, None).await,
        StatusCode::CREATED
    );
    assert_eq!(
        place(&harness, &session, alice, &HOST_CLIENT, Some(&HOST)).await,
        StatusCode::CREATED
    );
    // A Client of a Host that is not on the account is not registered at all.
    assert_eq!(
        place(
            &harness,
            &session,
            alice,
            &SECOND_CLIENT,
            Some(&SECOND_HOST)
        )
        .await,
        StatusCode::NOT_FOUND
    );
    let devices: DeviceList = harness.get("/v1/devices", &session).await.unwrap();
    assert_eq!(devices.devices.len(), 2);
    let client = devices
        .devices
        .iter()
        .find(|device| device.endpoint_id == key_hex(&HOST_CLIENT))
        .unwrap();
    assert_eq!(client.role, DeviceRole::Client);
    assert_eq!(
        client.follows_host.as_deref(),
        Some(key_hex(&HOST).as_str())
    );
    // Roles are fixed.
    assert_eq!(
        place(&harness, &session, alice, &HOST_CLIENT, None).await,
        StatusCode::CONFLICT
    );

    let ticket = ticket_to(&harness, &session, alice, &HOST_CLIENT, Some(&HOST))
        .await
        .unwrap();
    assert_eq!(ticket.node_id, NODE);
    assert_eq!(
        ticket_to(&harness, &session, alice, &HOST_CLIENT, Some(&SECOND_HOST))
            .await
            .unwrap_err(),
        StatusCode::CONFLICT
    );
    let nodes: Vec<NodeChoice> = harness.get("/v1/nodes", &session).await.unwrap();
    assert_eq!(nodes.len(), 2);

    // Only the Host's own key moves it; with one node, the account moves.
    assert_eq!(
        switch(&harness, &session, alice, &HOST, &HOST_CLIENT, "other-1").await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        switch(&harness, &session, alice, &HOST, &HOST, "other-1").await,
        StatusCode::OK
    );
    let me: AccountView = harness.get("/v1/accounts/me", &session).await.unwrap();
    assert_eq!(me.node_id, "other-1");
    let ticket = ticket_to(&harness, &session, alice, &HOST_CLIENT, Some(&HOST))
        .await
        .unwrap();
    assert_eq!(ticket.node_id, "other-1");
    assert_eq!(
        switch(&harness, &session, alice, &HOST, &HOST, NODE).await,
        StatusCode::CONFLICT,
        "a Host waits between its own moves"
    );

    // The old node is told to refuse both, once, and bills nothing more.
    let reply = report_on(&harness, NODE, NODE_TOKEN, &[(alice, 500, &[HOST])], None).await;
    assert_eq!(reply.accounts[0].standing, Standing::Suspended);
    let mut revoked: Vec<_> = reply
        .revocations
        .iter()
        .map(|revocation| revocation.endpoint_id.clone())
        .collect();
    revoked.sort();
    let mut expected = vec![key_hex(&HOST), key_hex(&HOST_CLIENT)];
    expected.sort();
    assert_eq!(revoked, expected);
    let last = reply
        .revocations
        .iter()
        .map(|revocation| revocation.id)
        .max();
    let again = report_on(&harness, NODE, NODE_TOKEN, &[], last).await;
    assert!(again.revocations.is_empty());
    let me: AccountView = harness.get("/v1/accounts/me", &session).await.unwrap();
    assert_eq!(me.month_upstream_bytes, 0);

    // The new node bills and steers the account.
    let reply = report_on(
        &harness,
        "other-1",
        "other-token",
        &[(alice, 500, &[HOST])],
        None,
    )
    .await;
    assert!(matches!(
        reply.accounts[0].standing,
        Standing::Active { .. }
    ));
    assert!(reply.accounts[0].revoked_endpoints.is_empty());
    let me: AccountView = harness.get("/v1/accounts/me", &session).await.unwrap();
    assert_eq!(me.month_upstream_bytes, 500);
    let devices: DeviceList = harness.get("/v1/devices", &session).await.unwrap();
    assert!(
        devices
            .devices
            .iter()
            .any(|device| device.endpoint_id == key_hex(&HOST) && device.online)
    );
    harness.control.shutdown().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn hosts_spread_over_as_many_nodes_as_the_plan_allows_and_return_when_it_shrinks() {
    let harness = with_room(&[
        (NODE, NODE_TOKEN),
        ("other-1", "other-token"),
        ("far-1", "far-token"),
    ])
    .await;
    let admin = admin_session(&harness, "admin@example.com").await;
    let (status, _) = send(
        &harness,
        &admin,
        reqwest::Method::PATCH,
        "/v1/admin/plans/basic",
        Some(serde_json::json!({"max_nodes": 2})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (alice, session) = harness.account("alice@example.com").await;
    for (device, follows) in [
        (&HOST, None),
        (&HOST_CLIENT, Some(&HOST)),
        (&SECOND_HOST, None),
        (&SECOND_CLIENT, Some(&SECOND_HOST)),
    ] {
        assert_eq!(
            place(&harness, &session, alice, device, follows).await,
            StatusCode::CREATED
        );
    }

    // Two nodes at once: the default and one more.
    assert_eq!(
        switch(&harness, &session, alice, &HOST, &HOST, "other-1").await,
        StatusCode::OK
    );
    assert_eq!(
        switch(
            &harness,
            &session,
            alice,
            &SECOND_HOST,
            &SECOND_HOST,
            "far-1"
        )
        .await,
        StatusCode::CONFLICT
    );
    let ticket = ticket_to(&harness, &session, alice, &HOST_CLIENT, Some(&HOST))
        .await
        .unwrap();
    assert_eq!(ticket.node_id, "other-1");
    let second = ticket_to(
        &harness,
        &session,
        alice,
        &SECOND_CLIENT,
        Some(&SECOND_HOST),
    )
    .await
    .unwrap();
    assert_eq!(second.node_id, NODE, "the other Host stayed");

    // The plan loses other-1: the Host goes home at its next ticket.
    let (status, _) = send(
        &harness,
        &admin,
        reqwest::Method::PATCH,
        "/v1/admin/plans/basic",
        Some(serde_json::json!({"nodes": [NODE, "far-1"]})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let ticket = ticket_to(&harness, &session, alice, &HOST, None)
        .await
        .unwrap();
    assert_eq!(ticket.node_id, NODE);
    let reply = report_on(&harness, "other-1", "other-token", &[], None).await;
    assert_eq!(reply.revocations.len(), 2, "the Host and its Client");

    // The console moves any Host, without waiting.
    let console = harness.console("alice@example.com").await;
    for host in [&SECOND_HOST, &SECOND_HOST] {
        let status = harness
            .http
            .put(format!(
                "{}/v1/devices/{}/node",
                harness.base,
                key_hex(host)
            ))
            .console(&console)
            .json(&serde_json::json!({"node_id": "far-1"}))
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, StatusCode::OK);
    }
    let second = ticket_to(
        &harness,
        &session,
        alice,
        &SECOND_CLIENT,
        Some(&SECOND_HOST),
    )
    .await
    .unwrap();
    assert_eq!(second.node_id, "far-1");
    harness.control.shutdown().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn a_full_node_takes_no_new_hosts_or_accounts_and_its_load_is_kept() {
    let harness = with_room(&[(NODE, NODE_TOKEN), ("other-1", "other-token")]).await;
    let admin = admin_session(&harness, "admin@example.com").await;
    let (alice, session) = harness.account("alice@example.com").await;
    assert_eq!(
        place(&harness, &session, alice, &HOST, None).await,
        StatusCode::CREATED
    );
    // other-1 has two devices connected and room for two.
    let (status, _) = send(
        &harness,
        &admin,
        reqwest::Method::PATCH,
        "/v1/admin/nodes/other-1",
        Some(serde_json::json!({"max_connections": 2, "region": "Tokyo"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    report_on(
        &harness,
        "other-1",
        "other-token",
        &[(alice, 700, &[HOST_CLIENT, SECOND_CLIENT])],
        None,
    )
    .await;

    let nodes: Vec<NodeChoice> = harness.get("/v1/nodes", &session).await.unwrap();
    let other = nodes.iter().find(|node| node.id == "other-1").unwrap();
    assert_eq!((other.load_percent, other.full), (Some(100), true));
    assert_eq!(other.region, "Tokyo");
    assert_eq!(
        switch(&harness, &session, alice, &HOST, &HOST, "other-1").await,
        StatusCode::CONFLICT
    );

    let (_, stats) = send(
        &harness,
        &admin,
        reqwest::Method::GET,
        "/v1/admin/nodes/other-1/stats",
        None,
    )
    .await;
    assert_eq!(stats["samples"][0]["connections"], 2);
    assert_eq!(stats["samples"][0]["upstream_bytes"], 700);
    assert_eq!(stats["days"].as_array().unwrap().len(), 30);
    assert_eq!(stats["days"][29]["connections_max"], 2);
    let (_, overview) = send(
        &harness,
        &admin,
        reqwest::Method::GET,
        "/v1/admin/overview",
        None,
    )
    .await;
    let listed = overview["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == "other-1")
        .unwrap()
        .clone();
    assert_eq!(listed["max_connections"], 2);

    // The sign-up node fills up: new accounts go to the one with room.
    let (status, _) = send(
        &harness,
        &admin,
        reqwest::Method::PATCH,
        "/v1/admin/nodes/other-1",
        Some(serde_json::json!({"max_connections": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send(
        &harness,
        &admin,
        reqwest::Method::PATCH,
        &format!("/v1/admin/nodes/{NODE}"),
        Some(serde_json::json!({"max_connections": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    report_on(&harness, NODE, NODE_TOKEN, &[(alice, 0, &[HOST])], None).await;
    let signed_up = harness
        .http
        .post(format!("{}/v1/accounts", harness.base))
        .header("x-yttt-web", "1")
        .json(&serde_json::json!({"email": "new@example.com", "password": "correct horse"}))
        .send()
        .await
        .unwrap();
    assert_eq!(signed_up.status(), StatusCode::CREATED);
    let cookie = signed_up.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let (_, me) = send(
        &harness,
        &cookie,
        reqwest::Method::GET,
        "/v1/accounts/me",
        None,
    )
    .await;
    assert_eq!(me["node_id"], "other-1");
    harness.control.shutdown().await;
}
