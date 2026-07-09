use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputOwnerKind {
    Workspace,
    Palette,
    Settings,
    Dialog,
    Editor,
    KeybindingRecorder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InputOwnerToken(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InputOwnerEntry {
    token: InputOwnerToken,
    kind: InputOwnerKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputOwnerStack {
    next_token: u64,
    entries: Vec<InputOwnerEntry>,
}

impl InputOwnerStack {
    pub fn push(&mut self, kind: InputOwnerKind) -> InputOwnerToken {
        self.next_token = self.next_token.saturating_add(1);
        let token = InputOwnerToken(self.next_token);
        self.entries.push(InputOwnerEntry { token, kind });
        token
    }

    pub fn pop(&mut self, token: InputOwnerToken) -> bool {
        let Some(index) = self.entries.iter().position(|entry| entry.token == token) else {
            return false;
        };
        self.entries.truncate(index);
        true
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn active_kind(&self) -> InputOwnerKind {
        self.entries
            .last()
            .map(|entry| entry.kind)
            .unwrap_or(InputOwnerKind::Workspace)
    }

    pub fn terminal_input_allowed(&self) -> bool {
        self.active_kind() == InputOwnerKind::Workspace
    }
}

#[derive(Clone, Debug)]
pub struct TerminalInputGate {
    allowed: Arc<AtomicBool>,
}

impl Default for TerminalInputGate {
    fn default() -> Self {
        Self {
            allowed: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl TerminalInputGate {
    pub fn shared_flag(&self) -> Arc<AtomicBool> {
        self.allowed.clone()
    }

    pub fn set_allowed(&self, allowed: bool) {
        self.allowed.store(allowed, Ordering::SeqCst);
    }

    pub fn sync_from_owner(&self, owner: InputOwnerKind) {
        self.set_allowed(owner == InputOwnerKind::Workspace);
    }

    pub fn allows_terminal_input(&self) -> bool {
        self.allowed.load(Ordering::SeqCst)
    }
}
