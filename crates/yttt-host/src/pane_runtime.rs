//! Runs the processes of layout panes.
//!
//! The Host decides when a pane runs: after every accepted intent, when a
//! Client opens a workspace, when an SSH machine connects and — for an
//! independent Host — at startup, it claims the panes that must run and
//! starts them. It records their exits, applies each pane's exit behavior and
//! keeps the Agent snapshot of Agent panes. Clients only attach.

use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use tokio::sync::{broadcast, watch};
use yttt_agent_core::{AgentSnapshot, AgentViewState};
use yttt_agent_providers::{
    OMP_EXTENSION_SOURCE, OMP_PROVIDER_ID,
    sessions::{AgentSessionRoots, omp_session_exists},
};
use yttt_core::model::{
    host_workspace::{PaneAddress, PaneLaunchTarget, WorkspaceIntent},
    ids::TerminalSessionId,
    layout::ProcessExitBehavior,
    project::ProjectLocation,
};
use yttt_protocol::{
    HostLifecycleState, ServerEvent, agent::AgentSnapshotUpdate, ssh::SshConnectionState,
    terminal::TerminalProcessState, workspace::WorkspaceId,
};

use crate::{
    agent_hooks::HostAgentHookRuntime,
    lifecycle::HostLifecycle,
    pane_launch::{LaunchSettings, PaneLaunch, PaneLaunchRequest, PaneLauncher},
    project::HostProjectRuntime,
    runtime::{HostRuntime, HostRuntimeError},
    ssh_runtime::{HostSshRuntime, SshRuntimeEvent},
    terminal::HostTerminalEvent,
    workspace::WorkspaceService,
};

/// Matches the restart delay the desktop used before the Host owned panes.
const AUTO_RESTART_DELAY: Duration = Duration::from_millis(500);
const AGENT_STATE_FLUSH_INTERVAL: Duration = Duration::from_secs(1);
/// An exited pane stays known for this long, so the Agent snapshot its exit
/// produces still reaches the pane.
const EXITED_PANE_RETENTION: Duration = Duration::from_secs(10);

struct LaunchedPane {
    workspace_id: WorkspaceId,
    address: PaneAddress,
    /// Set once the terminal is created, before its process starts.
    session_epoch: Option<u64>,
    exit_behavior: ProcessExitBehavior,
    /// The Agent resumes a saved session it has not confirmed yet.
    resuming: bool,
    exited_at: Option<Instant>,
}

pub(crate) struct PaneRuntime {
    workspaces: Arc<WorkspaceService>,
    runtime: Arc<HostRuntime>,
    agent_hooks: Arc<HostAgentHookRuntime>,
    ssh: Arc<HostSshRuntime>,
    projects: Arc<HostProjectRuntime>,
    lifecycle: Arc<HostLifecycle>,
    launcher: PaneLauncher,
    config_root: PathBuf,
    shell_candidates: Vec<String>,
    launched: Mutex<HashMap<TerminalSessionId, LaunchedPane>>,
    agents_provisioned: Mutex<bool>,
}

impl PaneRuntime {
    pub fn new(
        workspaces: Arc<WorkspaceService>,
        runtime: Arc<HostRuntime>,
        agent_hooks: Arc<HostAgentHookRuntime>,
        ssh: Arc<HostSshRuntime>,
        projects: Arc<HostProjectRuntime>,
        lifecycle: Arc<HostLifecycle>,
        config_root: &Path,
    ) -> Arc<Self> {
        Arc::new(Self {
            workspaces,
            runtime,
            agent_hooks,
            ssh,
            projects,
            lifecycle,
            launcher: PaneLauncher::new(config_root),
            config_root: config_root.to_path_buf(),
            shell_candidates: crate::pane_launch::detect_shells().1,
            launched: Mutex::new(HashMap::new()),
            agents_provisioned: Mutex::new(false),
        })
    }

    /// Follows terminal exits, Agent reports and SSH connections until `stop`.
    pub fn start(self: &Arc<Self>, stop: watch::Receiver<bool>) {
        let mut terminal_events = self.runtime.subscribe();
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                match terminal_events.recv().await {
                    Ok(HostTerminalEvent::Exited {
                        session_id,
                        session_epoch,
                        ..
                    }) => {
                        this.blocking(move |this| this.terminal_exited(&session_id, session_epoch))
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        for placement in this.runtime.placements() {
                            if let TerminalProcessState::Exited { .. } = placement.process_state {
                                this.blocking(move |this| {
                                    this.terminal_exited(
                                        &placement.session_id,
                                        placement.session_epoch,
                                    );
                                });
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let mut agent_updates = self.agent_hooks.subscribe();
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                match agent_updates.recv().await {
                    Ok(update) => this.agent_updated(update),
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        for update in this.agent_hooks.snapshots_after(&[]) {
                            this.agent_updated(update);
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let mut ssh_events = self.ssh.subscribe();
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                match ssh_events.recv().await {
                    Ok(SshRuntimeEvent::Broadcast(ServerEvent::SshStateChanged(status)))
                        if status.state == SshConnectionState::Connected =>
                    {
                        this.blocking(|this| this.ensure_all());
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        this.blocking(|this| this.ensure_all());
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let this = self.clone();
        let mut stop = stop;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(AGENT_STATE_FLUSH_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    changed = stop.changed() => {
                        if changed.is_err() || *stop.borrow() {
                            let this = this.clone();
                            let _ = tokio::task::spawn_blocking(move || {
                                this.workspaces.flush_agent_states();
                            })
                            .await;
                            break;
                        }
                        continue;
                    }
                }
                let this = this.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    this.workspaces.flush_agent_states();
                    this.prune_exited();
                })
                .await;
            }
        });
    }

    /// Runs Host state changes, which write to disk, off the async workers.
    fn blocking(self: &Arc<Self>, work: impl FnOnce(&Arc<Self>) + Send + 'static) {
        let this = self.clone();
        tokio::task::spawn_blocking(move || work(&this));
    }

    /// Starts the panes of every open workspace.
    pub fn ensure_all(self: &Arc<Self>) {
        for workspace_id in self.workspaces.open_workspaces() {
            self.ensure(&workspace_id);
        }
    }

    /// Starts the panes a workspace must run now.
    pub fn ensure(self: &Arc<Self>, workspace_id: &WorkspaceId) {
        let targets = match self
            .workspaces
            .claim_launches(workspace_id, |location| self.reachable(location))
        {
            Ok(targets) => targets,
            Err(error) => {
                eprintln!(
                    "panes of workspace {} were not claimed: {}",
                    workspace_id.as_str(),
                    error.message
                );
                return;
            }
        };
        for target in targets {
            self.launch(workspace_id, target);
        }
    }

    fn reachable(&self, location: &ProjectLocation) -> bool {
        match location {
            ProjectLocation::Local { .. } => true,
            ProjectLocation::Ssh { connection_id, .. } => {
                self.ssh.is_connected(connection_id.as_str())
            }
        }
    }

    fn launch(&self, workspace_id: &WorkspaceId, mut target: PaneLaunchTarget) {
        if target.restore
            && let Some(roots) = AgentSessionRoots::native()
            && drop_missing_omp_session(&mut target, &roots.omp)
        {
            let snapshot = target.agent_snapshot.clone();
            self.workspaces
                .update_agent_snapshot(workspace_id, &target.address, |_| snapshot);
        }
        let settings = LaunchSettings::load(&self.config_root, &self.shell_candidates);
        let planned = match self.launcher.plan(
            &settings,
            &PaneLaunchRequest {
                address: &target.address,
                location: &target.location,
                pane: &target.pane,
                agent_snapshot: target.agent_snapshot.as_ref(),
                restore: target.restore,
            },
        ) {
            PaneLaunch::Start(planned) => *planned,
            PaneLaunch::NotRestorable | PaneLaunch::Unresumable => {
                self.record_exited(workspace_id, &target.address);
                return;
            }
        };
        if planned.clears_agent_snapshot {
            self.workspaces
                .update_agent_snapshot(workspace_id, &target.address, |_| None);
        }
        if planned.agent.is_some()
            && let Err(error) = self.provision_agents()
        {
            eprintln!(
                "Agent pane {} did not start: Agent integration was not installed: {error}",
                target.address.terminal_session_id().as_str()
            );
            self.record_exited(workspace_id, &target.address);
            return;
        }
        let session_id = planned.spec.session_id.clone();
        let resuming = planned
            .agent
            .as_ref()
            .is_some_and(|agent| !agent.resume_arguments.is_empty());
        let mut pane = LaunchedPane {
            workspace_id: workspace_id.clone(),
            address: target.address.clone(),
            session_epoch: None,
            exit_behavior: planned.exit_behavior,
            resuming,
            exited_at: None,
        };
        if let Some(terminal) = self.runtime.terminal(&session_id)
            && !terminal.is_exited()
        {
            // A window reopened a workspace whose terminal kept running.
            pane.session_epoch = Some(terminal.session_epoch());
            pane.resuming = false;
            self.launched.lock().insert(session_id, pane);
            return;
        }
        self.runtime.discard_exited(&session_id);
        self.launched.lock().insert(session_id.clone(), pane);
        let local_cwd = match &target.location {
            ProjectLocation::Local { .. } => {
                match self.projects.local_root(&target.address.project_id) {
                    Ok(root) => Some(root),
                    Err(error) => {
                        self.launch_failed(workspace_id, &target.address, &error.to_string());
                        return;
                    }
                }
            }
            ProjectLocation::Ssh { .. } => None,
        };
        let mut scope = None;
        let spawned = self.runtime.spawn_with_transport(
            planned.spec,
            Some(self.ssh.transport()),
            local_cwd,
            |spec, epoch| {
                scope = Some(self.agent_hooks.secure_terminal_environment(spec, epoch));
                if let Some(pane) = self.launched.lock().get_mut(&session_id) {
                    pane.session_epoch = Some(epoch);
                }
            },
        );
        // Clients attach to the panes the Host starts once their catalog lists them.
        self.lifecycle.resource_changed();
        if let Err(error) = spawned {
            if let Some(scope) = scope {
                self.agent_hooks.cancel_terminal(&scope);
            }
            if !matches!(error, HostRuntimeError::AlreadyExists(_)) {
                self.launch_failed(workspace_id, &target.address, &error.to_string());
            }
        }
    }

    fn launch_failed(&self, workspace_id: &WorkspaceId, address: &PaneAddress, error: &str) {
        let session_id = address.terminal_session_id();
        eprintln!("pane {} did not start: {error}", session_id.as_str());
        self.launched.lock().remove(&session_id);
        self.record_exited(workspace_id, address);
    }

    fn record_exited(&self, workspace_id: &WorkspaceId, address: &PaneAddress) {
        if let Err(error) = self.workspaces.record_pane_exited(workspace_id, address) {
            eprintln!(
                "exit of pane {} was not recorded: {}",
                address.terminal_session_id().as_str(),
                error.message
            );
        }
    }

    fn terminal_exited(self: &Arc<Self>, session_id: &TerminalSessionId, session_epoch: u64) {
        // A stopping Host ends its panes itself; they stay `Running` so the
        // next Host run restores them instead of closing or restarting them.
        if self.lifecycle.state() != HostLifecycleState::Running {
            return;
        }
        let (workspace_id, address, exit_behavior) = {
            let mut launched = self.launched.lock();
            let Some(pane) = launched.get_mut(session_id).filter(|pane| {
                pane.session_epoch == Some(session_epoch) && pane.exited_at.is_none()
            }) else {
                return;
            };
            pane.exited_at = Some(Instant::now());
            (
                pane.workspace_id.clone(),
                pane.address.clone(),
                pane.exit_behavior,
            )
        };
        match self.workspaces.record_pane_exited(&workspace_id, &address) {
            Ok(Some(_)) => {}
            Ok(None) => {
                if !self
                    .workspaces
                    .pane_addresses(&workspace_id)
                    .contains(&address)
                {
                    self.discard(session_id);
                }
                return;
            }
            Err(error) => {
                eprintln!(
                    "exit of pane {} was not recorded: {}",
                    session_id.as_str(),
                    error.message
                );
                return;
            }
        }
        match exit_behavior {
            ProcessExitBehavior::Close => {
                let intent = WorkspaceIntent::ClosePaneForExit {
                    project_id: address.project_id,
                    tab_id: address.tab_id,
                    pane_id: address.pane_id,
                };
                match self.workspaces.apply(workspace_id, intent, None) {
                    Ok(_) => self.discard(session_id),
                    Err(error) => eprintln!(
                        "exited pane {} was not closed: {}",
                        session_id.as_str(),
                        error.message
                    ),
                }
            }
            ProcessExitBehavior::AutoRestart => {
                let this = self.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(AUTO_RESTART_DELAY).await;
                    this.blocking(move |this| this.restart(&workspace_id, address));
                });
            }
            ProcessExitBehavior::ManualRestart => {}
        }
    }

    /// Drops the exited terminal of a pane that left the layout: no Client
    /// shows the pane, so none will acknowledge the exit.
    fn discard(&self, session_id: &TerminalSessionId) {
        self.launched.lock().remove(session_id);
        self.runtime.discard_exited(session_id);
    }

    fn restart(self: &Arc<Self>, workspace_id: &WorkspaceId, address: PaneAddress) {
        let intent = WorkspaceIntent::RestartPane {
            project_id: address.project_id,
            tab_id: address.tab_id,
            pane_id: address.pane_id,
            resume: true,
        };
        // The pane may have been closed or restarted by a Client meanwhile.
        if self
            .workspaces
            .apply(workspace_id.clone(), intent, None)
            .is_ok()
        {
            self.ensure(workspace_id);
        }
    }

    /// Records an Agent report on its pane, keeping the board history and,
    /// until the resumed Agent confirms it, the saved session.
    fn agent_updated(&self, update: AgentSnapshotUpdate) {
        let (workspace_id, address, resuming) = {
            let launched = self.launched.lock();
            let Some(pane) = launched
                .get(&update.terminal_session_id)
                .filter(|pane| pane.session_epoch == Some(update.scope.generation))
            else {
                return;
            };
            (
                pane.workspace_id.clone(),
                pane.address.clone(),
                pane.resuming,
            )
        };
        let mut confirmed = false;
        self.workspaces
            .update_agent_snapshot(&workspace_id, &address, |previous| {
                let (snapshot, confirms) = merge_agent_report(previous, update.snapshot, resuming);
                confirmed = confirms;
                Some(snapshot)
            });
        if resuming
            && confirmed
            && let Some(pane) = self.launched.lock().get_mut(&update.terminal_session_id)
        {
            pane.resuming = false;
        }
    }

    fn prune_exited(&self) {
        self.launched.lock().retain(|_, pane| {
            pane.exited_at
                .is_none_or(|exited_at| exited_at.elapsed() < EXITED_PANE_RETENTION)
        });
    }

    /// Installs the managed Agent hooks and the OMP status extension once per
    /// Host run, before the first Agent starts.
    fn provision_agents(&self) -> io::Result<()> {
        let mut provisioned = self.agents_provisioned.lock();
        if *provisioned {
            return Ok(());
        }
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| io::Error::other("Host home directory is unavailable"))?;
        yttt_agent_providers::installer::install_managed_hooks_at(&self.config_root, &home)?;
        let extension = crate::pane_launch::omp_extension_path(&self.config_root);
        if std::fs::read(&extension).ok().as_deref() != Some(OMP_EXTENSION_SOURCE.as_bytes()) {
            let directory = extension
                .parent()
                .expect("the OMP extension lives in its provider directory");
            std::fs::create_dir_all(directory)?;
            crate::workspace::atomic_write(
                &extension,
                OMP_EXTENSION_SOURCE.as_bytes(),
                uuid::Uuid::new_v4().as_u128() as u64,
            )?;
        }
        *provisioned = true;
        Ok(())
    }
}

/// A saved OMP conversation removed from this machine cannot be resumed, so
/// the pane starts a new OMP session instead. An unreadable session store is
/// not evidence that the conversation is gone and keeps the session.
fn drop_missing_omp_session(target: &mut PaneLaunchTarget, omp_root: &Path) -> bool {
    if !matches!(target.location, ProjectLocation::Local { .. }) {
        return false;
    }
    let Some(snapshot) = target.agent_snapshot.as_mut() else {
        return false;
    };
    if snapshot.provider_id.as_str() != OMP_PROVIDER_ID {
        return false;
    }
    let Some(session_id) = snapshot
        .session
        .as_ref()
        .and_then(|session| session.session_id.as_deref())
    else {
        return false;
    };
    if !matches!(omp_session_exists(omp_root, session_id), Ok(false)) {
        return false;
    }
    snapshot.session = None;
    true
}

/// Merges an Agent report into the pane's previous snapshot, keeping the
/// board history. While a saved session is being resumed, a report without a
/// session keeps the saved one, so a failed resume can be tried again.
/// Returns whether the report confirms the resumed session.
fn merge_agent_report(
    previous: Option<&AgentSnapshot>,
    mut report: AgentSnapshot,
    resuming: bool,
) -> (AgentSnapshot, bool) {
    if let Some(previous) = previous {
        report.board.merge_retained(&previous.board);
    }
    let mut confirmed = false;
    if report.session.is_some() {
        confirmed = report.view_state() != AgentViewState::Failed;
    } else if resuming {
        report.session = previous.and_then(|previous| previous.session.clone());
    }
    (report, confirmed)
}

#[cfg(test)]
mod tests {
    use yttt_agent_core::{AgentInstanceId, AgentReducer, AgentSessionMetadata, ProviderId};
    use yttt_core::model::{
        ids::{ConnectionId, ProjectId},
        layout::{PaneConfig, PaneKind, TerminalExecutionMode},
        project::RemotePathBuf,
    };

    use super::*;

    fn target(location: ProjectLocation, provider: &'static str) -> PaneLaunchTarget {
        let mut snapshot = AgentReducer::new(
            AgentInstanceId::random(),
            ProviderId::from_static(provider),
            1,
        )
        .snapshot()
        .clone();
        snapshot.session = Some(AgentSessionMetadata {
            session_id: Some("saved".into()),
            ..Default::default()
        });
        PaneLaunchTarget {
            address: PaneAddress {
                project_id: ProjectId::new("project"),
                tab_id: "tab".into(),
                pane_id: "pane".into(),
            },
            location,
            pane: PaneConfig {
                id: "pane".into(),
                title: "Pane".into(),
                command: provider.into(),
                args: Vec::new(),
                execution_mode: TerminalExecutionMode::Command,
                exit_behavior: ProcessExitBehavior::ManualRestart,
                kind: PaneKind::Agent,
                notify_on_exit: false,
                detector: None,
            },
            agent_snapshot: Some(snapshot),
            restore: true,
        }
    }

    fn local() -> ProjectLocation {
        ProjectLocation::Local {
            path: PathBuf::from("/work/project"),
        }
    }

    fn session_id(target: &PaneLaunchTarget) -> Option<&str> {
        target
            .agent_snapshot
            .as_ref()?
            .session
            .as_ref()?
            .session_id
            .as_deref()
    }

    #[test]
    fn restore_drops_only_an_omp_session_missing_from_the_host() {
        let root = tempfile::tempdir().unwrap();

        let mut missing = target(local(), OMP_PROVIDER_ID);
        assert!(drop_missing_omp_session(&mut missing, root.path()));
        assert_eq!(session_id(&missing), None);

        let project = root.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("2026-10-29_saved.jsonl"), "").unwrap();
        let mut present = target(local(), OMP_PROVIDER_ID);
        assert!(!drop_missing_omp_session(&mut present, root.path()));
        assert_eq!(session_id(&present), Some("saved"));

        let empty = tempfile::tempdir().unwrap();
        let mut other_provider = target(local(), "codex");
        assert!(!drop_missing_omp_session(&mut other_provider, empty.path()));
        assert_eq!(session_id(&other_provider), Some("saved"));

        // An SSH pane's conversation lives on the remote machine.
        let mut remote = target(
            ProjectLocation::Ssh {
                connection_id: ConnectionId::new("connection"),
                root: RemotePathBuf::new("/srv/project").unwrap(),
            },
            OMP_PROVIDER_ID,
        );
        assert!(!drop_missing_omp_session(&mut remote, empty.path()));
        assert_eq!(session_id(&remote), Some("saved"));
    }

    #[test]
    fn a_resumed_agent_keeps_its_saved_session_until_it_confirms_one() {
        use yttt_agent_core::{AgentEventKind, AgentExitReason, AgentProcessExit};

        let saved = target(local(), "codex").agent_snapshot.unwrap();
        let reducer = || {
            let mut reducer = AgentReducer::new(
                AgentInstanceId::random(),
                ProviderId::from_static("codex"),
                1,
            );
            reducer.process_starting(7, 1);
            reducer.process_started(7, 2);
            reducer
        };
        let report_session = |snapshot: &AgentSnapshot| {
            snapshot
                .session
                .as_ref()
                .and_then(|session| session.session_id.clone())
        };

        let mut failed = reducer();
        failed.process_exited(
            7,
            AgentProcessExit {
                code: Some(1),
                reason: AgentExitReason::Failed,
            },
            3,
        );
        let (merged, confirmed) = merge_agent_report(Some(&saved), failed.snapshot().clone(), true);
        assert!(!confirmed);
        assert_eq!(report_session(&merged).as_deref(), Some("saved"));
        assert_eq!(merged.view_state(), AgentViewState::Failed);

        // A pane started fresh has no saved session to fall back to.
        let (fresh, _) = merge_agent_report(Some(&saved), failed.snapshot().clone(), false);
        assert_eq!(report_session(&fresh), None);

        let mut resumed = reducer();
        resumed.apply(
            7,
            AgentEventKind::SessionStarted {
                metadata: AgentSessionMetadata {
                    session_id: Some("resumed".into()),
                    ..Default::default()
                },
            },
            3,
        );
        let (merged, confirmed) =
            merge_agent_report(Some(&saved), resumed.snapshot().clone(), true);
        assert!(confirmed);
        assert_eq!(report_session(&merged).as_deref(), Some("resumed"));
    }
}
