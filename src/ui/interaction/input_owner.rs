use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use gpui::FocusHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputOwnerKind {
    Workspace,
    Palette,
    Settings,
    Dialog,
    Editor,
    KeybindingRecorder,
    ContextMenu,
    Popover,
}

impl InputOwnerKind {
    fn default_scope_id(self) -> InputScopeId {
        let scope_id = match self {
            InputOwnerKind::Workspace => "workspace",
            InputOwnerKind::Palette => "palette",
            InputOwnerKind::Settings => "settings",
            InputOwnerKind::Dialog => "dialog",
            InputOwnerKind::Editor => "editor",
            InputOwnerKind::KeybindingRecorder => "keybinding_recorder",
            InputOwnerKind::ContextMenu => "context_menu",
            InputOwnerKind::Popover => "popover",
        };
        InputScopeId::new(scope_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InputScopeId(String);

impl InputScopeId {
    pub fn new(scope_id: impl Into<String>) -> Self {
        Self(scope_id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for InputScopeId {
    fn default() -> Self {
        InputOwnerKind::Workspace.default_scope_id()
    }
}

impl fmt::Display for InputScopeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<&str> for InputScopeId {
    fn from(scope_id: &str) -> Self {
        Self::new(scope_id)
    }
}

impl From<String> for InputScopeId {
    fn from(scope_id: String) -> Self {
        Self::new(scope_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputOwnerRegistration {
    kind: InputOwnerKind,
    scope_id: InputScopeId,
    focus_handle: Option<FocusHandle>,
}

impl InputOwnerRegistration {
    pub fn new(kind: InputOwnerKind, scope_id: impl Into<InputScopeId>) -> Self {
        Self {
            kind,
            scope_id: scope_id.into(),
            focus_handle: None,
        }
    }

    pub fn workspace() -> Self {
        Self::new(
            InputOwnerKind::Workspace,
            InputOwnerKind::Workspace.default_scope_id(),
        )
    }

    pub fn blocking(kind: InputOwnerKind, scope_id: impl Into<InputScopeId>) -> Self {
        Self::new(kind, scope_id)
    }

    pub fn with_focus_handle(mut self, focus_handle: FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle);
        self
    }

    pub fn kind(&self) -> InputOwnerKind {
        self.kind
    }

    pub fn scope_id(&self) -> &InputScopeId {
        &self.scope_id
    }

    pub fn focus_handle(&self) -> Option<&FocusHandle> {
        self.focus_handle.as_ref()
    }
}

impl From<InputOwnerKind> for InputOwnerRegistration {
    fn from(kind: InputOwnerKind) -> Self {
        Self::new(kind, kind.default_scope_id())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalInputPolicy {
    Allow,
    Block,
}

impl TerminalInputPolicy {
    pub fn for_owner(kind: InputOwnerKind) -> Self {
        if kind == InputOwnerKind::Workspace {
            TerminalInputPolicy::Allow
        } else {
            TerminalInputPolicy::Block
        }
    }

    pub fn allows_terminal_input(self) -> bool {
        self == TerminalInputPolicy::Allow
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputOwnerSnapshot {
    active_kind: InputOwnerKind,
    active_scope_id: InputScopeId,
    terminal_input_policy: TerminalInputPolicy,
    focus_handle: Option<FocusHandle>,
}

impl InputOwnerSnapshot {
    fn new(
        active_kind: InputOwnerKind,
        active_scope_id: InputScopeId,
        focus_handle: Option<FocusHandle>,
    ) -> Self {
        Self {
            active_kind,
            active_scope_id,
            terminal_input_policy: TerminalInputPolicy::for_owner(active_kind),
            focus_handle,
        }
    }

    pub fn active_kind(&self) -> InputOwnerKind {
        self.active_kind
    }

    pub fn active_scope_id(&self) -> &InputScopeId {
        &self.active_scope_id
    }

    pub fn active_scope(&self) -> &InputScopeId {
        self.active_scope_id()
    }

    pub fn terminal_input_policy(&self) -> TerminalInputPolicy {
        self.terminal_input_policy
    }

    pub fn terminal_input_allowed(&self) -> bool {
        self.terminal_input_policy.allows_terminal_input()
    }

    pub fn focus_handle(&self) -> Option<&FocusHandle> {
        self.focus_handle.as_ref()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InputOwnerToken(u64);

impl InputOwnerToken {
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub fn from_raw_for_test(raw: u64) -> Self {
        Self(raw)
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InputOwnerEntry {
    token: InputOwnerToken,
    registration: InputOwnerRegistration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputOwnerStack {
    next_token: u64,
    workspace_scope_id: InputScopeId,
    entries: Vec<InputOwnerEntry>,
}

impl Default for InputOwnerStack {
    fn default() -> Self {
        Self {
            next_token: 0,
            workspace_scope_id: InputOwnerKind::Workspace.default_scope_id(),
            entries: Vec::new(),
        }
    }
}

impl InputOwnerStack {
    pub fn push(&mut self, owner: impl Into<InputOwnerRegistration>) -> InputOwnerToken {
        self.push_owner(owner)
    }

    pub fn push_owner(&mut self, owner: impl Into<InputOwnerRegistration>) -> InputOwnerToken {
        self.next_token = self.next_token.saturating_add(1);
        let token = InputOwnerToken(self.next_token);
        self.entries.push(InputOwnerEntry {
            token,
            registration: owner.into(),
        });
        token
    }

    pub fn pop(&mut self, token: InputOwnerToken) -> bool {
        let found = self.entries.iter().any(|entry| entry.token == token);
        if found {
            self.pop_owner(token);
        }
        found
    }

    pub fn pop_owner(&mut self, token: InputOwnerToken) -> InputOwnerSnapshot {
        if let Some(index) = self.entries.iter().position(|entry| entry.token == token) {
            self.entries.truncate(index);
        }
        self.snapshot()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn active_kind(&self) -> InputOwnerKind {
        self.entries
            .last()
            .map(|entry| entry.registration.kind())
            .unwrap_or(InputOwnerKind::Workspace)
    }

    pub fn active_scope_id(&self) -> &InputScopeId {
        self.entries
            .last()
            .map(|entry| entry.registration.scope_id())
            .unwrap_or(&self.workspace_scope_id)
    }

    pub fn active_focus_handle(&self) -> Option<&FocusHandle> {
        self.entries
            .last()
            .and_then(|entry| entry.registration.focus_handle())
    }

    pub fn active_owner(&self) -> InputOwnerSnapshot {
        self.snapshot()
    }

    pub fn focus_restore_target(&self) -> Option<&FocusHandle> {
        self.active_focus_handle()
    }

    pub fn snapshot(&self) -> InputOwnerSnapshot {
        InputOwnerSnapshot::new(
            self.active_kind(),
            self.active_scope_id().clone(),
            self.active_focus_handle().cloned(),
        )
    }

    pub fn terminal_input_allowed(&self) -> bool {
        self.snapshot().terminal_input_allowed()
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

    pub fn sync_from_snapshot(&self, snapshot: &InputOwnerSnapshot) {
        self.set_allowed(snapshot.terminal_input_allowed());
    }

    pub fn sync_from_owner(&self, owner: InputOwnerKind) {
        self.set_allowed(TerminalInputPolicy::for_owner(owner).allows_terminal_input());
    }

    pub fn allows_terminal_input(&self) -> bool {
        self.allowed.load(Ordering::SeqCst)
    }
}
