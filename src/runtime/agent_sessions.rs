use crate::config::{
    default_layout::BuiltinAgent,
    profile::{AgentSessionAccess, AgentSessionReadPolicy},
};
use std::path::{Path, PathBuf};
use yttt_agent_core::AgentSessionMetadata;
use yttt_agent_providers::{
    session_management::{SessionOperation, mutate_agent_session as mutate_provider_session},
    sessions::{AgentSessionRoots, SessionProvider, scan_agent_sessions_for_agents_with_roots},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSession {
    pub provider: BuiltinAgent,
    pub id: String,
    pub title: String,
    pub model: Option<String>,
    pub transcript_path: Option<PathBuf>,
    pub updated_at_ms: u64,
}
impl AgentSession {
    pub fn metadata(&self) -> AgentSessionMetadata {
        AgentSessionMetadata {
            session_id: Some(self.id.clone()),
            model: self.model.clone(),
            title: (!self.title.is_empty()).then(|| self.title.clone()),
            transcript_path: self
                .transcript_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
        }
    }
}

#[derive(Debug, Default)]
pub struct AgentSessionScan {
    pub sessions: Vec<AgentSession>,
    pub warnings: Vec<String>,
}

pub fn scan_agent_sessions(
    agents: &[BuiltinAgent],
    project_path: &Path,
    access: &AgentSessionAccess,
) -> Result<AgentSessionScan, String> {
    if crate::config::storage::is_remote() {
        return Err("remote Agent history must be requested from its Host".to_string());
    }
    if access.policy() == AgentSessionReadPolicy::Disabled {
        return Ok(AgentSessionScan::default());
    }
    let Some(roots) = access.roots() else {
        return Ok(AgentSessionScan::default());
    };
    let roots = AgentSessionRoots {
        codex: roots.codex.clone(),
        claude: roots.claude.clone(),
        grok: roots.grok.clone(),
        pi: roots.pi.clone(),
        omp: roots.omp.clone(),
    };
    let providers = agents
        .iter()
        .filter_map(|agent| SessionProvider::from_id(agent.id()))
        .collect::<Vec<_>>();
    scan_agent_sessions_for_agents_with_roots(
        &providers,
        project_path,
        &roots,
        access.allows_native_commands(),
    )
    .map_err(|error| error.to_string())
    .map(|scan| AgentSessionScan {
        sessions: scan
            .sessions
            .into_iter()
            .filter_map(|session| {
                Some(AgentSession {
                    provider: BuiltinAgent::from_id(session.provider.id())?,
                    id: session.id,
                    title: session.title,
                    model: session.model,
                    transcript_path: session.transcript_path,
                    updated_at_ms: session.updated_at_ms,
                })
            })
            .collect(),
        warnings: scan.warnings,
    })
}

/// Mutates one locally available Agent session within this profile's configured history roots.
pub fn mutate_agent_session(
    provider: BuiltinAgent,
    session_id: &str,
    project_path: &Path,
    access: &AgentSessionAccess,
    operation: &SessionOperation,
) -> Result<(), String> {
    if crate::config::storage::is_remote() {
        return Err("remote Agent session changes must be requested from its Host".to_string());
    }
    let roots = mutation_session_roots(access)?;
    let provider = SessionProvider::from_id(provider.id())
        .ok_or_else(|| "unknown Agent provider".to_string())?;
    mutate_provider_session(
        provider,
        session_id,
        project_path,
        &roots,
        access.allows_native_commands(),
        operation,
    )
}

fn mutation_session_roots(access: &AgentSessionAccess) -> Result<AgentSessionRoots, String> {
    if access.policy() == AgentSessionReadPolicy::Disabled {
        return Err("Agent session history is disabled for this profile".to_string());
    }
    session_roots(access)
}

fn session_roots(access: &AgentSessionAccess) -> Result<AgentSessionRoots, String> {
    let roots = access.roots().ok_or_else(|| {
        "Agent session history roots are unavailable for this profile".to_string()
    })?;
    Ok(AgentSessionRoots {
        codex: roots.codex.clone(),
        claude: roots.claude.clone(),
        grok: roots.grok.clone(),
        pi: roots.pi.clone(),
        omp: roots.omp.clone(),
    })
}

pub fn scan_remote_agent_sessions(
    runtime: &crate::host_runtime::DesktopHostRuntime,
    agents: &[BuiltinAgent],
    project_path: &Path,
) -> Result<AgentSessionScan, String> {
    use yttt_protocol::workspace::{WorkspaceRequest, WorkspaceResponse};
    let response = runtime.workspace_request(WorkspaceRequest::AgentSessions {
        providers: agents.iter().map(|agent| agent.id().to_string()).collect(),
        project_root: yttt_protocol::HostPath::from_client_path(project_path)
            .map_err(|error| error.to_string())?,
    })?;
    let WorkspaceResponse::AgentSessions { sessions, warnings } = response else {
        return Err("unexpected Agent history response".to_string());
    };
    let sessions = sessions
        .into_iter()
        .map(|session| {
            Ok(AgentSession {
                provider: BuiltinAgent::from_id(&session.provider)
                    .ok_or_else(|| "unknown Agent provider".to_string())?,
                id: session.id,
                title: session.title,
                model: session.model,
                updated_at_ms: session.updated_at_ms,
                transcript_path: session
                    .transcript_path
                    .map(|path| path.to_client_path())
                    .transpose()
                    .map_err(|error| error.to_string())?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(AgentSessionScan { sessions, warnings })
}

/// Requests that the connected Host mutate one session in the Host's native history store.
pub fn mutate_remote_agent_session(
    runtime: &crate::host_runtime::DesktopHostRuntime,
    provider: BuiltinAgent,
    session_id: &str,
    project_path: &Path,
    operation: &SessionOperation,
) -> Result<(), String> {
    use yttt_protocol::workspace::{AgentSessionOperation, WorkspaceRequest, WorkspaceResponse};

    let operation = match operation {
        SessionOperation::Rename { title } => AgentSessionOperation::Rename {
            title: title.clone(),
        },
        SessionOperation::Delete => AgentSessionOperation::Delete,
    };
    let response = runtime.workspace_request(WorkspaceRequest::MutateAgentSession {
        provider: provider.id().to_string(),
        session_id: session_id.to_string(),
        project_root: yttt_protocol::HostPath::from_client_path(project_path)
            .map_err(|error| error.to_string())?,
        operation,
    })?;
    if matches!(response, WorkspaceResponse::AgentSessionMutated) {
        Ok(())
    } else {
        Err("unexpected Agent session mutation response".to_string())
    }
}
