//! TCP ports a project's terminals listen on, as the Host sees them.

use serde::{Deserialize, Serialize};
use yttt_core::model::ids::TerminalSessionId;

/// A TCP port in the listening state, opened by a process started from one
/// of the project's terminals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListeningPort {
    pub port: u16,
    /// The local addresses the port is bound to, such as `127.0.0.1` or `::`.
    pub addresses: Vec<String>,
    pub pid: u32,
    pub process_name: String,
    /// The terminal whose process tree holds the listening process.
    pub session_id: TerminalSessionId,
}

/// A port the project's terminals often listen on, though not right now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrequentPort {
    pub port: u16,
    pub process_name: String,
    /// How long the Host has seen the port listening, summed over its scans.
    pub listening_seconds: u64,
    pub last_seen_millis: u64,
}

/// The answer to `Request::ProjectPorts`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPorts {
    /// Ordered by port.
    pub listening: Vec<ListeningPort>,
    /// Most frequent first; none of them is listening now.
    pub frequent: Vec<FrequentPort>,
}
