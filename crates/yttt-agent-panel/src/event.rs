//! What an agent reports to the panel. A future agent core emits these; for now
//! the scripted demo driver does.
use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }
    };
}

string_id!(MessageId);
string_id!(ToolCallId);
string_id!(PermissionOptionId);
string_id!(NoticeId);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ThreadEvent {
    UserMessage {
        id: MessageId,
        content: Vec<PromptSegment>,
        attachments: Vec<Attachment>,
    },
    AgentText {
        message_id: MessageId,
        delta: String,
    },
    AgentThinking {
        message_id: MessageId,
        delta: String,
    },
    ToolCall {
        id: ToolCallId,
        kind: ToolKind,
        title: String,
        status: ToolCallStatus,
        target: Option<ToolTarget>,
    },
    ToolCallUpdate {
        id: ToolCallId,
        fields: ToolCallFields,
    },
    PermissionRequest {
        tool_call_id: ToolCallId,
        options: Vec<PermissionOption>,
    },
    PermissionResolved {
        tool_call_id: ToolCallId,
        option_id: PermissionOptionId,
    },
    Stop {
        reason: StopReason,
    },
    /// Adds a notice, or replaces the one with the same id in place (a retry
    /// counting up, say).
    Notice {
        id: NoticeId,
        kind: NoticeKind,
        message: String,
        /// Buttons on the notice; a click reports the action's id.
        actions: Vec<Action>,
    },
    ClearNotice {
        id: NoticeId,
    },
    /// Drops the prompt `from` and everything after it, before an edited
    /// prompt or a retry runs in its place.
    Truncate {
        from: MessageId,
    },
    /// Context window use, for the composer's meter.
    Usage {
        used_tokens: u64,
        limit_tokens: u64,
    },
    /// The models the agent can use, replacing the list the panel has.
    ModelCatalog {
        models: Vec<ModelInfo>,
    },
    /// The model the agent is using now, after a switch from this panel or
    /// from anywhere else.
    ActiveModel {
        id: String,
    },
    /// A title the agent chose, such as a summary of the first exchange.
    /// A title the user typed wins over it.
    Title {
        title: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    /// Groups the picker, like the service that hosts the model.
    pub provider: String,
    pub context_window: Option<u64>,
    pub capabilities: ModelCapabilities,
    pub status: ModelStatus,
    /// An SVG asset path for the model or its provider; the picker shows a
    /// generic chip without one.
    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub thinking: bool,
    pub vision: bool,
    pub tools: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelStatus {
    #[default]
    Available,
    /// Usable once configured, such as with an API key.
    NeedsSetup,
    /// Not usable now, and why.
    Unavailable(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoticeKind {
    Info,
    Warning,
    /// The turn failed; the last error offers a retry.
    Error,
    /// A request is being retried, such as after a rate limit.
    Retrying,
    /// Earlier messages were summarized to free context.
    Compaction,
}

/// A button the agent offers on a notice or a tool card. Clicking it sends
/// its `id` back; the agent decides what it does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub id: String,
    pub label: String,
}

impl Action {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

/// What the user sends: text with mentions, and attachments.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prompt {
    pub segments: Vec<PromptSegment>,
    pub attachments: Vec<Attachment>,
}

impl Prompt {
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty() && self.attachments.is_empty()
    }
}

/// Something sent along with a prompt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub kind: AttachmentKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachmentKind {
    /// Pasted image bytes; `mime` is like `image/png`.
    Image { mime: String, data: Vec<u8> },
    /// A file on disk; images among them preview from the path.
    File { path: String },
}

impl AttachmentKind {
    pub fn is_image(&self) -> bool {
        match self {
            Self::Image { .. } => true,
            Self::File { path } => matches!(
                path.rsplit_once('.')
                    .map(|(_, extension)| extension.to_ascii_lowercase())
                    .as_deref(),
                Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg")
            ),
        }
    }
}

/// One piece of a submitted prompt. A mention's `id` names the resource
/// (for example `file:///abs/path`); `label` is what the chip shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptSegment {
    Text(String),
    Mention { id: String, label: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolKind {
    Read,
    /// Creates or overwrites a whole file.
    Write,
    Edit,
    Delete,
    Move,
    /// Searches the project: grep, glob, symbols.
    Search,
    Execute,
    Think,
    Fetch,
    WebSearch,
    Skill,
    /// Writes the agent's task list.
    Todo,
    /// A tool from an MCP server.
    Mcp,
    /// Hands a task to a subagent.
    Agent,
    Other,
}

/// What a tool call acts on. The card header shows it in the form that fits
/// (a path, a command, a query); without one the header shows the title.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolTarget {
    /// `lines` is an inclusive, 1-based range.
    File {
        path: String,
        lines: Option<(u32, u32)>,
    },
    /// A project search; `scope` narrows it, like a directory or glob.
    Pattern {
        pattern: String,
        scope: Option<String>,
    },
    Command {
        command: String,
        cwd: Option<String>,
    },
    Query(String),
    Url(String),
    Skill {
        name: String,
    },
    Mcp {
        server: String,
        tool: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolCallStatus {
    Pending,
    Running,
    AwaitingPermission,
    Completed,
    Failed,
    Rejected,
    Canceled,
}

impl ToolCallStatus {
    pub fn is_finished(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Rejected | Self::Canceled
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolContent {
    Markdown(String),
    /// `old_text` is `None` for a file the tool created.
    Diff {
        path: String,
        old_text: Option<String>,
        new_text: String,
    },
    /// Raw terminal output: ANSI colors and carriage returns are honored.
    Terminal {
        command: String,
        output: String,
        exit_code: Option<i32>,
    },
    /// Text read from a file, starting at line `first_line` (1-based).
    File {
        path: String,
        text: String,
        first_line: u32,
    },
    SearchResults {
        /// Highlighted in matching lines when given.
        query: Option<String>,
        files: Vec<FileMatches>,
    },
    WebResults(Vec<WebResult>),
    WebPage {
        url: String,
        title: String,
        /// Markdown.
        excerpt: String,
    },
    Image {
        source: ImageSource,
        caption: Option<String>,
    },
    /// Structured input or output, such as an MCP tool's arguments.
    Json {
        label: String,
        text: String,
    },
    Todo(Vec<TodoItem>),
    Error(String),
    /// Content only the host knows how to show, such as an MCP tool's own
    /// result: the panel draws it with the renderer registered for `kind`,
    /// or as JSON without one. `data` is JSON text.
    Custom {
        kind: String,
        data: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMatches {
    pub path: String,
    pub lines: Vec<MatchLine>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchLine {
    pub number: u32,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageSource {
    Path(String),
    Url(String),
    /// Encoded image bytes; `mime` is like `image/png`.
    Bytes {
        mime: String,
        data: Vec<u8>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub text: String,
    pub status: TodoStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

/// A partial update: only the fields that are `Some` change, and `content`
/// replaces the card's content as a whole.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallFields {
    pub title: Option<String>,
    pub status: Option<ToolCallStatus>,
    pub target: Option<ToolTarget>,
    pub content: Option<Vec<ToolContent>>,
    /// Buttons for the card, like "Run again"; replaces the card's actions.
    pub actions: Option<Vec<Action>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOption {
    pub id: PermissionOptionId,
    pub label: String,
    pub kind: PermissionOptionKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionOptionKind {
    AllowOnce,
    AllowAlways,
    Reject,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    Cancelled,
    Error { message: String },
}
