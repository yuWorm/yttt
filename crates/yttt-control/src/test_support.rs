//! A control plane with reporting relay nodes for integration tests in other
//! crates. It runs on a fresh SQLite file, or on a fresh PostgreSQL database
//! when `YTTT_TEST_DATABASE_URL` names a server.

use yttt_relay::{ControlConfig as NodeControl, NodeConfig, RelayNode};
use yttt_relay_ticket::api::{AccountCreated, CreateAccount};

use crate::{Control, ControlConfig, NodeEntry, PlanConfig};

/// The email and password of the account [`TestRelay::create_account`] makes.
pub const EMAIL: &str = "owner@example.com";
pub const PASSWORD: &str = "correct horse";
/// The plan every test account is on.
pub const PLAN: &str = "basic";

pub struct TestRelay {
    pub control: Control,
    pub nodes: Vec<RelayNode>,
    pub control_url: String,
    /// Keeps the SQLite file until the test ends.
    _dir: tempfile::TempDir,
}

impl TestRelay {
    /// A control plane and one reporting node per id; the first is where
    /// accounts start.
    pub async fn spawn(node_ids: &[&str]) -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let database_url = database_url(&dir).await;
        // The node needs the control URL and the control plane the node URL,
        // so the control plane's port is chosen first.
        let control_addr = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .expect("a free port");
        let control_url = format!("http://{control_addr}");
        let ticket_secret = [3; 32];
        let mut nodes = Vec::new();
        for id in node_ids {
            let mut config = NodeConfig::local(
                *id,
                "127.0.0.1:0".parse().expect("constant address"),
                &[yttt_relay_ticket::public_key(&ticket_secret)],
            );
            config.control = Some(NodeControl::from_parts(
                control_url.clone(),
                format!("{id}-token"),
                1,
            ));
            nodes.push(RelayNode::spawn(config).await.expect("a relay node"));
        }
        let control = Control::spawn(ControlConfig {
            listen: control_addr,
            public_url: None,
            web_root: None,
            relay_binaries_dir: None,
            trusted_proxy: false,
            signup: None,
            database_url,
            ticket_secret: yttt_relay_ticket::hex(&ticket_secret),
            secrets_key: None,
            ticket_ttl_secs: 30 * 24 * 3600,
            session_ttl_secs: 3600,
            dev_mode: true,
            oauth: Default::default(),
            plans: vec![PlanConfig {
                id: PLAN.into(),
                rate_bps: 5_000_000,
                burst_bytes: 1_000_000,
                monthly_bytes: 1 << 30,
                throttled_rate_bps: 10_000,
                max_devices: 5,
            }],
            nodes: node_ids
                .iter()
                .zip(&nodes)
                .map(|(id, node)| NodeEntry {
                    id: (*id).into(),
                    url: node_url(node),
                    token: format!("{id}-token"),
                })
                .collect(),
        })
        .await
        .expect("a control plane");
        Self {
            control,
            nodes,
            control_url,
            _dir: dir,
        }
    }

    /// The relay URL of the `index`th node.
    pub fn node_url(&self, index: usize) -> String {
        node_url(&self.nodes[index])
    }

    /// Creates the test account on `node` through the dev-mode endpoint.
    pub async fn create_account(&self, node: &str) -> AccountCreated {
        reqwest::Client::new()
            .post(format!("{}/v1/dev/accounts", self.control_url))
            .json(&CreateAccount {
                email: EMAIL.into(),
                password: PASSWORD.into(),
                plan: PLAN.into(),
                node: node.into(),
            })
            .send()
            .await
            .expect("the control plane answers")
            .error_for_status()
            .expect("the account is created")
            .json()
            .await
            .expect("an account")
    }

    /// Signs in on the console as the test account; its session cookie.
    pub async fn console_cookie(&self) -> String {
        let response = reqwest::Client::new()
            .post(format!("{}/v1/web/session", self.control_url))
            .header("x-yttt-web", "1")
            .json(&CreatePassword {
                email: EMAIL,
                password: PASSWORD,
            })
            .send()
            .await
            .expect("the control plane answers");
        assert!(response.status().is_success(), "console sign-in failed");
        response.headers()["set-cookie"]
            .to_str()
            .expect("an ASCII cookie")
            .split(';')
            .next()
            .expect("a cookie")
            .to_string()
    }
}

#[derive(serde::Serialize)]
struct CreatePassword<'a> {
    email: &'a str,
    password: &'a str,
}

fn node_url(node: &RelayNode) -> String {
    format!("http://{}", node.addr())
}

async fn database_url(dir: &tempfile::TempDir) -> String {
    let Ok(server) = std::env::var("YTTT_TEST_DATABASE_URL") else {
        return format!("sqlite://{}", dir.path().join("control.db").display());
    };
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let name = format!(
        "yttt_test_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    sqlx::any::install_default_drivers();
    let admin = sqlx::AnyPool::connect(&server)
        .await
        .expect("YTTT_TEST_DATABASE_URL names a reachable PostgreSQL server");
    let _ = sqlx::raw_sql(&format!("DROP DATABASE IF EXISTS {name}"))
        .execute(&admin)
        .await;
    sqlx::raw_sql(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .expect("a test database");
    let (prefix, _) = server.rsplit_once('/').expect("a database URL");
    format!("{prefix}/{name}")
}
