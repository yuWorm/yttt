//! Builds the terminal launch for a layout pane.
//!
//! Shell, environment and scrollback come from the Host's shared settings
//! (`[terminal]` in `settings.toml`, Host-scoped keys that apply to new
//! sessions).
//!
//! A pane's config says what it is; its saved Agent snapshot only says what
//! ran there. Restoring never turns a pane into something else:
//!
//! | Pane | Saved Agent | Launch |
//! |---|---|---|
//! | Agent | same provider, resumable session | resume it |
//! | Agent | same provider, session it cannot resume | report it unresumable |
//! | Agent | another provider | the pane's command, without that session |
//! | Agent | no session | the pane's command |
//! | Shell | still running when the Host stopped | resume it inside the shell |
//! | Shell | ended on its own, unresumable or no session | a plain shell |
//!
//! A pane in shell mode keeps its shell: an Agent resumed there returns to it.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use yttt_agent_core::{AgentProcessState, AgentProvider as _, AgentSnapshot};
use yttt_agent_providers::{
    OMP_EXTENSION_FILE_NAME, OMP_EXTENSION_SOURCE, OMP_PROVIDER_ID, provider_for_command,
};
use yttt_agent_runtime::{AgentLaunchPlan, AgentRuntime};
use yttt_core::model::{
    host_workspace::PaneAddress,
    layout::{PaneConfig, PaneKind, ProcessExitBehavior, TerminalExecutionMode},
    project::ProjectLocation,
};
use yttt_protocol::{
    ProjectRelativePath,
    terminal::{
        RemoteTerminalExecutionSpec, TerminalExecutionSpec, TerminalGeometry, TerminalSpawnSpec,
    },
};

const AUTO_SHELL: &str = "auto";
const DEFAULT_SCROLLBACK: u32 = 10_000;
/// Size of a terminal nobody has attached to yet; the first attach resizes it.
const DEFAULT_GEOMETRY: TerminalGeometry = TerminalGeometry {
    cols: 120,
    rows: 40,
    cell_width: 0,
    cell_height: 0,
};
/// Inherited values must not reach the child; the Host injects its own.
const AGENT_HOOK_ENVIRONMENT_VARIABLES: [&str; 3] = [
    "YTTT_AGENT_HOOK_ENDPOINT",
    "YTTT_AGENT_HOOK_TOKEN",
    "YTTT_AGENT_HOOK_SCOPE",
];

/// Host-scoped `[terminal]` settings read at each launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LaunchSettings {
    pub shell: String,
    pub environment: BTreeMap<String, String>,
    pub scrollback: u32,
}

impl LaunchSettings {
    /// Reads `<config_root>/settings.toml`; a missing file, unreadable file or
    /// invalid key falls back to that key's default.
    pub fn load(config_root: &Path, shell_candidates: &[String]) -> Self {
        let terminal = fs::read_to_string(config_root.join("settings.toml"))
            .ok()
            .and_then(|text| text.parse::<toml::Table>().ok())
            .and_then(|mut table| match table.remove("terminal") {
                Some(toml::Value::Table(terminal)) => Some(terminal),
                _ => None,
            })
            .unwrap_or_default();
        let configured_shell = terminal
            .get("shell")
            .and_then(toml::Value::as_str)
            .map(str::trim)
            .filter(|shell| !shell.is_empty() && *shell != AUTO_SHELL);
        let shell = configured_shell
            .map(str::to_string)
            .or_else(|| shell_candidates.first().cloned())
            .unwrap_or_else(|| "sh".to_string());
        let environment = terminal
            .get("environment")
            .and_then(toml::Value::as_table)
            .map(|table| {
                table
                    .iter()
                    .filter_map(|(name, value)| {
                        value
                            .as_str()
                            .map(|value| (name.clone(), value.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let scrollback = terminal
            .get("scrollback")
            .and_then(toml::Value::as_integer)
            .filter(|lines| *lines > 0)
            .map_or(DEFAULT_SCROLLBACK, |lines| {
                u32::try_from(lines).unwrap_or(u32::MAX)
            });
        Self {
            shell,
            environment,
            scrollback,
        }
    }
}

/// Shells offered for `terminal.shell = "auto"`: the login shell, then `/etc/shells`.
pub(crate) fn detect_shells() -> (Option<String>, Vec<String>) {
    let shell = std::env::var(if cfg!(windows) { "COMSPEC" } else { "SHELL" }).ok();
    let mut candidates = shell.iter().cloned().collect::<Vec<_>>();
    if let Ok(shells) = fs::read_to_string("/etc/shells") {
        for line in shells
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with('/'))
            .take(64)
        {
            if Path::new(line).is_file() && !candidates.iter().any(|shell| shell == line) {
                candidates.push(line.to_string());
            }
        }
    }
    (shell, candidates)
}

pub(crate) struct PaneLaunchRequest<'a> {
    pub address: &'a PaneAddress,
    pub location: &'a ProjectLocation,
    pub pane: &'a PaneConfig,
    pub agent_snapshot: Option<&'a AgentSnapshot>,
    /// The pane ran before and its terminal is gone (Host restart): do not
    /// replay a shell's startup command or re-run a one-shot command.
    pub restore: bool,
}

#[derive(Debug)]
pub(crate) enum PaneLaunch {
    Start(Box<PlannedLaunch>),
    /// A command pane is not re-run after the Host lost its terminal, and
    /// has no Agent to resume instead.
    NotRestorable,
    /// The saved Agent session cannot be resumed by its provider.
    Unresumable,
}

#[derive(Debug)]
pub(crate) struct PlannedLaunch {
    pub spec: TerminalSpawnSpec,
    pub exit_behavior: ProcessExitBehavior,
    pub agent: Option<AgentLaunchPlan>,
    /// The pane's saved Agent snapshot no longer applies to this launch.
    pub clears_agent_snapshot: bool,
    /// The saved session belongs to another provider than the pane's: its
    /// board stays as history, the session is never resumed.
    pub drops_agent_session: bool,
}

impl PlannedLaunch {
    /// The saved Agent snapshot this launch leaves on the pane. A restored
    /// snapshot describes a process that is gone, so the pane shows no live
    /// Agent until the new process reports.
    pub fn saved_snapshot(
        &self,
        saved: Option<&AgentSnapshot>,
        restore: bool,
    ) -> Option<AgentSnapshot> {
        if self.clears_agent_snapshot {
            return None;
        }
        let mut snapshot = saved?.clone();
        if self.drops_agent_session {
            snapshot.session = None;
        }
        if self.drops_agent_session || restore {
            snapshot.mark_disconnected();
        }
        Some(snapshot)
    }
}

pub(crate) struct PaneLauncher {
    agents: AgentRuntime,
    omp_extension_path: PathBuf,
}

impl PaneLauncher {
    pub fn new(config_root: &Path) -> Self {
        let mut agents = AgentRuntime::default();
        for provider in yttt_agent_providers::builtin_providers() {
            agents.register_provider(provider);
        }
        Self {
            agents,
            omp_extension_path: omp_extension_path(config_root),
        }
    }

    pub fn is_agent_pane(&self, pane: &PaneConfig) -> bool {
        pane.kind == PaneKind::Agent || self.agents.matches_command(&pane.command)
    }

    pub fn plan(&self, settings: &LaunchSettings, request: &PaneLaunchRequest<'_>) -> PaneLaunch {
        if self.is_agent_pane(request.pane) {
            self.plan_agent_pane(settings, request)
        } else {
            self.plan_plain_pane(settings, request)
        }
    }

    fn plan_agent_pane(
        &self,
        settings: &LaunchSettings,
        request: &PaneLaunchRequest<'_>,
    ) -> PaneLaunch {
        let pane = request.pane;
        let (command, args) = (pane.command.as_str(), pane.args.as_slice());
        // Another provider's session came from something run inside this pane.
        let foreign = request.agent_snapshot.is_some_and(|snapshot| {
            provider_for_command(command)
                .is_some_and(|provider| snapshot.provider_id.as_str() != provider)
        });
        let snapshot = request.agent_snapshot.filter(|_| !foreign);

        // OMP that never reported a session starts a new OMP.
        let fresh_omp = snapshot.is_some_and(|snapshot| {
            snapshot.provider_id.as_str() == OMP_PROVIDER_ID && snapshot.session.is_none()
        });
        let fresh_program = (fresh_omp
            && !yttt_agent_providers::OmpProvider.matches_command(command))
        .then_some(OMP_PROVIDER_ID);
        let plan = self.agents.plan_launch(
            fresh_program.unwrap_or(command),
            snapshot.filter(|_| !fresh_omp),
        );
        let Some(plan) = plan else {
            if snapshot.is_some_and(|snapshot| snapshot.session.is_some()) {
                return PaneLaunch::Unresumable;
            }
            return PaneLaunch::Start(Box::new(PlannedLaunch {
                spec: self.spec(settings, request, command, args, None),
                exit_behavior: pane.exit_behavior,
                agent: None,
                clears_agent_snapshot: snapshot.is_some(),
                drops_agent_session: foreign,
            }));
        };
        self.start_agent(
            settings,
            request,
            command,
            args,
            plan,
            fresh_program,
            fresh_omp,
            foreign,
        )
    }

    fn plan_plain_pane(
        &self,
        settings: &LaunchSettings,
        request: &PaneLaunchRequest<'_>,
    ) -> PaneLaunch {
        let pane = request.pane;
        let snapshot = request.agent_snapshot;
        // After a Host restart only an Agent that outlived its pane's Host
        // comes back; an explicit restart may resume one that ended.
        let resume = snapshot
            .filter(|snapshot| {
                !request.restore || snapshot.process_state != AgentProcessState::Exited
            })
            .filter(|snapshot| snapshot.session.is_some())
            .and_then(|snapshot| self.agents.plan_launch("", Some(snapshot)));
        if let Some(plan) = resume {
            return self.start_agent(settings, request, "", &[], plan, None, false, false);
        }
        if request.restore && pane.execution_mode != TerminalExecutionMode::Shell {
            return PaneLaunch::NotRestorable;
        }
        // Restoring a shell must never replay its old startup command.
        let (command, args): (&str, &[String]) = if request.restore {
            ("", &[])
        } else {
            (&pane.command, &pane.args)
        };
        PaneLaunch::Start(Box::new(PlannedLaunch {
            spec: self.spec(settings, request, command, args, None),
            exit_behavior: pane.exit_behavior,
            agent: None,
            // An Agent that ended stays on the pane as history.
            clears_agent_snapshot: snapshot
                .is_some_and(|snapshot| snapshot.process_state != AgentProcessState::Exited),
            drops_agent_session: false,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    fn start_agent(
        &self,
        settings: &LaunchSettings,
        request: &PaneLaunchRequest<'_>,
        command: &str,
        args: &[String],
        plan: AgentLaunchPlan,
        fresh_program: Option<&'static str>,
        clears_agent_snapshot: bool,
        drops_agent_session: bool,
    ) -> PaneLaunch {
        let pane = request.pane;
        let in_shell = pane.execution_mode == TerminalExecutionMode::Shell;
        let resuming = !plan.resume_arguments.is_empty();
        let launch = AgentCommand {
            program_override: fresh_program.or(plan.program_override),
            additional_args: self.agent_arguments(&plan, request.location),
            remote_omp_extension: plan.provider_id.as_str() == OMP_PROVIDER_ID
                && matches!(request.location, ProjectLocation::Ssh { .. }),
            in_shell,
        };
        PaneLaunch::Start(Box::new(PlannedLaunch {
            spec: self.spec(settings, request, command, args, Some(&launch)),
            // A failed resume must not loop; the user restarts explicitly.
            // In a shell it simply returns to the prompt.
            exit_behavior: if resuming && !in_shell {
                ProcessExitBehavior::ManualRestart
            } else {
                pane.exit_behavior
            },
            agent: Some(plan),
            clears_agent_snapshot,
            drops_agent_session,
        }))
    }

    fn agent_arguments(&self, plan: &AgentLaunchPlan, location: &ProjectLocation) -> Vec<String> {
        let mut arguments = plan.resume_arguments.clone();
        if plan.provider_id.as_str() == OMP_PROVIDER_ID
            && matches!(location, ProjectLocation::Local { .. })
        {
            arguments.push("--extension".to_string());
            arguments.push(self.omp_extension_path.to_string_lossy().into_owned());
        }
        arguments
    }

    fn spec(
        &self,
        settings: &LaunchSettings,
        request: &PaneLaunchRequest<'_>,
        command: &str,
        args: &[String],
        agent: Option<&AgentCommand>,
    ) -> TerminalSpawnSpec {
        let shell = settings.shell.clone();
        let command_args = || {
            let mut command_args = if agent.is_some_and(|agent| agent.program_override.is_some()) {
                Vec::new()
            } else {
                args.to_vec()
            };
            if let Some(agent) = agent {
                command_args.extend(agent.additional_args.iter().cloned());
            }
            command_args
        };
        let program_override = agent.and_then(|agent| agent.program_override);
        let execution = match request.location {
            ProjectLocation::Ssh { connection_id, .. } => {
                let in_shell = agent.is_some_and(|agent| agent.in_shell);
                let remote_command = agent
                    .filter(|agent| agent.remote_omp_extension)
                    .map(|agent| agent.remote_omp_command(command, args));
                let execution = match (remote_command, program_override) {
                    // The script ends in `exec` and assumes a POSIX shell.
                    (Some(command), _) if in_shell => RemoteTerminalExecutionSpec::Shell {
                        command: format!("sh -c {}", shell_quote(&command)),
                    },
                    (Some(command), _) => RemoteTerminalExecutionSpec::Shell { command },
                    (None, Some(program)) if in_shell => RemoteTerminalExecutionSpec::Shell {
                        command: shell_words::join(
                            std::iter::once(program.to_string()).chain(command_args()),
                        ),
                    },
                    (None, Some(program)) => RemoteTerminalExecutionSpec::Command {
                        program: program.to_string(),
                        args: command_args(),
                    },
                    (None, None) => match request.pane.execution_mode {
                        TerminalExecutionMode::Shell => RemoteTerminalExecutionSpec::Shell {
                            command: command.to_string(),
                        },
                        TerminalExecutionMode::Command => RemoteTerminalExecutionSpec::Command {
                            program: command.to_string(),
                            args: command_args(),
                        },
                    },
                };
                TerminalExecutionSpec::Ssh {
                    connection_id: connection_id.as_str().to_string(),
                    execution,
                }
            }
            ProjectLocation::Local { .. } => {
                match (program_override, request.pane.execution_mode) {
                    (Some(program), _) => TerminalExecutionSpec::Command {
                        shell,
                        program: program.to_string(),
                        args: command_args(),
                        return_to_shell: agent.is_some_and(|agent| agent.in_shell),
                    },
                    (None, TerminalExecutionMode::Shell) => TerminalExecutionSpec::Shell {
                        args: interactive_shell_args(&shell),
                        program: shell,
                        initial_command: (!command.trim().is_empty()).then(|| command.to_string()),
                    },
                    (None, TerminalExecutionMode::Command) => TerminalExecutionSpec::Command {
                        shell,
                        program: command.to_string(),
                        args: command_args(),
                        return_to_shell: false,
                    },
                }
            }
        };
        let mut environment = settings.environment.clone();
        for name in AGENT_HOOK_ENVIRONMENT_VARIABLES {
            environment.remove(name);
        }
        TerminalSpawnSpec {
            session_id: request.address.terminal_session_id(),
            project_id: request.address.project_id.clone(),
            cwd: ProjectRelativePath::root(),
            execution,
            geometry: DEFAULT_GEOMETRY,
            geometry_epoch: 1,
            query_palette: Vec::new(),
            palette_revision: 0,
            scrollback_limit: settings.scrollback,
            environment: environment.into_iter().collect(),
            removed_environment: AGENT_HOOK_ENVIRONMENT_VARIABLES
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
        }
    }
}

struct AgentCommand {
    program_override: Option<&'static str>,
    additional_args: Vec<String>,
    remote_omp_extension: bool,
    /// The pane keeps its shell: the program runs in it and returns to it.
    in_shell: bool,
}

impl AgentCommand {
    /// Installs the OMP status extension on the remote machine, then runs OMP with it.
    fn remote_omp_command(&self, program: &str, args: &[String]) -> String {
        let encoded = STANDARD.encode(OMP_EXTENSION_SOURCE);
        let program = self.program_override.unwrap_or(program);
        let mut command = format!(
            "umask 077 && yttt_agent_dir=\"$HOME/.config/yttt/agent-providers/{OMP_PROVIDER_ID}\" && \\
             if [ ! -d \"$yttt_agent_dir\" ]; then mkdir -p \"$yttt_agent_dir\"; fi && \\
             yttt_agent_path=\"$yttt_agent_dir/{OMP_EXTENSION_FILE_NAME}\" && \\
             if [ ! -f \"$yttt_agent_path\" ] || ! printf '%s' '{encoded}' | base64 -d | cmp -s - \"$yttt_agent_path\"; then \\
             yttt_agent_tmp=\"$yttt_agent_path.$$\" && printf '%s' '{encoded}' | base64 -d > \"$yttt_agent_tmp\" && \\
             mv -f \"$yttt_agent_tmp\" \"$yttt_agent_path\" || exit $?; fi && exec {}",
            shell_quote(program)
        );
        if self.program_override.is_none() {
            for arg in args {
                command.push(' ');
                command.push_str(&shell_quote(arg));
            }
        }
        for arg in &self.additional_args {
            command.push(' ');
            command.push_str(&shell_quote(arg));
        }
        command.push_str(" --extension \"$yttt_agent_path\"");
        command
    }
}

pub(crate) fn omp_extension_path(config_root: &Path) -> PathBuf {
    config_root
        .join("agent-providers")
        .join(OMP_PROVIDER_ID)
        .join(OMP_EXTENSION_FILE_NAME)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn interactive_shell_args(shell: &str) -> Vec<String> {
    let name = shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell)
        .to_ascii_lowercase();
    if matches!(name.as_str(), "cmd" | "cmd.exe") {
        vec!["/D".to_string()]
    } else if matches!(
        name.as_str(),
        "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe"
    ) {
        vec!["-NoLogo".to_string()]
    } else if matches!(
        name.as_str(),
        "sh" | "sh.exe"
            | "ash"
            | "ash.exe"
            | "bash"
            | "bash.exe"
            | "dash"
            | "dash.exe"
            | "fish"
            | "fish.exe"
            | "ksh"
            | "ksh.exe"
            | "mksh"
            | "mksh.exe"
            | "zsh"
            | "zsh.exe"
    ) {
        vec!["-li".to_string()]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use yttt_agent_core::{
        AgentInstanceId, AgentReducer, AgentSessionMetadata, AgentSnapshot, ProviderId,
    };
    use yttt_core::model::{
        ids::{ConnectionId, ProjectId},
        project::RemotePathBuf,
    };

    use super::*;

    fn settings() -> LaunchSettings {
        LaunchSettings {
            shell: "/bin/zsh".into(),
            environment: BTreeMap::from([
                ("EDITOR".into(), "vim".into()),
                ("YTTT_AGENT_HOOK_TOKEN".into(), "forged".into()),
            ]),
            scrollback: 2_000,
        }
    }

    fn address() -> PaneAddress {
        PaneAddress {
            project_id: ProjectId::new("project"),
            tab_id: "tab".into(),
            pane_id: "pane".into(),
        }
    }

    fn pane(command: &str, args: &[&str], mode: TerminalExecutionMode) -> PaneConfig {
        PaneConfig {
            id: "pane".into(),
            title: "Pane".into(),
            command: command.into(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            execution_mode: mode,
            exit_behavior: ProcessExitBehavior::AutoRestart,
            kind: PaneKind::default(),
            notify_on_exit: false,
            detector: None,
        }
    }

    fn local() -> ProjectLocation {
        ProjectLocation::Local {
            path: PathBuf::from("/work/project"),
        }
    }

    fn snapshot(provider: &'static str, session_id: Option<&str>) -> AgentSnapshot {
        let mut snapshot = AgentReducer::new(
            AgentInstanceId::random(),
            ProviderId::from_static(provider),
            1,
        )
        .snapshot()
        .clone();
        snapshot.session = session_id.map(|id| AgentSessionMetadata {
            session_id: (!id.is_empty()).then(|| id.to_string()),
            ..Default::default()
        });
        snapshot
    }

    fn plan(
        location: &ProjectLocation,
        pane: &PaneConfig,
        agent_snapshot: Option<&AgentSnapshot>,
        restore: bool,
    ) -> PaneLaunch {
        PaneLauncher::new(Path::new("/config")).plan(
            &settings(),
            &PaneLaunchRequest {
                address: &address(),
                location,
                pane,
                agent_snapshot,
                restore,
            },
        )
    }

    fn started(launch: PaneLaunch) -> PlannedLaunch {
        match launch {
            PaneLaunch::Start(planned) => *planned,
            other => panic!("expected a launch, got {other:?}"),
        }
    }

    fn agent_pane(command: &str, mode: TerminalExecutionMode) -> PaneConfig {
        PaneConfig {
            kind: PaneKind::Agent,
            ..pane(command, &[], mode)
        }
    }

    fn ended(mut snapshot: AgentSnapshot) -> AgentSnapshot {
        snapshot.process_state = AgentProcessState::Exited;
        snapshot
    }

    #[test]
    fn agent_pane_starts_its_command_when_the_session_is_another_providers() {
        let omp = agent_pane("omp", TerminalExecutionMode::Command);
        let saved = snapshot("grok", Some("grok-session"));
        let planned = started(plan(&local(), &omp, Some(&saved), true));

        assert_eq!(planned.agent.as_ref().unwrap().provider_id.as_str(), "omp");
        assert_eq!(planned.exit_behavior, omp.exit_behavior);
        assert!(!planned.clears_agent_snapshot);
        let kept = planned.saved_snapshot(Some(&saved), true).unwrap();
        assert!(kept.session.is_none());
        assert_eq!(kept.process_state, AgentProcessState::Exited);
    }

    #[test]
    fn shell_pane_does_not_resume_an_agent_that_ended_on_its_own() {
        let shell = pane("", &[], TerminalExecutionMode::Shell);
        let saved = ended(snapshot("claude", Some("claude-session")));
        let planned = started(plan(&local(), &shell, Some(&saved), true));

        assert!(planned.agent.is_none());
        assert!(matches!(
            planned.spec.execution,
            TerminalExecutionSpec::Shell { .. }
        ));
        assert!(
            !planned.clears_agent_snapshot,
            "an ended Agent stays as history"
        );
    }

    #[test]
    fn shell_pane_resumes_a_running_agent_inside_its_shell() {
        let shell = pane("", &[], TerminalExecutionMode::Shell);
        let saved = snapshot("claude", Some("claude-session"));
        let planned = started(plan(&local(), &shell, Some(&saved), true));

        assert_eq!(
            planned.spec.execution,
            TerminalExecutionSpec::Command {
                shell: "/bin/zsh".into(),
                program: "claude".into(),
                args: vec!["--resume".into(), "claude-session".into()],
                return_to_shell: true,
            }
        );
        assert_eq!(planned.exit_behavior, shell.exit_behavior);
        let restored = planned.saved_snapshot(Some(&saved), true).unwrap();
        assert_eq!(restored.process_state, AgentProcessState::Exited);
        assert!(
            restored.session.is_some(),
            "the resume confirms the session"
        );
    }

    #[test]
    fn ssh_shell_pane_resumes_an_agent_at_its_prompt() {
        let ssh = ProjectLocation::Ssh {
            connection_id: ConnectionId::new("connection"),
            root: RemotePathBuf::new("/srv/project").unwrap(),
        };
        let shell = pane("", &[], TerminalExecutionMode::Shell);

        let claude = started(plan(
            &ssh,
            &shell,
            Some(&snapshot("claude", Some("saved session"))),
            true,
        ));
        assert_eq!(
            claude.spec.execution,
            TerminalExecutionSpec::Ssh {
                connection_id: "connection".into(),
                execution: RemoteTerminalExecutionSpec::Shell {
                    command: "claude --resume 'saved session'".into()
                },
            }
        );

        let omp = started(plan(
            &ssh,
            &shell,
            Some(&snapshot("omp", Some("saved-session"))),
            true,
        ));
        let TerminalExecutionSpec::Ssh {
            execution: RemoteTerminalExecutionSpec::Shell { command },
            ..
        } = &omp.spec.execution
        else {
            panic!("expected a remote shell, got {:?}", omp.spec.execution);
        };
        assert!(command.starts_with("sh -c '"), "{command}");
    }

    #[test]
    fn shell_pane_with_an_unresumable_session_restores_a_plain_shell() {
        let shell = pane("cargo test", &[], TerminalExecutionMode::Shell);
        let saved = snapshot("claude", Some(""));
        let planned = started(plan(&local(), &shell, Some(&saved), true));

        assert!(planned.agent.is_none());
        assert!(planned.clears_agent_snapshot);
        assert!(matches!(
            planned.spec.execution,
            TerminalExecutionSpec::Shell {
                initial_command: None,
                ..
            }
        ));
    }

    #[test]
    fn explicit_restart_resumes_an_ended_agent_in_a_shell_pane() {
        let shell = pane("", &[], TerminalExecutionMode::Shell);
        let saved = ended(snapshot("claude", Some("claude-session")));
        let planned = started(plan(&local(), &shell, Some(&saved), false));

        assert!(planned.agent.is_some());
        assert_eq!(
            planned.saved_snapshot(Some(&saved), false).as_ref(),
            Some(&saved)
        );
    }

    #[test]
    fn a_new_agent_process_does_not_start_in_a_restored_shell() {
        let shell = pane("", &[], TerminalExecutionMode::Shell);
        let planned = started(plan(&local(), &shell, Some(&snapshot("omp", None)), true));

        assert!(planned.agent.is_none());
        assert!(planned.clears_agent_snapshot);
        assert!(matches!(
            planned.spec.execution,
            TerminalExecutionSpec::Shell { .. }
        ));
    }

    #[test]
    fn settings_read_host_terminal_keys_and_fall_back_per_key() {
        let config = tempfile::tempdir().unwrap();
        let candidates = ["/bin/fish".to_string()];
        assert_eq!(
            LaunchSettings::load(config.path(), &candidates),
            LaunchSettings {
                shell: "/bin/fish".into(),
                environment: BTreeMap::new(),
                scrollback: DEFAULT_SCROLLBACK,
            }
        );

        fs::write(
            config.path().join("settings.toml"),
            "[terminal]\nshell = \"/bin/bash\"\nscrollback = 0\n[terminal.environment]\nA = \"1\"\nB = 2\n",
        )
        .unwrap();
        let loaded = LaunchSettings::load(config.path(), &candidates);
        assert_eq!(loaded.shell, "/bin/bash");
        assert_eq!(loaded.scrollback, DEFAULT_SCROLLBACK);
        assert_eq!(
            loaded.environment,
            BTreeMap::from([("A".to_string(), "1".to_string())])
        );

        fs::write(
            config.path().join("settings.toml"),
            "[terminal]\nshell = \"auto\"\nscrollback = 500\n",
        )
        .unwrap();
        let loaded = LaunchSettings::load(config.path(), &[]);
        assert_eq!((loaded.shell.as_str(), loaded.scrollback), ("sh", 500));
    }

    #[test]
    fn fresh_shell_pane_runs_its_startup_command_without_inherited_hook_values() {
        let planned = started(plan(
            &local(),
            &pane("cargo test", &[], TerminalExecutionMode::Shell),
            None,
            false,
        ));
        assert_eq!(
            planned.spec.execution,
            TerminalExecutionSpec::Shell {
                program: "/bin/zsh".into(),
                args: vec!["-li".into()],
                initial_command: Some("cargo test".into()),
            }
        );
        assert_eq!(planned.spec.session_id.as_str(), "project:tab:pane");
        assert_eq!(planned.spec.scrollback_limit, 2_000);
        assert_eq!(
            planned.spec.environment,
            vec![("EDITOR".to_string(), "vim".to_string())]
        );
        assert!(
            planned
                .spec
                .removed_environment
                .contains(&"YTTT_AGENT_HOOK_TOKEN".to_string())
        );
        assert_eq!(planned.exit_behavior, ProcessExitBehavior::AutoRestart);
    }

    #[test]
    fn restore_starts_a_clean_shell_and_never_reruns_a_command_pane() {
        let planned = started(plan(
            &local(),
            &pane("cargo test", &[], TerminalExecutionMode::Shell),
            None,
            true,
        ));
        assert!(matches!(
            planned.spec.execution,
            TerminalExecutionSpec::Shell {
                initial_command: None,
                ..
            }
        ));

        let command = pane("cargo", &["test"], TerminalExecutionMode::Command);
        assert!(matches!(
            plan(&local(), &command, None, true),
            PaneLaunch::NotRestorable
        ));
        assert_eq!(
            started(plan(&local(), &command, None, false))
                .spec
                .execution,
            TerminalExecutionSpec::Command {
                shell: "/bin/zsh".into(),
                program: "cargo".into(),
                args: vec!["test".into()],
                return_to_shell: false,
            }
        );
    }

    #[test]
    fn saved_agent_session_resumes_and_an_unresumable_one_is_reported() {
        let codex = pane("codex", &["--model", "x"], TerminalExecutionMode::Command);
        let saved = snapshot("codex", Some("saved-session"));
        let planned = started(plan(&local(), &codex, Some(&saved), true));
        assert_eq!(
            planned.spec.execution,
            TerminalExecutionSpec::Command {
                shell: "/bin/zsh".into(),
                program: "codex".into(),
                args: vec!["resume".into(), "saved-session".into()],
                return_to_shell: false,
            }
        );
        assert_eq!(planned.exit_behavior, ProcessExitBehavior::ManualRestart);
        assert!(!planned.clears_agent_snapshot);

        let without_id = snapshot("codex", Some(""));
        assert!(matches!(
            plan(&local(), &codex, Some(&without_id), true),
            PaneLaunch::Unresumable
        ));

        let fresh = started(plan(&local(), &codex, None, false));
        assert_eq!(fresh.agent.unwrap().provider_id.as_str(), "codex");
        assert_eq!(fresh.exit_behavior, ProcessExitBehavior::AutoRestart);
    }

    #[test]
    fn omp_without_a_session_starts_new_omp_with_the_local_extension() {
        let agent = agent_pane("", TerminalExecutionMode::Command);
        let planned = started(plan(&local(), &agent, Some(&snapshot("omp", None)), true));
        assert_eq!(
            planned.spec.execution,
            TerminalExecutionSpec::Command {
                shell: "/bin/zsh".into(),
                program: "omp".into(),
                args: vec![
                    "--extension".into(),
                    "/config/agent-providers/omp/yttt-agent-extension.ts".into()
                ],
                return_to_shell: false,
            }
        );
        assert!(planned.clears_agent_snapshot);
    }

    #[test]
    fn stale_agent_snapshot_without_session_restores_a_plain_shell() {
        let shell = pane("", &[], TerminalExecutionMode::Shell);
        let planned = started(plan(
            &local(),
            &shell,
            Some(&snapshot("claude", None)),
            true,
        ));
        assert!(planned.agent.is_none());
        assert!(planned.clears_agent_snapshot);
        assert!(matches!(
            planned.spec.execution,
            TerminalExecutionSpec::Shell { .. }
        ));
    }

    #[test]
    fn ssh_panes_run_remotely_and_omp_installs_its_extension_first() {
        let ssh = ProjectLocation::Ssh {
            connection_id: ConnectionId::new("connection"),
            root: RemotePathBuf::new("/srv/project").unwrap(),
        };
        let planned = started(plan(
            &ssh,
            &pane("ls", &[], TerminalExecutionMode::Shell),
            None,
            false,
        ));
        assert_eq!(
            planned.spec.execution,
            TerminalExecutionSpec::Ssh {
                connection_id: "connection".into(),
                execution: RemoteTerminalExecutionSpec::Shell {
                    command: "ls".into()
                },
            }
        );

        let planned = started(plan(
            &ssh,
            &pane("omp", &[], TerminalExecutionMode::Command),
            None,
            false,
        ));
        let TerminalExecutionSpec::Ssh {
            execution: RemoteTerminalExecutionSpec::Shell { command },
            ..
        } = planned.spec.execution
        else {
            panic!("remote OMP runs through its installer wrapper");
        };
        assert!(command.contains("exec 'omp'"));
        assert!(command.ends_with(" --extension \"$yttt_agent_path\""));
    }
}
