//! A stand-in for the agent: plays a fixed turn into the panel, pausing for
//! permission like a real agent would. It drives the UI only through
//! [`ThreadEvent`]s and [`AgentPanelEvent`]s, the same seam a real agent
//! core will use.
use std::{collections::VecDeque, rc::Rc, time::Duration};

use gpui::{
    AnyElement, Context, Entity, EventEmitter, IntoElement as _, ParentElement as _, SharedString,
    Styled as _, Subscription, Task, div, px,
};
use gpui_component::IconNamed as _;
use gpui_component::{h_flex, v_flex};
use yttt_ui::{
    Icon, IconName,
    primitives::icon_button::{YtttIconButtonKind, yttt_icon_button},
    style::UiStyle,
    theme::WorkbenchTheme,
};

use crate::{
    composer::{
        Candidate, CandidateSource, SelectorKind, SelectorOption,
        trigger::{TriggerKind, rank},
    },
    event::{
        Action, Attachment, AttachmentKind, FileMatches, ImageSource, MatchLine, MessageId,
        ModelCapabilities, ModelInfo, ModelStatus, NoticeId, NoticeKind, PermissionOption,
        PermissionOptionId, PermissionOptionKind, Prompt, PromptSegment, StopReason, ThreadEvent,
        TodoItem, TodoStatus, ToolCallFields, ToolCallId, ToolCallStatus, ToolContent, ToolKind,
        ToolTarget, WebResult,
    },
    model::ThreadStatus,
    panel::{AgentPanel, AgentPanelEvent},
    slots::{MessageAction, MessagePlacement, Slot, SlotItem},
    thread_view::prompt_text,
};

const STREAM_CHUNK_WORDS: usize = 3;
const STREAM_DELAY_MS: u64 = 40;
const ALLOW_ONCE: &str = "allow-once";
const REJECT: &str = "reject";
/// The error notice's button that runs the turn again.
const RETRY_FAST: &str = "retry-fast";

pub enum DriverEvent {
    /// The panel is waiting for a permission answer.
    AwaitingPermission,
    TurnFinished,
}

enum Step {
    Emit(ThreadEvent),
    Pause(u64),
    /// Asks for permission and waits; rejecting plays `rejected` instead of
    /// the rest of the turn.
    AwaitPermission {
        tool_call_id: ToolCallId,
        rejected: Vec<Step>,
    },
}

pub struct ScriptedDriver {
    panel: Entity<AgentPanel>,
    /// The latest prompt and its message, for a retry to run again.
    last: Option<(MessageId, Prompt)>,
    queue: VecDeque<Step>,
    waiting: Option<(ToolCallId, Vec<Step>)>,
    timer: Option<Task<()>>,
    speed: f32,
    auto_allow: bool,
    turn: usize,
    _subscription: Subscription,
}

impl EventEmitter<DriverEvent> for ScriptedDriver {}

impl ScriptedDriver {
    /// `speed` scales every delay; `auto_allow` answers permission prompts
    /// itself, for unattended captures.
    pub fn new(
        panel: Entity<AgentPanel>,
        speed: f32,
        auto_allow: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.subscribe(&panel, |this, _, event, cx| match event {
            AgentPanelEvent::Submit { prompt, edit_of } => {
                this.start(prompt.clone(), edit_of.clone(), cx)
            }
            AgentPanelEvent::PermissionChosen {
                tool_call_id,
                option_id,
            } => this.resolve(tool_call_id, option_id.clone(), cx),
            AgentPanelEvent::Stop => this.stop(cx),
            AgentPanelEvent::Retry => this.retry(cx),
            AgentPanelEvent::NewThread => this.reset(),
            AgentPanelEvent::NoticeAction { action_id, .. } if action_id == RETRY_FAST => {
                this.retry(cx)
            }
            AgentPanelEvent::ModelSelected { id } => this.confirm_model(id.clone(), cx),
            AgentPanelEvent::RefreshModels => this.refresh_models(cx),
            // The demo has no files, models or plans to act on; a host
            // would handle these.
            AgentPanelEvent::OpenPath { .. }
            | AgentPanelEvent::KeepEdits { .. }
            | AgentPanelEvent::UndoEdits { .. }
            | AgentPanelEvent::Selected { .. }
            | AgentPanelEvent::NoticeAction { .. }
            | AgentPanelEvent::ToolAction { .. }
            | AgentPanelEvent::MessageAction { .. }
            | AgentPanelEvent::PlanAction { .. }
            | AgentPanelEvent::PlanStepClicked { .. }
            | AgentPanelEvent::TitleChanged { .. }
            | AgentPanelEvent::FavoriteModel { .. }
            | AgentPanelEvent::ModelSetup { .. }
            | AgentPanelEvent::ManageModels => {}
        });
        Self {
            panel,
            last: None,
            queue: VecDeque::new(),
            waiting: None,
            timer: None,
            speed: speed.max(0.01),
            auto_allow,
            turn: 0,
            _subscription: subscription,
        }
    }

    /// Plays a turn for `prompt`; `edit_of` first drops the message it
    /// rewrites. A prompt mentioning "fail" plays a turn that errors.
    pub fn start(&mut self, prompt: Prompt, edit_of: Option<MessageId>, cx: &mut Context<Self>) {
        let text = prompt_text(&prompt.segments);
        self.reset();
        if text.trim_start().starts_with("/clear") {
            self.panel.update(cx, |panel, cx| panel.clear(cx));
            return;
        }
        if let Some(from) = edit_of {
            self.emit(ThreadEvent::Truncate { from }, cx);
        }
        self.turn += 1;
        self.last = Some((
            MessageId(format!("turn-{}-user", self.turn)),
            prompt.clone(),
        ));
        self.queue = if text.to_lowercase().contains("fail") {
            failing_scenario(self.turn, prompt)
        } else {
            scenario(self.turn, prompt)
        }
        .into();
        self.advance(cx);
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        if let Some((id, prompt)) = self.last.clone() {
            self.start(prompt, Some(id), cx);
        }
    }

    /// Answers a pick the way an agent would: switch, then confirm.
    fn confirm_model(&mut self, id: String, cx: &mut Context<Self>) {
        let delay = Duration::from_millis((300. / self.speed) as u64);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |this, cx| {
                this.emit(ThreadEvent::ActiveModel { id }, cx)
            })
            .ok();
        })
        .detach();
    }

    /// Sends a fresh list after a moment; by then the local server is up.
    fn refresh_models(&mut self, cx: &mut Context<Self>) {
        let delay = Duration::from_millis((900. / self.speed) as u64);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |this, cx| {
                this.emit(
                    ThreadEvent::ModelCatalog {
                        models: demo_models(true),
                    },
                    cx,
                )
            })
            .ok();
        })
        .detach();
    }

    /// Forgets the turn in progress without telling the panel.
    fn reset(&mut self) {
        self.timer = None;
        self.queue.clear();
        self.waiting = None;
    }

    fn advance(&mut self, cx: &mut Context<Self>) {
        while let Some(step) = self.queue.pop_front() {
            match step {
                Step::Emit(event) => {
                    let finished = matches!(event, ThreadEvent::Stop { .. });
                    self.emit(event, cx);
                    if finished {
                        cx.emit(DriverEvent::TurnFinished);
                    }
                }
                Step::Pause(ms) => {
                    let delay = Duration::from_millis((ms as f32 / self.speed) as u64);
                    self.timer = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(delay).await;
                        this.update(cx, |this, cx| this.advance(cx)).ok();
                    }));
                    return;
                }
                Step::AwaitPermission {
                    tool_call_id,
                    rejected,
                } => {
                    self.emit(
                        ThreadEvent::PermissionRequest {
                            tool_call_id: tool_call_id.clone(),
                            options: permission_options(),
                        },
                        cx,
                    );
                    cx.emit(DriverEvent::AwaitingPermission);
                    if self.auto_allow {
                        let id = tool_call_id.clone();
                        let delay = Duration::from_millis((600. / self.speed) as u64);
                        self.timer = Some(cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(delay).await;
                            this.update(cx, |this, cx| {
                                this.resolve(&id, PermissionOptionId::from(ALLOW_ONCE), cx)
                            })
                            .ok();
                        }));
                    }
                    self.waiting = Some((tool_call_id, rejected));
                    return;
                }
            }
        }
    }

    fn resolve(
        &mut self,
        tool_call_id: &ToolCallId,
        option_id: PermissionOptionId,
        cx: &mut Context<Self>,
    ) {
        let Some((waiting_id, rejected)) = self.waiting.take() else {
            return;
        };
        if &waiting_id != tool_call_id {
            self.waiting = Some((waiting_id, rejected));
            return;
        }
        self.timer = None;
        let rejecting = option_id.0 == REJECT;
        self.emit(
            ThreadEvent::PermissionResolved {
                tool_call_id: waiting_id,
                option_id,
            },
            cx,
        );
        if rejecting {
            self.queue = rejected.into();
        }
        self.advance(cx);
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.reset();
        self.emit(
            ThreadEvent::Stop {
                reason: StopReason::Cancelled,
            },
            cx,
        );
        cx.emit(DriverEvent::TurnFinished);
    }

    fn emit(&self, event: ThreadEvent, cx: &mut Context<Self>) {
        self.panel
            .update(cx, |panel, cx| panel.apply_event(event, cx));
    }
}

fn permission_options() -> Vec<PermissionOption> {
    [
        (ALLOW_ONCE, "Allow once", PermissionOptionKind::AllowOnce),
        (
            "allow-always",
            "Always allow",
            PermissionOptionKind::AllowAlways,
        ),
        (REJECT, "Reject", PermissionOptionKind::Reject),
    ]
    .into_iter()
    .map(|(id, label, kind)| PermissionOption {
        id: id.into(),
        label: label.to_owned(),
        kind,
    })
    .collect()
}

/// Streams `text` a few words at a time.
fn stream(message: &MessageId, thinking: bool, text: &str) -> Vec<Step> {
    let words = text.split_inclusive(' ').collect::<Vec<_>>();
    words
        .chunks(STREAM_CHUNK_WORDS)
        .flat_map(|chunk| {
            let delta = chunk.concat();
            let event = if thinking {
                ThreadEvent::AgentThinking {
                    message_id: message.clone(),
                    delta,
                }
            } else {
                ThreadEvent::AgentText {
                    message_id: message.clone(),
                    delta,
                }
            };
            [Step::Emit(event), Step::Pause(STREAM_DELAY_MS)]
        })
        .collect()
}

fn tool_call(
    id: &ToolCallId,
    kind: ToolKind,
    title: &str,
    target: Option<ToolTarget>,
    status: ToolCallStatus,
) -> Step {
    Step::Emit(ThreadEvent::ToolCall {
        id: id.clone(),
        kind,
        title: title.to_owned(),
        status,
        target,
    })
}

fn update(id: &ToolCallId, status: Option<ToolCallStatus>, content: Vec<ToolContent>) -> Step {
    Step::Emit(ThreadEvent::ToolCallUpdate {
        id: id.clone(),
        fields: ToolCallFields {
            status,
            content: Some(content),
            ..Default::default()
        },
    })
}

fn actions(id: &ToolCallId, actions: Vec<Action>) -> Step {
    Step::Emit(ThreadEvent::ToolCallUpdate {
        id: id.clone(),
        fields: ToolCallFields {
            actions: Some(actions),
            ..Default::default()
        },
    })
}

fn set_status(id: &ToolCallId, status: ToolCallStatus) -> Step {
    Step::Emit(ThreadEvent::ToolCallUpdate {
        id: id.clone(),
        fields: ToolCallFields {
            status: Some(status),
            ..Default::default()
        },
    })
}

fn file(path: &str, lines: Option<(u32, u32)>) -> Option<ToolTarget> {
    Some(ToolTarget::File {
        path: path.to_owned(),
        lines,
    })
}

/// The plan with the first `done` items finished and the next one underway.
fn plan(done: usize) -> Vec<ToolContent> {
    const STEPS: [&str; 4] = [
        "Find where the timeout is set",
        "Check how reqwest applies timeouts",
        "Make the timeout configurable",
        "Add a test and run the suite",
    ];
    vec![ToolContent::Todo(
        STEPS
            .iter()
            .enumerate()
            .map(|(ix, text)| TodoItem {
                text: (*text).to_owned(),
                status: match ix.cmp(&done) {
                    std::cmp::Ordering::Less => TodoStatus::Completed,
                    std::cmp::Ordering::Equal => TodoStatus::InProgress,
                    std::cmp::Ordering::Greater => TodoStatus::Pending,
                },
            })
            .collect(),
    )]
}

fn matched(path: &str, lines: &[(u32, &str)]) -> FileMatches {
    FileMatches {
        path: path.to_owned(),
        lines: lines
            .iter()
            .map(|(number, text)| MatchLine {
                number: *number,
                text: (*text).to_owned(),
            })
            .collect(),
    }
}

const CONFIG_RS: &str = "\
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub base_url: String,
    /// Seconds before a request gives up.
    #[serde(default = \"default_timeout\")]
    pub request_timeout_secs: u64,
}

fn default_timeout() -> u64 {
    30
}
";

const CLIENT_BEFORE: &str = "\
use std::time::Duration;

pub struct Client {
    http: reqwest::Client,
}

impl Client {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect(\"http client\");
        Self { http }
    }
}
";

const CLIENT_AFTER: &str = "\
use std::time::Duration;

use crate::config::Config;

pub struct Client {
    http: reqwest::Client,
}

impl Client {
    pub fn new(config: &Config) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .expect(\"http client\");
        Self { http }
    }
}
";

const TIMEOUT_TEST: &str = "\
use demo_client::{Client, Config};

#[test]
fn timeout_comes_from_config() {
    let config = Config {
        base_url: \"http://localhost\".into(),
        request_timeout_secs: 5,
    };
    let client = Client::new(&config);
    assert_eq!(client.timeout().as_secs(), 5);
}
";

const DEPENDENCIES: [&str; 18] = [
    "libc v0.2.170",
    "cfg-if v1.0.0",
    "once_cell v1.21.3",
    "bytes v1.10.1",
    "itoa v1.0.15",
    "pin-project-lite v0.2.16",
    "futures-core v0.3.31",
    "tokio v1.45.0",
    "http v1.3.1",
    "serde v1.0.219",
    "serde_json v1.0.140",
    "url v2.5.4",
    "hyper v1.6.0",
    "rustls v0.23.27",
    "hyper-util v0.1.11",
    "tower v0.5.2",
    "reqwest v0.12.15",
    "demo-client v0.1.0",
];

/// Colored like cargo prints it, including a progress line that redraws
/// itself with carriage returns.
fn test_output_steps(id: &ToolCallId) -> Vec<Step> {
    let command = "cargo test -p demo-client".to_owned();
    let mut output = String::from(
        "\x1b[1m\x1b[36m    Building\x1b[0m [>        ] 0/18\r\x1b[1m\x1b[36m    Building\x1b[0m [=====>   ] 9/18\r\x1b[1m\x1b[36m    Building\x1b[0m [=========] 18/18",
    );
    let mut steps = Vec::new();
    let push = |output: &str, exit_code: Option<i32>, steps: &mut Vec<Step>| {
        steps.push(Step::Pause(160));
        steps.push(update(
            id,
            exit_code.map(|_| ToolCallStatus::Completed),
            vec![ToolContent::Terminal {
                command: command.clone(),
                output: output.to_owned(),
                exit_code,
            }],
        ));
    };
    push(&output, None, &mut steps);
    for dependency in DEPENDENCIES {
        output.push_str(&format!(
            "\n\x1b[1m\x1b[32m   Compiling\x1b[0m {dependency}"
        ));
        push(&output, None, &mut steps);
    }
    output.push_str(
        "\n\x1b[1m\x1b[32m    Finished\x1b[0m `test` profile [unoptimized + debuginfo] in 6.82s\
         \n\x1b[1m\x1b[32m     Running\x1b[0m tests/timeout.rs\
         \n\nrunning 4 tests\
         \ntest default_timeout_is_30s ... \x1b[32mok\x1b[0m\
         \ntest builds_with_config ... \x1b[32mok\x1b[0m\
         \ntest timeout_comes_from_config ... \x1b[32mok\x1b[0m\
         \ntest slow_proxy_times_out ... \x1b[33mignored\x1b[0m\
         \n\ntest result: \x1b[32mok\x1b[0m. 3 passed; 0 failed; 1 ignored",
    );
    push(&output, Some(0), &mut steps);
    steps
}

/// One full turn that touches every kind of tool card: a skill, a plan,
/// reads, searches, the web, an image, an MCP tool, an edit that needs
/// permission, a new file and a test run.
fn scenario(turn: usize, prompt: Prompt) -> Vec<Step> {
    let message = MessageId(format!("turn-{turn}"));
    let id = |name: &str| ToolCallId(format!("turn-{turn}-{name}"));
    let (skill, todo, read, search, web, fetch_missing, fetch, image, mcp, edit, write, test) = (
        id("skill"),
        id("todo"),
        id("read"),
        id("search"),
        id("web"),
        id("fetch-missing"),
        id("fetch"),
        id("image"),
        id("mcp"),
        id("edit"),
        id("write"),
        id("test"),
    );
    let target = prompt
        .segments
        .iter()
        .find_map(|segment| match segment {
            PromptSegment::Mention { label, .. } => Some(label.clone()),
            PromptSegment::Text(_) => None,
        })
        .unwrap_or_else(|| "the client".to_owned());

    let mut steps = vec![
        Step::Emit(ThreadEvent::UserMessage {
            id: MessageId(format!("turn-{turn}-user")),
            content: prompt.segments,
            attachments: prompt.attachments,
        }),
        usage(18_400),
        Step::Pause(400),
    ];
    steps.extend(stream(
        &message,
        true,
        &format!(
            "The request is about {target}. The HTTP timeout is probably hard-coded; \
             I should check how config is loaded and how reqwest applies timeouts \
             before changing the client constructor."
        ),
    ));
    steps.extend([
        tool_call(
            &skill,
            ToolKind::Skill,
            "Load skill",
            Some(ToolTarget::Skill {
                name: "rust-testing".to_owned(),
            }),
            ToolCallStatus::Running,
        ),
        Step::Pause(300),
        update(
            &skill,
            Some(ToolCallStatus::Completed),
            vec![ToolContent::Markdown(
                "Put integration tests in `tests/`, name each test after the behavior it \
                 checks, and run only the affected package with `cargo test -p`."
                    .to_owned(),
            )],
        ),
        tool_call(
            &todo,
            ToolKind::Todo,
            "Plan",
            None,
            ToolCallStatus::Completed,
        ),
        update(&todo, None, plan(0)),
    ]);
    let explore = id("explore");
    let query = id("query");
    steps.extend([
        tool_call(
            &explore,
            ToolKind::Agent,
            "Explore how config reaches the client",
            None,
            ToolCallStatus::Running,
        ),
        Step::Pause(900),
        update(
            &explore,
            Some(ToolCallStatus::Completed),
            vec![ToolContent::Markdown(
                "`Config` is loaded once in `main.rs` and passed to the server, but \
                 `Client::new` takes no arguments, so the client never sees it."
                    .to_owned(),
            )],
        ),
        usage(46_200),
    ]);
    steps.extend(stream(
        &message,
        false,
        "I'll start with the config module.",
    ));
    steps.extend([
        tool_call(
            &read,
            ToolKind::Read,
            "Read src/config.rs",
            file("src/config.rs", Some((1, 13))),
            ToolCallStatus::Running,
        ),
        Step::Pause(300),
        update(
            &read,
            Some(ToolCallStatus::Completed),
            vec![ToolContent::File {
                path: "src/config.rs".to_owned(),
                text: CONFIG_RS.to_owned(),
                first_line: 1,
            }],
        ),
        actions(&read, vec![Action::new("open", "Open file")]),
        tool_call(
            &search,
            ToolKind::Search,
            "Search for from_secs",
            Some(ToolTarget::Pattern {
                pattern: "from_secs".to_owned(),
                scope: Some("src, tests".to_owned()),
            }),
            ToolCallStatus::Running,
        ),
        Step::Pause(450),
        update(
            &search,
            Some(ToolCallStatus::Completed),
            vec![ToolContent::SearchResults {
                query: Some("from_secs".to_owned()),
                files: vec![
                    matched("src/client.rs", &[(11, "            .timeout(Duration::from_secs(30))")]),
                    matched(
                        "tests/client.rs",
                        &[
                            (4, "    let expected = Duration::from_secs(30);"),
                            (18, "    assert_eq!(client.timeout(), Duration::from_secs(30));"),
                        ],
                    ),
                ],
            }],
        ),
        update(&todo, None, plan(1)),
        tool_call(
            &web,
            ToolKind::WebSearch,
            "Search the web",
            Some(ToolTarget::Query("reqwest ClientBuilder timeout vs connect_timeout".to_owned())),
            ToolCallStatus::Running,
        ),
        Step::Pause(700),
        update(
            &web,
            Some(ToolCallStatus::Completed),
            vec![ToolContent::WebResults(vec![
                WebResult {
                    title: "ClientBuilder in reqwest - Rust".to_owned(),
                    url: "https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html".to_owned(),
                    snippet: "Enables a total request timeout. The timeout is applied from when the request starts connecting until the response body has finished.".to_owned(),
                },
                WebResult {
                    title: "Timeouts in reqwest: connect vs. read vs. total".to_owned(),
                    url: "https://example.com/blog/reqwest-timeouts".to_owned(),
                    snippet: "connect_timeout only bounds the TCP and TLS handshake; use timeout to bound the whole request, including a slow body.".to_owned(),
                },
                WebResult {
                    title: "Request hangs forever behind proxy · Issue #1132".to_owned(),
                    url: "https://github.com/seanmonstar/reqwest/issues/1132".to_owned(),
                    snippet: "Without a total timeout a stalled proxy keeps the connection open indefinitely.".to_owned(),
                },
            ])],
        ),
        tool_call(
            &fetch_missing,
            ToolKind::Fetch,
            "Fetch page",
            Some(ToolTarget::Url("https://docs.rs/reqwest/0.11/reqwest/struct.Timeout.html".to_owned())),
            ToolCallStatus::Running,
        ),
        Step::Pause(400),
        update(
            &fetch_missing,
            Some(ToolCallStatus::Failed),
            vec![ToolContent::Error("404 Not Found — the page does not exist.".to_owned())],
        ),
        tool_call(
            &fetch,
            ToolKind::Fetch,
            "Fetch page",
            Some(ToolTarget::Url("https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html".to_owned())),
            ToolCallStatus::Running,
        ),
        Step::Pause(500),
        update(
            &fetch,
            Some(ToolCallStatus::Completed),
            vec![ToolContent::WebPage {
                url: "https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html".to_owned(),
                title: "ClientBuilder in reqwest - Rust".to_owned(),
                excerpt: "### `timeout`\n\nEnables a total request timeout.\n\nThe timeout is applied from when the request starts connecting until the response body has finished. Default is no timeout.\n\n### `connect_timeout`\n\nSet a timeout for only the connect phase of a `Client`.\n\nDefault is `None`.\n\n### `read_timeout`\n\nEnables a read timeout. The timeout applies to each read operation, and resets after a successful read.".to_owned(),
            }],
        ),
        tool_call(
            &image,
            ToolKind::Read,
            "Read image",
            file("assets/app-icon/png/128.png", None),
            ToolCallStatus::Running,
        ),
        Step::Pause(250),
        update(
            &image,
            Some(ToolCallStatus::Completed),
            vec![ToolContent::Image {
                source: ImageSource::Bytes {
                    mime: "image/png".to_owned(),
                    data: SAMPLE_IMAGE.to_vec(),
                },
                caption: Some("128 × 128 PNG · 6.4 KB".to_owned()),
            }],
        ),
        tool_call(
            &mcp,
            ToolKind::Mcp,
            "github.search_issues",
            Some(ToolTarget::Mcp {
                server: "github".to_owned(),
                tool: "search_issues".to_owned(),
            }),
            ToolCallStatus::Running,
        ),
        Step::Pause(600),
        update(
            &mcp,
            Some(ToolCallStatus::Completed),
            vec![
                ToolContent::Json {
                    label: "Input".to_owned(),
                    text: r#"{"repo":"acme/demo-client","query":"timeout is:open"}"#.to_owned(),
                },
                ToolContent::Json {
                    label: "Output".to_owned(),
                    text: r#"[{"number":42,"title":"Requests hang forever behind a slow proxy","state":"open","labels":["bug","network"]},{"number":57,"title":"Make the HTTP timeout configurable","state":"open","labels":["enhancement"]}]"#.to_owned(),
                },
            ],
        ),
        tool_call(
            &query,
            ToolKind::Mcp,
            "postgres.query",
            Some(ToolTarget::Mcp {
                server: "postgres".to_owned(),
                tool: "query".to_owned(),
            }),
            ToolCallStatus::Running,
        ),
        Step::Pause(500),
        update(
            &query,
            Some(ToolCallStatus::Completed),
            vec![ToolContent::Custom {
                kind: TABLE.to_owned(),
                data: r#"{"columns":["service","timeout_s","p99_ms"],"rows":[["billing","30","2840"],["search","30","412"],["uploads","30","29700"]]}"#
                    .to_owned(),
            }],
        ),
        update(&todo, None, plan(2)),
        usage(161_800),
        Step::Pause(300),
        Step::Emit(ThreadEvent::Notice {
            id: NoticeId(format!("turn-{turn}-compaction")),
            kind: NoticeKind::Compaction,
            message: "summarized 38 earlier messages".to_owned(),
            actions: Vec::new(),
        }),
        usage(64_300),
    ]);
    steps.extend(stream(
        &message,
        true,
        "Config already has `request_timeout_secs`, but the client ignores it. \
         Passing the config into `Client::new` is the smallest change, and issue #57 asks for exactly that.",
    ));
    steps.extend([
        tool_call(
            &edit,
            ToolKind::Edit,
            "Edit src/client.rs",
            file("src/client.rs", None),
            ToolCallStatus::Pending,
        ),
        update(
            &edit,
            None,
            vec![ToolContent::Diff {
                path: "src/client.rs".to_owned(),
                old_text: Some(CLIENT_BEFORE.to_owned()),
                new_text: CLIENT_AFTER.to_owned(),
            }],
        ),
        Step::AwaitPermission {
            tool_call_id: edit.clone(),
            rejected: {
                let mut rejected = stream(
                    &message,
                    false,
                    "Understood — I left `src/client.rs` unchanged. \
                     Tell me if you'd prefer a different approach.",
                );
                rejected.push(Step::Emit(ThreadEvent::Stop {
                    reason: StopReason::EndTurn,
                }));
                rejected
            },
        },
        Step::Pause(300),
        set_status(&edit, ToolCallStatus::Completed),
        tool_call(
            &write,
            ToolKind::Write,
            "Write tests/timeout.rs",
            file("tests/timeout.rs", None),
            ToolCallStatus::Running,
        ),
        Step::Pause(300),
        update(
            &write,
            Some(ToolCallStatus::Completed),
            vec![ToolContent::Diff {
                path: "tests/timeout.rs".to_owned(),
                old_text: None,
                new_text: TIMEOUT_TEST.to_owned(),
            }],
        ),
        update(&todo, None, plan(3)),
        tool_call(
            &test,
            ToolKind::Execute,
            "Run tests",
            Some(ToolTarget::Command {
                command: "cargo test -p demo-client".to_owned(),
                cwd: None,
            }),
            ToolCallStatus::Running,
        ),
    ]);
    steps.extend(test_output_steps(&test));
    steps.push(actions(
        &test,
        vec![
            Action::new("rerun", "Run again"),
            Action::new("copy-command", "Copy command"),
        ],
    ));
    steps.push(update(&todo, None, plan(4)));
    steps.push(Step::Emit(ThreadEvent::Title {
        title: Some("Configurable request timeout".to_owned()),
    }));
    steps.push(usage(71_900));
    steps.extend(stream(
        &message,
        false,
        "Done. The client now reads its timeout from config:\n\n\
         - `Client::new` takes `&Config` and uses `request_timeout_secs`\n\
         - the 30s default stays in `default_timeout()`\n\
         - `tests/timeout.rs` covers the new path\n\n\
         3 tests pass, 1 is ignored. This should also close #57.",
    ));
    steps.push(Step::Emit(ThreadEvent::Stop {
        reason: StopReason::EndTurn,
    }));
    steps
}

const CONTEXT_WINDOW: u64 = 200_000;

fn usage(used_tokens: u64) -> Step {
    Step::Emit(ThreadEvent::Usage {
        used_tokens,
        limit_tokens: CONTEXT_WINDOW,
    })
}

/// A turn whose requests keep failing: a rate limit retried twice, then an
/// error that ends it.
fn failing_scenario(turn: usize, prompt: Prompt) -> Vec<Step> {
    let message = MessageId(format!("turn-{turn}"));
    let retry = NoticeId(format!("turn-{turn}-retry"));
    let mut steps = vec![
        Step::Emit(ThreadEvent::UserMessage {
            id: MessageId(format!("turn-{turn}-user")),
            content: prompt.segments,
            attachments: prompt.attachments,
        }),
        Step::Pause(400),
    ];
    steps.extend(stream(
        &message,
        true,
        "Let me look at the failing request first.",
    ));
    for attempt in 1..=2 {
        steps.push(Step::Emit(ThreadEvent::Notice {
            id: retry.clone(),
            kind: NoticeKind::Retrying,
            message: format!(
                "Rate limited by the provider — retrying in 2s (attempt {attempt} of 3)"
            ),
            actions: Vec::new(),
        }));
        steps.push(Step::Pause(1200));
    }
    steps.extend([
        Step::Emit(ThreadEvent::ClearNotice { id: retry }),
        Step::Emit(ThreadEvent::Notice {
            id: NoticeId(format!("turn-{turn}-error")),
            kind: NoticeKind::Error,
            message: "The provider returned 529 Overloaded three times. Try again in a moment."
                .to_owned(),
            actions: vec![
                Action::new(RETRY_FAST, "Retry with fast model"),
                Action::new("status", "Provider status"),
            ],
        }),
        Step::Emit(ThreadEvent::Stop {
            reason: StopReason::Error {
                message: "529 Overloaded".to_owned(),
            },
        }),
    ]);
    steps
}

/// Fills the panel's pickers, empty-thread suggestions and a footer slot
/// with samples.
pub fn configure(
    panel: &mut AgentPanel,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    cx: &mut Context<AgentPanel>,
) {
    let chip = move |icon: IconName, label: &'static str| {
        h_flex()
            .flex_shrink_0()
            .h(px(22.))
            .gap(px(4.))
            .px(px(6.))
            .rounded(px(5.))
            .border_1()
            .border_color(theme.border_variant)
            .text_size(ui_style.text.xsmall)
            .text_color(theme.text_muted)
            .child(Icon::new(icon).size(px(12.)).text_color(theme.icon_muted))
            .child(label)
    };
    panel.insert_slot(
        Slot::HeaderStart,
        SlotItem::new("location", 0, move |_, _, _| {
            Some(
                div()
                    .px(px(5.))
                    .rounded(px(4.))
                    .bg(theme.element_background)
                    .text_size(ui_style.text.xsmall)
                    .text_color(theme.text_subtle)
                    .child("local")
                    .into_any_element(),
            )
        }),
        cx,
    );
    panel.insert_slot(
        Slot::HeaderEnd,
        SlotItem::new("settings", 0, move |_, _, _| {
            Some(
                yttt_icon_button(
                    "demo-settings",
                    IconName::Settings,
                    YtttIconButtonKind::Toolbar,
                    theme,
                    ui_style,
                    |_, _, _| {},
                )
                .into_any_element(),
            )
        }),
        cx,
    );
    // Between the activity line and the plan, only while a turn runs.
    panel.insert_slot(
        Slot::StatusBar,
        SlotItem::new("sandbox", 150, move |context, _, _| {
            (context.status != ThreadStatus::Idle).then(|| {
                h_flex()
                    .gap(px(6.))
                    .px(px(2.))
                    .text_size(ui_style.text.xsmall)
                    .text_color(theme.text_subtle)
                    .child(
                        Icon::new(IconName::Info)
                            .size(px(12.))
                            .text_color(theme.icon_muted),
                    )
                    .child("Sandboxed · network limited to docs.rs and github.com")
                    .into_any_element()
            })
        }),
        cx,
    );
    panel.insert_slot(
        Slot::ComposerFooterStart,
        SlotItem::new("project", 0, move |_, _, _| {
            Some(chip(IconName::Folder, "demo-client").into_any_element())
        }),
        cx,
    );
    panel.set_custom_renderer(
        TABLE,
        Rc::new(move |data, _, _| render_table(data, theme, ui_style)),
        cx,
    );
    panel.set_message_actions(
        vec![
            MessageAction {
                id: "fork".to_owned(),
                label: "Fork".to_owned(),
                placement: MessagePlacement::User,
            },
            MessageAction {
                id: "helpful".to_owned(),
                label: "Helpful".to_owned(),
                placement: MessagePlacement::TurnEnd,
            },
        ],
        cx,
    );
    panel.set_plan_actions(vec![Action::new("edit-plan", "Edit plan")], cx);
    let option = |id: &str, label: &str, detail: &str| SelectorOption {
        id: id.to_owned(),
        label: label.to_owned(),
        detail: Some(detail.to_owned()),
    };
    panel.set_options(
        SelectorKind::Mode,
        vec![
            option(
                "agent",
                "Agent",
                "Edits files and runs commands, asking first",
            ),
            option("auto", "Auto-edit", "Applies edits without asking"),
            option("plan", "Plan", "Reads and plans, changes nothing"),
        ],
        0,
        cx,
    );
    panel.set_models(demo_models(false), cx);
    panel.set_favorite_models(vec!["large".to_owned()], cx);
    panel.set_suggestions(
        vec![
            "Explain how this project is structured".to_owned(),
            "Find and fix the failing tests".to_owned(),
            "Make the request timeout configurable".to_owned(),
        ],
        cx,
    );
}

/// Sample models from three providers; the local one runs only once
/// `local_up`.
pub fn demo_models(local_up: bool) -> Vec<ModelInfo> {
    let model = |id: &str,
                 name: &str,
                 (provider, icon): (&str, &SharedString),
                 context_window: u64,
                 (thinking, vision, tools): (bool, bool, bool),
                 status: ModelStatus| ModelInfo {
        id: id.to_owned(),
        name: name.to_owned(),
        provider: provider.to_owned(),
        context_window: Some(context_window),
        capabilities: ModelCapabilities {
            thinking,
            vision,
            tools,
        },
        status,
        icon: Some(icon.to_string()),
    };
    let (globe, building, drive) = (
        IconName::Globe.path(),
        IconName::Building2.path(),
        IconName::HardDrive.path(),
    );
    let cloud = ("Cloud", &globe);
    let partner = ("Partner", &building);
    let local = ("Local", &drive);
    vec![
        model(
            "large",
            "Large",
            cloud,
            200_000,
            (true, true, true),
            ModelStatus::Available,
        ),
        model(
            "fast",
            "Fast",
            cloud,
            200_000,
            (false, true, true),
            ModelStatus::Available,
        ),
        model(
            "reasoner",
            "Reasoner",
            cloud,
            128_000,
            (true, false, true),
            ModelStatus::Available,
        ),
        model(
            "partner-pro",
            "Partner Pro",
            partner,
            1_000_000,
            (true, true, true),
            ModelStatus::NeedsSetup,
        ),
        model(
            "local-8b",
            "Local 8B",
            local,
            32_000,
            (false, false, false),
            if local_up {
                ModelStatus::Available
            } else {
                ModelStatus::Unavailable("Server not running".to_owned())
            },
        ),
    ]
}

/// The custom content kind the demo draws as a table:
/// `{"columns": [..], "rows": [[..], ..]}`.
const TABLE: &str = "table";

fn render_table(data: &str, theme: WorkbenchTheme, ui_style: UiStyle) -> AnyElement {
    #[derive(serde::Deserialize)]
    struct Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    }
    let Ok(table) = serde_json::from_str::<Table>(data) else {
        return div().child("Unreadable table").into_any_element();
    };
    let row = |cells: Vec<String>, header: bool| {
        h_flex()
            .py(px(3.))
            .border_b_1()
            .border_color(theme.border_variant)
            .children(cells.into_iter().map(move |cell| {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(if header {
                        theme.text_subtle
                    } else {
                        theme.text
                    })
                    .child(cell)
            }))
    };
    v_flex()
        .text_size(ui_style.text.xsmall)
        .child(row(table.columns, true))
        .children(table.rows.into_iter().map(|cells| row(cells, false)))
        .into_any_element()
}

/// A small real PNG from the repository, for the image card.
const SAMPLE_IMAGE: &[u8] = include_bytes!("../../../assets/app-icon/png/128.png");

/// Sample files and commands for the demo's `@` and `/` popovers.
pub struct DemoCandidates;

const DEMO_FILES: [&str; 8] = [
    "src/main.rs",
    "src/config.rs",
    "src/client.rs",
    "src/client/retry.rs",
    "tests/client.rs",
    "Cargo.toml",
    "README.md",
    "docs/configuration.md",
];

const DEMO_COMMANDS: [(&str, &str); 4] = [
    ("clear", "Start a new thread"),
    ("compact", "Summarize the thread to free up context"),
    ("model", "Switch the model"),
    ("help", "Show what the agent can do"),
];

impl CandidateSource for DemoCandidates {
    fn candidates(&self, kind: TriggerKind, query: &str) -> Vec<Candidate> {
        match kind {
            TriggerKind::Mention => rank(query, &DEMO_FILES, |path| path)
                .into_iter()
                .map(|path| {
                    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
                    Candidate {
                        id: format!("file:///demo/{path}"),
                        label: name.to_owned(),
                        detail: (!dir.is_empty()).then(|| dir.to_owned()),
                    }
                })
                .collect(),
            TriggerKind::Command => rank(query, &DEMO_COMMANDS, |(name, _)| name)
                .into_iter()
                .map(|(name, description)| Candidate {
                    id: format!("command:{name}"),
                    label: (*name).to_owned(),
                    detail: Some((*description).to_owned()),
                })
                .collect(),
        }
    }
}

/// The prompt an unattended capture submits, with an image attached.
pub fn demo_prompt() -> Prompt {
    Prompt {
        segments: vec![
            PromptSegment::Text("Make the request timeout in ".to_owned()),
            PromptSegment::Mention {
                id: "file:///demo/src/client.rs".to_owned(),
                label: "client.rs".to_owned(),
            },
            PromptSegment::Text(" configurable".to_owned()),
        ],
        attachments: vec![Attachment {
            name: "icon.png".to_owned(),
            kind: AttachmentKind::Image {
                mime: "image/png".to_owned(),
                data: SAMPLE_IMAGE.to_vec(),
            },
        }],
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use gpui::{AppContext as _, TestAppContext, VisualTestContext};
    use yttt_ui::{style::UiStyle, theme::WorkbenchTheme};

    use super::*;
    use crate::model::{ThreadEntry, ThreadStatus};

    /// Components expect the component library's root at the top of the
    /// window, as the app's windows have it.
    fn setup(
        cx: &mut TestAppContext,
    ) -> (
        Entity<AgentPanel>,
        Entity<ScriptedDriver>,
        &mut VisualTestContext,
    ) {
        cx.update(yttt_ui::init);
        let slot = Rc::new(RefCell::new(None));
        let panel_slot = slot.clone();
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(|cx| {
                AgentPanel::new(
                    Rc::new(DemoCandidates),
                    WorkbenchTheme::zed_defaults(true),
                    UiStyle::default(),
                    window,
                    cx,
                )
            });
            *panel_slot.borrow_mut() = Some(panel.clone());
            yttt_ui::window::yttt_root(panel, window, cx)
        });
        let panel = slot.borrow_mut().take().expect("window built the panel");
        let driver = cx.update(|_, cx| {
            let panel = panel.clone();
            // Every delay rounds to zero at this speed, so parking plays the
            // whole turn. Advancing the test clock instead overflows the stack
            // once an icon has painted, in gpui's test platform.
            cx.new(|cx| ScriptedDriver::new(panel, 1000., false, cx))
        });
        (panel, driver, cx)
    }

    fn tool_status(
        panel: &Entity<AgentPanel>,
        cx: &mut VisualTestContext,
        id: &str,
    ) -> ToolCallStatus {
        panel.read_with(cx, |panel, cx| {
            panel
                .thread
                .read(cx)
                .entries()
                .iter()
                .find_map(|entry| match entry {
                    ThreadEntry::ToolCall(call) if call.id.0 == id => Some(call.status),
                    _ => None,
                })
                .expect("tool call exists")
        })
    }

    #[gpui::test]
    fn a_turn_waits_for_permission_then_finishes(cx: &mut TestAppContext) {
        let (panel, driver, cx) = setup(cx);
        driver.update(cx, |driver, cx| driver.start(demo_prompt(), None, cx));
        cx.run_until_parked();
        assert_eq!(
            panel.read_with(cx, |panel, cx| panel.status(cx)),
            ThreadStatus::AwaitingPermission
        );

        panel.update(cx, |_, cx| {
            cx.emit(AgentPanelEvent::PermissionChosen {
                tool_call_id: "turn-1-edit".into(),
                option_id: ALLOW_ONCE.into(),
            })
        });
        cx.run_until_parked();
        assert_eq!(
            panel.read_with(cx, |panel, cx| panel.status(cx)),
            ThreadStatus::Idle
        );
        assert_eq!(
            tool_status(&panel, cx, "turn-1-edit"),
            ToolCallStatus::Completed
        );
        assert_eq!(
            tool_status(&panel, cx, "turn-1-test"),
            ToolCallStatus::Completed
        );
    }

    #[gpui::test]
    fn rejecting_ends_the_turn_without_running_tests(cx: &mut TestAppContext) {
        let (panel, driver, cx) = setup(cx);
        driver.update(cx, |driver, cx| driver.start(demo_prompt(), None, cx));
        cx.run_until_parked();
        panel.update(cx, |_, cx| {
            cx.emit(AgentPanelEvent::PermissionChosen {
                tool_call_id: "turn-1-edit".into(),
                option_id: REJECT.into(),
            })
        });
        cx.run_until_parked();
        assert_eq!(
            panel.read_with(cx, |panel, cx| panel.status(cx)),
            ThreadStatus::Idle
        );
        assert_eq!(
            tool_status(&panel, cx, "turn-1-edit"),
            ToolCallStatus::Rejected
        );
        let ran_tests = panel.read_with(cx, |panel, cx| {
            panel.thread.read(cx).entries().iter().any(
                |entry| matches!(entry, ThreadEntry::ToolCall(call) if call.id.0 == "turn-1-test"),
            )
        });
        assert!(!ran_tests);
    }

    #[gpui::test]
    fn stopping_cancels_the_waiting_card(cx: &mut TestAppContext) {
        let (panel, driver, cx) = setup(cx);
        driver.update(cx, |driver, cx| driver.start(demo_prompt(), None, cx));
        cx.run_until_parked();
        panel.update(cx, |_, cx| cx.emit(AgentPanelEvent::Stop));
        cx.run_until_parked();
        assert_eq!(
            panel.read_with(cx, |panel, cx| panel.status(cx)),
            ThreadStatus::Idle
        );
        assert_eq!(
            tool_status(&panel, cx, "turn-1-edit"),
            ToolCallStatus::Canceled
        );
    }
}
