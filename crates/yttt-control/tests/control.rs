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
        DeviceProofBody, Login, RegisterDevice, SessionGranted, SetSuspended, Standing,
        TicketGranted, TicketRequest, UsageReply, UsageReport,
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

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

struct Harness {
    control: Control,
    http: reqwest::Client,
    base: String,
}

impl Harness {
    async fn start() -> Self {
        Self::start_with_nodes(&[(NODE, NODE_TOKEN)]).await
    }

    async fn start_with_nodes(nodes: &[(&str, &str)]) -> Self {
        let server = std::env::var("YTTT_TEST_DATABASE_URL")
            .expect("YTTT_TEST_DATABASE_URL names a PostgreSQL server");
        let name = format!("yttt_control_test_{:016x}", rand_suffix());
        let admin = sqlx_admin(&server).await;
        sqlx_execute(&admin, &format!("CREATE DATABASE {name}")).await;
        let (prefix, _) = server.rsplit_once('/').unwrap();
        let control = Control::spawn(ControlConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: format!("{prefix}/{name}"),
            ticket_secret: yttt_relay_ticket::hex(&[1; 32]),
            ticket_ttl_secs: 30 * 24 * 3600,
            session_ttl_secs: 3600,
            dev_mode: true,
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
        })
        .await
        .unwrap();
        let base = format!("http://{}", control.addr());
        Self {
            control,
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

    async fn register(&self, session: &str, account: u64, device: &[u8; 32]) -> StatusCode {
        self.status(
            "/v1/devices",
            Some(session),
            &RegisterDevice {
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
    assert_eq!(
        harness.ticket(&session, alice, &LAPTOP).await.unwrap_err(),
        StatusCode::FORBIDDEN
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
