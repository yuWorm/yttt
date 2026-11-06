//! JSON bodies exchanged between relay nodes and the control plane.

use serde::{Deserialize, Serialize};

/// What a node reports every interval: bytes each account sent since the last
/// accepted report. Accounts with live connections or a non-active standing
/// appear even with zero bytes, so the node keeps learning their standing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageReport {
    pub accounts: Vec<AccountUsage>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountUsage {
    pub account_id: u64,
    pub upstream_bytes: u64,
}

/// The control plane's answer: the standing of every account in the report.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageReply {
    pub accounts: Vec<AccountStanding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountStanding {
    pub account_id: u64,
    pub standing: Standing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Standing {
    /// Relay at the rate the account's tickets carry.
    Active,
    /// Over quota: relay, but no faster than this.
    Throttled { rate_bps: u32, burst_bytes: u32 },
    /// Drop every connection and admit none.
    Suspended,
}
