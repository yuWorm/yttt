//! Finds the TCP ports each project's local terminals listen on, and keeps
//! how long each one listened so a project's usual ports show while closed.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::sync::watch;
use yttt_core::model::ids::{ProjectId, TerminalSessionId};
use yttt_protocol::ports::{FrequentPort, ListeningPort, ProjectPorts};

use crate::runtime::HostRuntime;

const SCAN_INTERVAL: Duration = Duration::from_secs(3);
/// A scan longer apart than this (the machine slept) counts as one interval.
const MAX_SAMPLE: Duration = Duration::from_secs(6);
const SAVE_INTERVAL: Duration = Duration::from_secs(60);
const FREQUENT_LIMIT: usize = 8;
const HISTORY_LIMIT: usize = 32;
/// A port's listening time counts half after this many days without it.
const HALF_LIFE_DAYS: f64 = 14.0;
/// Ports listening for less than this are not worth suggesting.
const MIN_FREQUENT_SECONDS: u64 = 30;
const HISTORY_FILE: &str = "project-ports.json";
const HISTORY_VERSION: u32 = 1;

pub(crate) struct HostProjectPorts {
    path: PathBuf,
    state: Mutex<PortsState>,
}

#[derive(Default)]
struct PortsState {
    current: HashMap<ProjectId, Vec<ListeningPort>>,
    history: PortHistory,
    dirty: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct PortHistory {
    version: u32,
    projects: BTreeMap<String, Vec<PortHistoryEntry>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PortHistoryEntry {
    port: u16,
    process_name: String,
    listening_seconds: u64,
    last_seen_millis: u64,
}

impl HostProjectPorts {
    pub(crate) fn load(state_root: &Path) -> Arc<Self> {
        let path = state_root.join(HISTORY_FILE);
        let history = match read_history(&path) {
            Ok(history) => history,
            Err(error) if error.kind() == io::ErrorKind::NotFound => PortHistory::default(),
            Err(error) => {
                tracing::warn!(%error, "cannot read the project port history; starting empty");
                PortHistory::default()
            }
        };
        Arc::new(Self {
            path,
            state: Mutex::new(PortsState {
                history,
                ..PortsState::default()
            }),
        })
    }

    pub(crate) fn snapshot(&self, project_id: &ProjectId) -> ProjectPorts {
        let state = self.state.lock();
        let listening = state.current.get(project_id).cloned().unwrap_or_default();
        let frequent = state
            .history
            .projects
            .get(project_id.as_str())
            .map(|entries| frequent_ports(entries, &listening, now_millis()))
            .unwrap_or_default();
        ProjectPorts {
            listening,
            frequent,
        }
    }

    fn apply_scan(
        &self,
        current: HashMap<ProjectId, Vec<ListeningPort>>,
        sample_seconds: u64,
        now: u64,
    ) {
        let mut state = self.state.lock();
        if sample_seconds > 0 {
            let changed = record_sample(&mut state.history, &current, sample_seconds, now);
            state.dirty |= changed;
        }
        state.current = current;
    }

    fn save_if_dirty(&self) {
        let history = {
            let mut state = self.state.lock();
            if !state.dirty {
                return;
            }
            state.dirty = false;
            state.history.clone()
        };
        if let Err(error) = write_history(&self.path, &history) {
            tracing::warn!(%error, "cannot save the project port history");
        }
    }
}

pub(crate) async fn run(
    runtime: Arc<HostRuntime>,
    ports: Arc<HostProjectPorts>,
    mut stop: watch::Receiver<bool>,
) {
    if !sysinfo::IS_SUPPORTED_SYSTEM {
        return;
    }
    let mut scanner = PortScanner::default();
    let mut interval = tokio::time::interval(SCAN_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_scan: Option<Instant> = None;
    let mut last_save = Instant::now();
    loop {
        tokio::select! {
            _ = interval.tick() => {}
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break;
                }
                continue;
            }
        }
        let roots = runtime.local_project_process_roots();
        let scan = tokio::task::spawn_blocking(move || {
            let current = scanner.scan(&roots);
            (scanner, current)
        })
        .await;
        let Ok((next_scanner, current)) = scan else {
            break;
        };
        scanner = next_scanner;
        let now = Instant::now();
        let sample = last_scan
            .map(|last| now.duration_since(last).min(MAX_SAMPLE).as_secs())
            .unwrap_or(0);
        last_scan = Some(now);
        ports.apply_scan(current, sample, now_millis());
        if last_save.elapsed() >= SAVE_INTERVAL {
            last_save = Instant::now();
            let ports = ports.clone();
            let _ = tokio::task::spawn_blocking(move || ports.save_if_dirty()).await;
        }
    }
    let _ = tokio::task::spawn_blocking(move || ports.save_if_dirty()).await;
}

#[derive(Default)]
struct PortScanner {
    system: System,
}

impl PortScanner {
    fn scan(
        &mut self,
        roots: &[(TerminalSessionId, ProjectId, u32)],
    ) -> HashMap<ProjectId, Vec<ListeningPort>> {
        if roots.is_empty() {
            return HashMap::new();
        }
        let sockets = match listeners::get_all() {
            Ok(sockets) => sockets,
            Err(error) => {
                tracing::debug!(%error, "cannot list the listening sockets");
                return HashMap::new();
            }
        };
        let sockets = sockets
            .into_iter()
            .filter(|socket| {
                socket.protocol == listeners::Protocol::TCP
                    && socket.state == listeners::SocketState::Listen
            })
            .map(|socket| ListeningSocket {
                pid: socket.process.pid,
                process_name: socket.process.name,
                port: socket.socket.port(),
                address: socket.socket.ip().to_string(),
            })
            .collect::<Vec<_>>();
        if sockets.is_empty() {
            return HashMap::new();
        }
        self.system
            .refresh_processes_specifics(ProcessesToUpdate::All, ProcessRefreshKind::new());
        let parents = self
            .system
            .processes()
            .iter()
            .map(|(pid, process)| (pid.as_u32(), process.parent().map(|pid| pid.as_u32())))
            .collect::<HashMap<_, _>>();
        attribute_sockets(roots, &parents, &sockets)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ListeningSocket {
    pid: u32,
    process_name: String,
    port: u16,
    address: String,
}

/// Groups the sockets whose process descends from a terminal's shell by the
/// terminal's project; one entry per port, holding every bound address.
fn attribute_sockets(
    roots: &[(TerminalSessionId, ProjectId, u32)],
    parents: &HashMap<u32, Option<u32>>,
    sockets: &[ListeningSocket],
) -> HashMap<ProjectId, Vec<ListeningPort>> {
    let root_by_pid = roots
        .iter()
        .map(|(session_id, project_id, pid)| (*pid, (session_id, project_id)))
        .collect::<HashMap<_, _>>();
    let mut ports: HashMap<ProjectId, BTreeMap<u16, (ListeningPort, BTreeSet<String>)>> =
        HashMap::new();
    for socket in sockets {
        let Some((session_id, project_id)) = owning_root(socket.pid, parents, &root_by_pid) else {
            continue;
        };
        let (_, addresses) = ports
            .entry(project_id.clone())
            .or_default()
            .entry(socket.port)
            .or_insert_with(|| {
                (
                    ListeningPort {
                        port: socket.port,
                        addresses: Vec::new(),
                        pid: socket.pid,
                        process_name: socket.process_name.clone(),
                        session_id: session_id.clone(),
                    },
                    BTreeSet::new(),
                )
            });
        addresses.insert(socket.address.clone());
    }
    ports
        .into_iter()
        .map(|(project_id, by_port)| {
            let ports = by_port
                .into_values()
                .map(|(mut port, addresses)| {
                    port.addresses = addresses.into_iter().collect();
                    port
                })
                .collect();
            (project_id, ports)
        })
        .collect()
}

fn owning_root<'a>(
    pid: u32,
    parents: &HashMap<u32, Option<u32>>,
    root_by_pid: &HashMap<u32, (&'a TerminalSessionId, &'a ProjectId)>,
) -> Option<(&'a TerminalSessionId, &'a ProjectId)> {
    let mut current = pid;
    // A bound on the walk guards against a parent cycle in a racing snapshot.
    for _ in 0..256 {
        if let Some(root) = root_by_pid.get(&current) {
            return Some(*root);
        }
        current = (*parents.get(&current)?)?;
    }
    None
}

/// Adds `seconds` of listening to every port seen in this scan; whether the
/// history changed in a way worth saving.
fn record_sample(
    history: &mut PortHistory,
    current: &HashMap<ProjectId, Vec<ListeningPort>>,
    seconds: u64,
    now: u64,
) -> bool {
    let mut changed = false;
    for (project_id, ports) in current {
        let entries = history
            .projects
            .entry(project_id.as_str().to_string())
            .or_default();
        for port in ports {
            match entries
                .iter_mut()
                .find(|entry| entry.port == port.port && entry.process_name == port.process_name)
            {
                Some(entry) => {
                    entry.listening_seconds = entry.listening_seconds.saturating_add(seconds);
                    entry.last_seen_millis = now;
                }
                None => entries.push(PortHistoryEntry {
                    port: port.port,
                    process_name: port.process_name.clone(),
                    listening_seconds: seconds,
                    last_seen_millis: now,
                }),
            }
            changed = true;
        }
        if entries.len() > HISTORY_LIMIT {
            entries.sort_by(|left, right| score(right, now).total_cmp(&score(left, now)));
            entries.truncate(HISTORY_LIMIT);
        }
    }
    changed
}

fn score(entry: &PortHistoryEntry, now: u64) -> f64 {
    let age_days = now.saturating_sub(entry.last_seen_millis) as f64 / 86_400_000.0;
    entry.listening_seconds as f64 * 0.5_f64.powf(age_days / HALF_LIFE_DAYS)
}

/// The project's usual ports that are not listening now, most frequent
/// first, one per port.
fn frequent_ports(
    entries: &[PortHistoryEntry],
    listening: &[ListeningPort],
    now: u64,
) -> Vec<FrequentPort> {
    let mut candidates = entries
        .iter()
        .filter(|entry| entry.listening_seconds >= MIN_FREQUENT_SECONDS)
        .filter(|entry| !listening.iter().any(|port| port.port == entry.port))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| score(right, now).total_cmp(&score(left, now)));
    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|entry| seen.insert(entry.port))
        .take(FREQUENT_LIMIT)
        .map(|entry| FrequentPort {
            port: entry.port,
            process_name: entry.process_name.clone(),
            listening_seconds: entry.listening_seconds,
            last_seen_millis: entry.last_seen_millis,
        })
        .collect()
}

fn read_history(path: &Path) -> io::Result<PortHistory> {
    let bytes = crate::workspace::read_bounded_file(path, 1024 * 1024)?;
    let history: PortHistory = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if history.version != HISTORY_VERSION {
        return Ok(PortHistory::default());
    }
    Ok(history)
}

fn write_history(path: &Path, history: &PortHistory) -> io::Result<()> {
    let history = PortHistory {
        version: HISTORY_VERSION,
        projects: history.projects.clone(),
    };
    let bytes = serde_json::to_vec(&history).map_err(io::Error::other)?;
    crate::workspace::atomic_write(path, &bytes, uuid::Uuid::new_v4().as_u128() as u64)
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(session: &str, project: &str, pid: u32) -> (TerminalSessionId, ProjectId, u32) {
        (
            TerminalSessionId::new(session),
            ProjectId::new(project),
            pid,
        )
    }

    fn socket(pid: u32, name: &str, port: u16, address: &str) -> ListeningSocket {
        ListeningSocket {
            pid,
            process_name: name.to_string(),
            port,
            address: address.to_string(),
        }
    }

    #[test]
    fn a_port_belongs_to_the_project_whose_shell_started_its_process() {
        let roots = [root("a:t:p", "a", 100), root("b:t:p", "b", 200)];
        // 100 -> 110 (npm) -> 111 (node); 200 -> 210 (python); 300 unrelated.
        let parents = HashMap::from([
            (100, Some(1)),
            (110, Some(100)),
            (111, Some(110)),
            (200, Some(1)),
            (210, Some(200)),
            (300, Some(1)),
        ]);
        let sockets = [
            socket(111, "node", 5173, "::1"),
            socket(111, "node", 5173, "127.0.0.1"),
            socket(210, "python3", 8000, "0.0.0.0"),
            socket(300, "postgres", 5432, "127.0.0.1"),
        ];
        let ports = attribute_sockets(&roots, &parents, &sockets);
        let a = &ports[&ProjectId::new("a")];
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].port, 5173);
        assert_eq!(a[0].addresses, vec!["127.0.0.1", "::1"]);
        assert_eq!(a[0].session_id, TerminalSessionId::new("a:t:p"));
        assert_eq!(ports[&ProjectId::new("b")][0].port, 8000);
        assert_eq!(ports.len(), 2);
    }

    #[test]
    fn a_parent_cycle_does_not_hang_the_walk() {
        let roots = [root("a:t:p", "a", 100)];
        let parents = HashMap::from([(5, Some(6)), (6, Some(5))]);
        let ports = attribute_sockets(&roots, &parents, &[socket(5, "x", 1, "::")]);
        assert!(ports.is_empty());
    }

    fn listening(project: &str, port: u16, name: &str) -> HashMap<ProjectId, Vec<ListeningPort>> {
        HashMap::from([(
            ProjectId::new(project),
            vec![ListeningPort {
                port,
                addresses: vec!["127.0.0.1".to_string()],
                pid: 1,
                process_name: name.to_string(),
                session_id: TerminalSessionId::new("s"),
            }],
        )])
    }

    #[test]
    fn frequent_ports_leave_out_listening_ones_and_brief_ones() {
        let mut history = PortHistory::default();
        let day = 86_400_000;
        for _ in 0..40 {
            record_sample(&mut history, &listening("a", 3000, "node"), 3, day);
        }
        record_sample(&mut history, &listening("a", 9229, "node"), 3, day);
        for _ in 0..20 {
            record_sample(&mut history, &listening("a", 8080, "java"), 3, day);
        }
        let entries = &history.projects["a"];

        let frequent = frequent_ports(entries, &[], day);
        assert_eq!(
            frequent.iter().map(|port| port.port).collect::<Vec<_>>(),
            vec![3000, 8080]
        );
        assert_eq!(frequent[0].listening_seconds, 120);

        let now_listening = &listening("a", 3000, "node")[&ProjectId::new("a")];
        let frequent = frequent_ports(entries, now_listening, day);
        assert_eq!(
            frequent.iter().map(|port| port.port).collect::<Vec<_>>(),
            vec![8080]
        );
    }

    #[test]
    fn an_old_port_ranks_below_a_recent_one_with_less_time() {
        let day = 86_400_000;
        let mut history = PortHistory::default();
        record_sample(&mut history, &listening("a", 3000, "node"), 1000, day);
        record_sample(&mut history, &listening("a", 4000, "node"), 300, 60 * day);
        let frequent = frequent_ports(&history.projects["a"], &[], 60 * day);
        assert_eq!(frequent[0].port, 4000);
        assert_eq!(frequent[1].port, 3000);
    }

    #[test]
    fn history_survives_a_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let ports = HostProjectPorts::load(dir.path());
        ports.apply_scan(listening("a", 3000, "node"), 60, now_millis());
        ports.save_if_dirty();

        let reloaded = HostProjectPorts::load(dir.path());
        let snapshot = reloaded.snapshot(&ProjectId::new("a"));
        assert!(snapshot.listening.is_empty());
        assert_eq!(snapshot.frequent.len(), 1);
        assert_eq!(snapshot.frequent[0].port, 3000);
        assert_eq!(snapshot.frequent[0].process_name, "node");
    }
}
