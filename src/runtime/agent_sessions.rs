use crate::config::{
    default_layout::BuiltinAgent,
    profile::{AgentSessionAccess, AgentSessionReadPolicy},
};
use std::path::{Path, PathBuf};
use yttt_agent_core::AgentSessionMetadata;
use yttt_agent_providers::sessions::{
    AgentSessionRoots, SessionProvider, scan_agent_sessions_for_agents_with_roots,
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

pub fn scan_agent_sessions(
    agents: &[BuiltinAgent],
    project_path: &Path,
    access: &AgentSessionAccess,
) -> Result<Vec<AgentSession>, String> {
    if crate::config::storage::is_remote() {
        return Err("remote Agent history must be requested from its Host".to_string());
    }
    if access.policy() == AgentSessionReadPolicy::Disabled {
        return Ok(Vec::new());
    }
    let Some(roots) = access.roots() else {
        return Ok(Vec::new());
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
    .map(|sessions| {
        sessions
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
            .collect()
    })
}

pub fn scan_remote_agent_sessions(
    runtime: &crate::host_runtime::DesktopHostRuntime,
    agents: &[BuiltinAgent],
    project_path: &Path,
) -> Result<Vec<AgentSession>, String> {
    use yttt_protocol::workspace::{WorkspaceRequest, WorkspaceResponse};
    let response = runtime.workspace_request(WorkspaceRequest::AgentSessions {
        providers: agents.iter().map(|agent| agent.id().to_string()).collect(),
        project_root: yttt_protocol::HostPath::from_path(project_path)
            .map_err(|error| error.to_string())?,
    })?;
    let WorkspaceResponse::AgentSessions(sessions) = response else {
        return Err("unexpected Agent history response".to_string());
    };
    sessions
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
                    .map(|path| path.to_path())
                    .transpose()
                    .map_err(|error| error.to_string())?,
            })
        })
        .collect()
}
