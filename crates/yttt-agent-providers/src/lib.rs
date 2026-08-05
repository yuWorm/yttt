#![forbid(unsafe_code)]

use std::sync::Arc;

use yttt_agent_core::AgentProvider;

mod command;
mod omp;
mod sources;

pub use command::{
    CLAUDE_PROVIDER_ID, CODEX_PROVIDER_ID, ClaudeProvider, CodexProvider, OPENCODE_PROVIDER_ID,
    OpenCodeProvider, PI_PROVIDER_ID, PiProvider,
};
pub use omp::{OMP_EXTENSION_FILE_NAME, OMP_EXTENSION_SOURCE, OMP_PROVIDER_ID, OmpProvider};
pub use sources::{OPENCODE_PLUGIN_SOURCE, PI_EXTENSION_SOURCE};

pub fn builtin_providers() -> Vec<Arc<dyn AgentProvider>> {
    vec![
        Arc::new(CodexProvider),
        Arc::new(ClaudeProvider),
        Arc::new(OpenCodeProvider),
        Arc::new(PiProvider),
        Arc::new(OmpProvider),
    ]
}
