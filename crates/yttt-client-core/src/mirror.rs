use yttt_core::model::ids::TerminalSessionId;
use yttt_protocol::terminal::{
    SemanticViewport, TerminalProcessState, TerminalStreamApply, TerminalStreamUpdate,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorApply {
    Updated,
    Ignored,
    SequenceGap,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalMirrorMetadata {
    pub title: Option<String>,
    pub process_state: TerminalProcessState,
    pub session_epoch: u64,
    pub sequence: u64,
}

#[derive(Clone, Debug)]
pub struct TerminalMirror {
    viewport: SemanticViewport,
}

impl TerminalMirror {
    pub fn new(viewport: SemanticViewport) -> Self {
        Self { viewport }
    }

    pub fn session_id(&self) -> &TerminalSessionId {
        &self.viewport.session_id
    }

    pub fn viewport(&self) -> &SemanticViewport {
        &self.viewport
    }

    pub fn metadata(&self) -> TerminalMirrorMetadata {
        TerminalMirrorMetadata {
            title: self.viewport.modes.title.clone(),
            process_state: self.viewport.process_state.clone(),
            session_epoch: self.viewport.session_epoch,
            sequence: self.viewport.sequence,
        }
    }

    pub fn apply(&mut self, update: &TerminalStreamUpdate) -> MirrorApply {
        if matches!(update, TerminalStreamUpdate::RawTail { .. }) {
            return MirrorApply::SequenceGap;
        }
        match self.viewport.apply_stream_update_ref(update) {
            TerminalStreamApply::Updated(_) => MirrorApply::Updated,
            TerminalStreamApply::Ignored => MirrorApply::Ignored,
            TerminalStreamApply::SequenceGap => MirrorApply::SequenceGap,
        }
    }
}
