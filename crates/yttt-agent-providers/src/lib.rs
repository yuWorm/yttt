#![forbid(unsafe_code)]

use std::sync::Arc;

use yttt_agent_core::AgentProvider;

mod command;
pub mod installer;
mod omp;
pub mod sessions;
mod sources;

pub use command::{
    CLAUDE_PROVIDER_ID, CODEX_PROVIDER_ID, ClaudeProvider, CodexProvider, GROK_PROVIDER_ID,
    GrokProvider, OPENCODE_PROVIDER_ID, OpenCodeProvider, PI_PROVIDER_ID, PiProvider,
};
pub use omp::{OMP_EXTENSION_FILE_NAME, OMP_EXTENSION_SOURCE, OMP_PROVIDER_ID, OmpProvider};
pub use sources::{OPENCODE_PLUGIN_SOURCE, PI_EXTENSION_SOURCE};

pub fn builtin_providers() -> Vec<Arc<dyn AgentProvider>> {
    vec![
        Arc::new(CodexProvider),
        Arc::new(ClaudeProvider),
        Arc::new(GrokProvider),
        Arc::new(OpenCodeProvider),
        Arc::new(PiProvider),
        Arc::new(OmpProvider),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_providers_include_grok() {
        let grok = builtin_providers()
            .into_iter()
            .find(|provider| provider.descriptor().id.as_str() == GROK_PROVIDER_ID)
            .expect("Grok must be registered as a built-in provider");

        assert_eq!(grok.descriptor().display_name, "Grok");
    }
}
