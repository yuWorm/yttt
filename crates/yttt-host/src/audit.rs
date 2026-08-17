use std::collections::VecDeque;

use parking_lot::Mutex;

const MAX_AUDIT_ENTRIES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditResult {
    Allowed,
    Denied,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEntry {
    pub request_id: u64,
    pub actor_client_id: String,
    pub actor_device_id: Option<String>,
    pub action: String,
    pub resource: String,
    pub result: AuditResult,
}

pub struct HostAuditLog {
    entries: Mutex<VecDeque<AuditEntry>>,
}

impl HostAuditLog {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(VecDeque::new()),
        }
    }

    pub fn record(&self, entry: AuditEntry) {
        let mut entries = self.entries.lock();
        if entries.len() >= MAX_AUDIT_ENTRIES {
            entries.pop_front();
        }
        entries.push_back(entry);
    }

    #[cfg(test)]
    pub fn entries(&self) -> Vec<AuditEntry> {
        self.entries.lock().iter().cloned().collect()
    }
}

impl Default for HostAuditLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_log_evicts_at_hard_limit_and_never_stores_payloads() {
        let log = HostAuditLog::new();
        for request_id in 0..=MAX_AUDIT_ENTRIES as u64 {
            log.record(AuditEntry {
                request_id,
                actor_client_id: "client".to_string(),
                actor_device_id: Some("device".to_string()),
                action: "git.mutate".to_string(),
                resource: "project".to_string(),
                result: AuditResult::Denied,
            });
        }
        let entries = log.entries();
        assert_eq!(entries.len(), MAX_AUDIT_ENTRIES);
        assert_eq!(entries[0].request_id, 1);
        assert!(
            entries
                .iter()
                .all(|entry| entry.action == "git.mutate" && entry.resource == "project")
        );
    }
}
