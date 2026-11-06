//! Sends usage to the control plane and applies the standings it returns.
//!
//! When the control plane is unreachable the node keeps relaying on the
//! tickets it already trusts and the last standings it heard, and keeps the
//! unsent bytes for the next report.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use tracing::warn;
use yttt_relay_ticket::api::{UsageReply, UsageReport};

use crate::{ControlConfig, node::Node};

pub(crate) async fn run(
    node: Arc<Node>,
    node_id: String,
    control: ControlConfig,
    cancel: CancellationToken,
) {
    let client = reqwest::Client::builder()
        .timeout(control.report_interval())
        .build()
        .expect("a TLS-capable HTTP client");
    let url = format!(
        "{}/v1/nodes/{}/usage",
        control.url.trim_end_matches('/'),
        node_id
    );
    let mut tick = tokio::time::interval(control.report_interval());
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            _ = tick.tick() => {}
        }
        let report = node.take_usage();
        match send(&client, &url, &control.node_token, &report).await {
            Ok(reply) => node.apply(&reply.accounts),
            Err(error) => {
                warn!(%error, "usage report failed; keeping the bytes for the next one");
                node.restore(&report);
            }
        }
    }
}

async fn send(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    report: &UsageReport,
) -> Result<UsageReply, reqwest::Error> {
    client
        .post(url)
        .bearer_auth(token)
        .json(report)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
}
