use super::UiTextKey;

pub(super) fn text(key: UiTextKey) -> &'static str {
    match key {
        UiTextKey::AppName => "yttt",
        UiTextKey::FileOpen => "Open",
        UiTextKey::FileOpenExternal => "Open with Default Application",
        UiTextKey::FileDownloadOpen => "Download and Open with Default Application…",
        UiTextKey::FileDownloadedCopy => "Opens a local copy. External changes are not uploaded.",
        UiTextKey::FilePreview => "Preview",
        UiTextKey::FilePreviewLoading => "Loading preview…",
        UiTextKey::FilePreviewUnavailable => "Preview unavailable",
        UiTextKey::FilePreviewFit => "Fit",
        UiTextKey::FilePreviewActualSize => "Actual Size",
        UiTextKey::FilePreviewSource => "Source",
        UiTextKey::FilePreviewReload => "Reload",
        UiTextKey::TrayOpen => "Open yttt",
        UiTextKey::TrayNewWindow => "New Window",
        UiTextKey::TrayOpenLogs => "Open Logs",
        UiTextKey::TrayStartHost => "Start Host",
        UiTextKey::TrayStopHost => "Stop Host If Idle",
        UiTextKey::TrayRestartHost => "Restart Host If Idle",
        UiTextKey::TrayQuitDesktop => "Quit Desktop",
        UiTextKey::TrayQuitAll => "Quit All",
        UiTextKey::TrayHost => "Host",
        UiTextKey::TrayConnecting => "Connecting",
        UiTextKey::TrayStopped => "Stopped",
        UiTextKey::TrayUnavailable => "Unavailable",
        UiTextKey::TrayRunning => "Running",
        UiTextKey::TrayDraining => "Draining",
        UiTextKey::TrayStopping => "Stopping",
        UiTextKey::TrayUnexpectedResponse => "Unexpected response",
        UiTextKey::TrayStatusSummary => {
            "{host}: {state} · {terminals} terminals · {clients} clients · {jobs} jobs"
        }
        UiTextKey::OnboardingTitle => "Welcome to yttt",
        UiTextKey::OnboardingSubtitle => {
            "Choose a language, terminal font, default layout, and coding agent, then optionally import themes from Zed."
        }
        UiTextKey::OnboardingLanguageHeading => "Choose your language",
        UiTextKey::OnboardingLanguageSubtitle => {
            "Preview your language here; it is saved when you finish onboarding. You can change it later in Settings."
        }
        UiTextKey::OnboardingFontHeading => "Choose a terminal font",
        UiTextKey::OnboardingFontSubtitle => {
            "The default terminal font is bundled Hack Nerd Font Mono. When no font is configured, a detected monospaced Nerd Font is selected automatically. You can change it here or later in Settings."
        }
        UiTextKey::OnboardingFontRecommendation => {
            "The terminal uses bundled Hack Nerd Font Mono by default. You can also install another monospaced Nerd Font, such as Maple Mono NF:"
        }
        UiTextKey::OnboardingFontDetectedRecommendation => {
            "Detected recommended monospaced Nerd Font (your existing selection is kept):"
        }
        UiTextKey::OnboardingAgentHeading => "Choose a coding agent",
        UiTextKey::OnboardingAgentSubtitle => "This agent will be used in the layout you selected.",
        UiTextKey::OnboardingLayoutHeading => "Choose a default layout",
        UiTextKey::OnboardingSplitLayoutTitle => "Split view",
        UiTextKey::OnboardingSplitLayoutDescription => "One tab with Agent and Shell side by side.",
        UiTextKey::OnboardingTabsLayoutTitle => "Separate tabs",
        UiTextKey::OnboardingTabsLayoutDescription => "Agent and Shell each open in their own tab.",
        UiTextKey::OnboardingAgentPane => "Agent",
        UiTextKey::OnboardingShellPane => "Shell",
        UiTextKey::OnboardingCommandPaletteHint => "Open the Command Palette anytime",
        UiTextKey::OnboardingNext => "Next",
        UiTextKey::OnboardingBack => "Back",
        UiTextKey::OnboardingContinue => "Use this layout",
        UiTextKey::OnboardingZedHeading => "Import themes from Zed",
        UiTextKey::OnboardingZedSubtitle => {
            "Compatible themes were detected in your installed Zed extensions. Review them before importing."
        }
        UiTextKey::OnboardingZedUiThemes => "UI themes",
        UiTextKey::OnboardingZedIconThemes => "Icon themes",
        UiTextKey::OnboardingZedNoThemes => "No compatible Zed themes were detected.",
        UiTextKey::OnboardingZedDetectionWarnings => "Some Zed extensions could not be inspected.",
        UiTextKey::OnboardingZedSkip => "Skip import",
        UiTextKey::OnboardingZedImport => "Import and finish",
        UiTextKey::EmptySubtitle => "Open a directory or choose a recent project.",
        UiTextKey::EmptySidebarNote => "Sidebar shows opened projects only.",
        UiTextKey::CreateProjectDirectoryError => "Failed to create project directory",
        UiTextKey::CreateProjectPickerError => "Failed to choose a project location",
        UiTextKey::OpenDirectory => "Open Directory",
        UiTextKey::OpenRecent => "Open Recent",
        UiTextKey::RestoreLastSession => "Restore Last Session",
        UiTextKey::CommandPalette => "Command Palette",
        UiTextKey::NewTab => "New Tab",
        UiTextKey::NoTerminalTabs => "No terminal tabs",
        UiTextKey::Projects => "Projects",
        UiTextKey::ProjectFiles => "Files",
        UiTextKey::AgentSessions => "Agent Sessions",
        UiTextKey::AgentSessionsRefresh => "Refresh Sessions",
        UiTextKey::AgentSessionsLoading => "Scanning agent sessions…",
        UiTextKey::AgentSessionsPartial => {
            "Some history could not be refreshed; available sessions remain searchable."
        }
        UiTextKey::AgentSessionsEmpty => "No sessions found for this project",
        UiTextKey::AgentSessionsSearchPlaceholder => "Search sessions…",
        UiTextKey::AgentSessionsNoMatches => "No matching sessions",
        UiTextKey::AgentSessionsRemoteUnavailable => {
            "Session scanning is not available for remote projects"
        }
        UiTextKey::AgentSessionsResumeHint => {
            "Double-click to resume; right-click to rename or delete history"
        }
        UiTextKey::AgentSessionsJustNow => "now",
        UiTextKey::AgentSessionsSessionId => "Session ID",
        UiTextKey::AgentSessionsModel => "Model",
        UiTextKey::AgentSessionsTranscript => "Transcript",
        UiTextKey::AgentSessionsUntitled => "Untitled session",
        UiTextKey::AgentSessionsRename => "Rename session",
        UiTextKey::AgentSessionsRenameDescription => {
            "Change the native Agent session name, not just its name in yttt."
        }
        UiTextKey::AgentSessionsDelete => "Delete session history",
        UiTextKey::AgentSessionsDeleteDescription => {
            "Permanently delete this Agent's saved conversation and its session-owned data. This cannot be undone. Project files are not deleted. Close this session in all terminals first."
        }
        UiTextKey::AgentSessionsGrokDeleteDescription => {
            "Permanently delete this Grok conversation and its session-owned data through Grok. If Grok is signed in, its synced cloud copy may also be deleted. This cannot be undone. Project files are not deleted. Close this session in all terminals first."
        }
        UiTextKey::AgentSessionsMutating => "Updating session history…",
        UiTextKey::AgentSessionsMutationFailed => "Session operation failed",
        UiTextKey::AgentSessionsSelectionChanged => {
            "The selected project changed. Open the session menu again."
        }
        UiTextKey::CommandCenterTitle => "Dispatch center",
        UiTextKey::CommandCenterHostLive => "Host live",
        UiTextKey::CommandCenterHostLiveReadOnly => "Host live · read-only",
        UiTextKey::CommandCenterHostDisconnectedRetained => "Host disconnected · retained history",
        UiTextKey::CommandCenterHostUnavailableRetained => "Host unavailable · retained history",
        UiTextKey::CommandCenterNoRecords => {
            "No yttt-managed agent records are available in this Host or workspace."
        }
        UiTextKey::CommandCenterProjectsCount => "projects {count}",
        UiTextKey::CommandCenterTasksCount => "tasks {count}",
        UiTextKey::CommandCenterQuestionsCount => "questions {count}",
        UiTextKey::CommandCenterExecutionsCount => "executions {count}",
        UiTextKey::CommandCenterTaskAccepted => "accepted",
        UiTextKey::CommandCenterTaskRetired => "retired",
        UiTextKey::CommandCenterPlanPending => "pending",
        UiTextKey::CommandCenterPlanWorking => "working",
        UiTextKey::CommandCenterPlanCompleted => "completed",
        UiTextKey::CommandCenterPlanBlocked => "blocked",
        UiTextKey::CommandCenterPlanAbandoned => "abandoned",
        UiTextKey::CommandCenterPlanUnknown => "unknown",
        UiTextKey::CommandCenterQuestions => "Needs attention",
        UiTextKey::CommandCenterRecordsCount => "records {count}",
        UiTextKey::CommandCenterNoQuestions => "No unresolved questions reported.",
        UiTextKey::CommandCenterDurationHours => "{value}h",
        UiTextKey::CommandCenterDurationMinutes => "{value}m",
        UiTextKey::CommandCenterDurationSeconds => "{value}s",
        UiTextKey::CommandCenterOptionPreview => "preview",
        UiTextKey::CommandCenterQueueAnswer => "Queue answer",
        UiTextKey::CommandCenterQueuedHint => "Queued is not provider acceptance.",
        UiTextKey::CommandCenterLinkedConversation => "Linked conversation",
        UiTextKey::CommandCenterOpenSession => "Open session",
        UiTextKey::CommandCenterOpenSessionHint => {
            "Open the owning project, tab, and terminal. Child agents open their hosting session."
        }
        UiTextKey::CommandCenterSessionNotOpen => {
            "The corresponding session is not open in this window."
        }
        UiTextKey::CommandCenterOpenSessionFailed => "Could not open the session",
        UiTextKey::CommandCenterNoSelection => "no selection",
        UiTextKey::CommandCenterSelectRecord => "Select a task, question, or execution.",
        UiTextKey::CommandCenterNoConversation => {
            "No conversation records were reported for this execution."
        }
        UiTextKey::CommandCenterYou => "you",
        UiTextKey::CommandCenterAgent => "agent",
        UiTextKey::CommandCenterMessageTruncated => "Provider truncated this retained message.",
        UiTextKey::CommandCenterPromptHint => "Safe prompt to this idle agent",
        UiTextKey::CommandCenterSend => "Send",
        UiTextKey::CommandCenterAcceptCompletedTask => "Accept completed task",
        UiTextKey::CommandCenterAcceptHint => {
            "Human acceptance remains separate from provider completion."
        }
        UiTextKey::CommandCenterAcceptRetainedReadOnly => {
            "Retained history is read-only; this task cannot be accepted here."
        }
        UiTextKey::CommandCenterAcceptDesktopReadOnly => {
            "This desktop is read-only; task acceptance is disabled."
        }
        UiTextKey::CommandCenterConfirmTerminateTerminal => "Confirm terminate terminal",
        UiTextKey::CommandCenterTerminateTerminal => "Terminate terminal (all work in pane)",
        UiTextKey::CommandCenterTerminateHint => {
            "This closes the terminal, not one derived execution."
        }
        UiTextKey::CommandCenterInputFailed => "Could not send terminal input",
        UiTextKey::CommandCenterTerminateDesktopReadOnly => {
            "This desktop is read-only; terminal termination is disabled."
        }
        UiTextKey::CommandCenterExecutionSummary => {
            "session summary (provider did not report execution records)"
        }
        UiTextKey::CommandCenterExecutionRecords => "Agent activity",
        UiTextKey::CommandCenterNoCurrentExecutions => "No current activity.",
        UiTextKey::CommandCenterActivityCount => "current {count}",
        UiTextKey::CommandCenterExecutionHistoryShow => "Show execution history ({count})",
        UiTextKey::CommandCenterExecutionHistoryHide => "Hide execution history ({count})",
        UiTextKey::CommandCenterExecutionUsage => "Execution usage · not per Todo",
        UiTextKey::CommandCenterModel => "Model",
        UiTextKey::CommandCenterTokenCount => "{value} tokens",
        UiTextKey::CommandCenterLastRequestTokens => "{value} tokens (last request)",
        UiTextKey::CommandCenterLastRequestTokensHint => {
            "Claude Code reports tokens for the subagent's final API request, not the whole run."
        }
        UiTextKey::CommandCenterTaskExecutionUsage => "Run {value} tok",
        UiTextKey::CommandCenterTaskLastRequestUsage => "Last req. {value} tok",
        UiTextKey::CommandCenterTaskUsageHint => {
            "Usage belongs to the associated execution, not this individual plan item. Items sharing an execution share these metrics; do not add them together."
        }
        UiTextKey::CommandCenterTaskUsageUnavailable => {
            "No token usage reported for the associated execution. Missing data is not zero."
        }
        UiTextKey::CommandCenterInputTokens => "Input tokens",
        UiTextKey::CommandCenterOutputTokens => "Output tokens",
        UiTextKey::CommandCenterCacheReadTokens => "Cache-read tokens",
        UiTextKey::CommandCenterCacheWriteTokens => "Cache-write tokens",
        UiTextKey::CommandCenterContextTokens => "Context tokens",
        UiTextKey::CommandCenterRequests => "Requests",
        UiTextKey::CommandCenterToolCalls => "Tool calls",
        UiTextKey::CommandCenterReportedDuration => "Reported elapsed",
        UiTextKey::CommandCenterCost => "Reported cost (USD)",
        UiTextKey::CommandCenterSelfEstimate => "self-est. {value}%",
        UiTextKey::CommandCenterSelfEstimateHint => {
            "Agent's own estimate, not verified completion. Missing estimates are not zero."
        }
        UiTextKey::CommandCenterTelemetryObserved => "Last observed {value} ago",
        UiTextKey::CommandCenterCurrentAction => "Current action",
        UiTextKey::CommandCenterParentExecution => "Parent execution",
        UiTextKey::CommandCenterSessionActions => "Session actions…",
        UiTextKey::CommandCenterHideSessionActions => "Hide session actions",
        UiTextKey::CommandCenterShowResolvedQuestions => "Show resolved history",
        UiTextKey::CommandCenterHideResolvedQuestions => "Hide resolved history",
        UiTextKey::CommandCenterSelectedQuestion => "Selected question",
        UiTextKey::CommandCenterPreviousQuestion => "Previous",
        UiTextKey::CommandCenterNextQuestion => "Next",
        UiTextKey::CommandCenterQuestionShortcutsHint => {
            "Alt+↑/↓ navigate · Ctrl/⌘+Enter submit · 1–9 choose after focusing a question"
        }
        UiTextKey::CommandCenterOnlineAgentsCount => "online root agents {count}",
        UiTextKey::CommandCenterActiveChildrenCount => "active children {count}",
        UiTextKey::CommandCenterAgentStarting => "starting",
        UiTextKey::CommandCenterAgentOnlineIdle => "online · idle",
        UiTextKey::CommandCenterAgentOnlineWorking => "online · working",
        UiTextKey::CommandCenterAgentOnlineWaiting => "online · waiting",
        UiTextKey::CommandCenterAgentOnlineCompleted => "online · turn finished",
        UiTextKey::CommandCenterAgentOnlineFailed => "online · turn failed",
        UiTextKey::CommandCenterAgentOnlineInterrupted => "online · turn interrupted",
        UiTextKey::CommandCenterAgentOnlineUnknown => "online · turn unknown",
        UiTextKey::CommandCenterAgentDisconnected => "observation lost",
        UiTextKey::CommandCenterAgentExited => "exited",
        UiTextKey::CommandCenterAgentRetained => "retained · not live",
        UiTextKey::CommandCenterPendingQuestionsCount => "to answer {count}",
        UiTextKey::CommandCenterTaskHistoryShow => "Show history records ({count})",
        UiTextKey::CommandCenterTaskHistoryHide => "Hide history records ({count})",
        UiTextKey::CommandCenterHistoryCompact => "History · {count}",
        UiTextKey::CommandCenterProjectUsageUnreported => {
            "Project tokens: no accumulable usage reported. Missing usage is not zero."
        }
        UiTextKey::CommandCenterProjectUsagePartial => {
            "Project tokens: at least {tokens} · partial coverage ({reported}/{observed} observed sources reported)"
        }
        UiTextKey::CommandCenterProjectUsageComplete => {
            "Project tokens: {tokens} · all {observed} known billing sources covered"
        }
        UiTextKey::CommandCenterProjectUsageScope => {
            "Accumulates reported historical and new usage, deduplicating stable sources and excluding parent/child overlap. Closing sessions, trimming history and restarting do not reset it. Coverage describes known billing sources, not complete sessions or project lifetime; offline values are retained observations."
        }
        UiTextKey::CommandCenterRoot => "root",
        UiTextKey::CommandCenterOptionalAnswerPlaceholder => "Optional additional answer",
        UiTextKey::CommandCenterPromptPlaceholder => "Send a follow-up to the selected idle agent",
        UiTextKey::CommandCenterQuestionNoLongerRetained => {
            "Question is no longer retained by this Host."
        }
        UiTextKey::CommandCenterAnswerAllQuestions => {
            "Answer every question before queueing the request."
        }
        UiTextKey::CommandCenterAnswerRequired => {
            "Answer every question with an option or additional text."
        }
        UiTextKey::CommandCenterSendingToHost => "Sending to Host…",
        UiTextKey::CommandCenterQueuedWaitingProvider => {
            "Queued by Host; waiting for provider confirmation."
        }
        UiTextKey::CommandCenterUnexpectedCommandResponse => {
            "Host returned an unexpected command response."
        }
        UiTextKey::CommandCenterHostRejectedCommand => "Host rejected command",
        UiTextKey::CommandCenterCommandChannelClosed => {
            "Host command channel closed; no command will be replayed."
        }
        UiTextKey::CommandCenterHostUnavailableReadOnly => {
            "Host is unavailable; retained history is read-only."
        }
        UiTextKey::CommandCenterLiveSnapshotRequired => {
            "Only a live Host snapshot can receive this command."
        }
        UiTextKey::CommandCenterDesktopReadOnlyCommands => {
            "This desktop is read-only; Host commands are disabled."
        }
        UiTextKey::CommandCenterQuestionNotInSelectedPane => {
            "Question does not belong to the selected live agent pane."
        }
        UiTextKey::CommandCenterQuestionNotAwaiting => {
            "Question is {state}, not awaiting an answer."
        }
        UiTextKey::CommandCenterProviderNoAnswerChannel => {
            "This provider did not expose an active answer channel."
        }
        UiTextKey::CommandCenterAnswerRejected => "Answer rejected",
        UiTextKey::CommandCenterPromptRequired => "Enter a prompt before sending.",
        UiTextKey::CommandCenterSessionNoLongerRetained => {
            "Selected session is no longer retained."
        }
        UiTextKey::CommandCenterPromptOtherWindow => {
            "This agent is live in another window; prompt it from its owning terminal."
        }
        UiTextKey::CommandCenterAwaitingHostInput => "Waiting for Host input acknowledgement…",
        UiTextKey::CommandCenterPromptAccepted => {
            "Host accepted terminal input; agent processing is observed separately."
        }
        UiTextKey::CommandCenterUnexpectedInputResponse => {
            "Host returned an unexpected input response."
        }
        UiTextKey::CommandCenterHostRejectedInput => "Host rejected terminal input",
        UiTextKey::CommandCenterInputAcknowledgementLost => {
            "Host acknowledgement was lost; input will not be replayed."
        }
        UiTextKey::CommandCenterTerminationConfirmed => {
            "Host confirmed terminal termination; all work in this pane is ending."
        }
        UiTextKey::CommandCenterHostRejectedTermination => "Host rejected terminal termination",
        UiTextKey::CommandCenterQuestionSubmitted => {
            "Queued with Host; waiting for provider confirmation."
        }
        UiTextKey::CommandCenterQuestionAnswered => {
            "Provider recorded this answer; the retained card is read-only."
        }
        UiTextKey::CommandCenterQuestionCancelled => {
            "Provider cancelled this question; the retained card is read-only."
        }
        UiTextKey::CommandCenterQuestionUnavailable => {
            "Provider answer channel is unavailable; retained history is read-only."
        }
        UiTextKey::CommandCenterQuestionLiveSnapshotRequired => {
            "Only a live Host snapshot can answer this retained question."
        }
        UiTextKey::CommandCenterDesktopReadOnlyAnswer => {
            "This desktop is read-only; answering is disabled."
        }
        UiTextKey::CommandCenterPromptRetainedReadOnly => {
            "Retained history is read-only; the original terminal is not live here."
        }
        UiTextKey::CommandCenterDesktopReadOnlyPrompt => {
            "This desktop is read-only; prompting is disabled."
        }
        UiTextKey::CommandCenterAgentNotRunning => "Agent process is not running.",
        UiTextKey::CommandCenterAgentBusy => {
            "Agent is busy; wait for an idle or completed turn before prompting."
        }
        UiTextKey::CommandCenterTurnIdle => "idle",
        UiTextKey::CommandCenterTurnWorking => "working",
        UiTextKey::CommandCenterTurnWaiting => "waiting",
        UiTextKey::CommandCenterTurnCompleted => "completed",
        UiTextKey::CommandCenterTurnFailed => "failed",
        UiTextKey::CommandCenterTurnInterrupted => "interrupted",
        UiTextKey::CommandCenterTurnUnknown => "unknown",
        UiTextKey::CommandCenterQuestionAwaitingAnswer => "awaiting answer",
        UiTextKey::CommandCenterQuestionQueued => "queued",
        UiTextKey::CommandCenterQuestionAnsweredStatus => "answered",
        UiTextKey::CommandCenterQuestionCancelledStatus => "cancelled",
        UiTextKey::CommandCenterQuestionUnavailableStatus => "unavailable",
        UiTextKey::ProjectFilesShow => "Show Project Panel",
        UiTextKey::ProjectFilesHide => "Hide Project Panel",
        UiTextKey::ProjectFilesRefresh => "Refresh",
        UiTextKey::ProjectFilesLoading => "Loading project files…",
        UiTextKey::ProjectFilesEmptyDirectory => "Empty directory",
        UiTextKey::ProjectFilesDirectoryError => "Unable to load directory",
        UiTextKey::ProjectFilesRetry => "Retry",
        UiTextKey::ProjectFilesNewFile => "New File",
        UiTextKey::ProjectFilesNewDirectory => "New Folder",
        UiTextKey::ProjectFilesCreateProjectLayout => "Create Project Layout",
        UiTextKey::ProjectFilesProjectLayoutReady => "Project layout is ready",
        UiTextKey::ProjectFilesRename => "Rename",
        UiTextKey::ProjectFilesDelete => "Delete",
        UiTextKey::ProjectFilesCopy => "Copy",
        UiTextKey::ProjectFilesCut => "Cut",
        UiTextKey::ProjectFilesPaste => "Paste",
        UiTextKey::ProjectFilesShowHidden => "Show Hidden Files",
        UiTextKey::ProjectFilesHideHidden => "Hide Hidden Files",
        UiTextKey::ProjectFilesEntryPlaceholder => "name or path; end with / for a folder",
        UiTextKey::ProjectFilesDeleteConfirmTitle => "Delete project entry?",
        UiTextKey::ProjectFilesDeleteConfirmMessage => {
            "The selected file or folder will be permanently deleted. This cannot be undone."
        }
        UiTextKey::ProjectFileUnsupportedBinary => "Binary files are not supported",
        UiTextKey::ProjectFileInvalidEncoding => "Only UTF-8 files are supported",
        UiTextKey::ProjectFileTooLarge => "File exceeds the 6 MiB editor limit",
        UiTextKey::ProjectFileOutsideProject => "File is outside the project",
        UiTextKey::FileSaveAction => "Save",
        UiTextKey::FileSaving => "Saving…",
        UiTextKey::FileSaved => "Saved",
        UiTextKey::FileSaveFailed => "Save failed",
        UiTextKey::FileChangedOnDisk => "File changed on disk",
        UiTextKey::FileOverwrite => "Overwrite",
        UiTextKey::FileReload => "Reload",
        UiTextKey::MarkdownShowRendered => "Show rendered Markdown",
        UiTextKey::MarkdownShowSource => "Edit Markdown source",
        UiTextKey::EditorHeldElsewhere => {
            "Another client is editing this file. Showing its unsaved changes read-only."
        }
        UiTextKey::EditorTakeOver => "Take Over Editing",
        UiTextKey::DocumentLeaseLostTitle => "Another client took over this file",
        UiTextKey::DocumentLeaseLostBody => {
            "Your changes to {path} that were not yet stored on the Host were saved to {copy}."
        }
        UiTextKey::DocumentLeaseLostKeep => "Keep Copy",
        UiTextKey::DocumentLeaseLostDiscard => "Discard Copy",
        UiTextKey::Tabs => "Tabs",
        UiTextKey::TabCloseCurrent => "Close Current",
        UiTextKey::TabCloseAll => "Close All",
        UiTextKey::TabCloseBefore => "Close Tabs to the Left",
        UiTextKey::TabCloseAfter => "Close Tabs to the Right",
        UiTextKey::TabCloseAllFiles => "Close All Files",
        UiTextKey::TabCloseAllTerminals => "Close All Terminals",
        UiTextKey::Lazy => "lazy",
        UiTextKey::Started => "started",
        UiTextKey::Active => "active",
        UiTextKey::NoResults => "No results",
        UiTextKey::TypeToFilter => "Type to filter",
        UiTextKey::PalettePlaceholderCommand => "Execute a command...",
        UiTextKey::PalettePlaceholderNewTabCommand => "Choose a command for the new tab...",
        UiTextKey::PalettePlaceholderFile => "Search files...",
        UiTextKey::PalettePlaceholderProject => "Switch project...",
        UiTextKey::PalettePlaceholderOpenedProject => "Switch opened project...",
        UiTextKey::PalettePlaceholderRecentProject => "Open recent project...",
        UiTextKey::PalettePlaceholderTab => "Switch tab...",
        UiTextKey::PalettePlaceholderPane => "Switch pane...",
        UiTextKey::PalettePlaceholderGitBranch => "Switch Git branch...",
        UiTextKey::FileFinderLoading => "Indexing project files…",
        UiTextKey::FileFinderLoadFailed => "Could not index project files",
        UiTextKey::FileFinderPreviewTitle => "File Preview",
        UiTextKey::FileFinderPreviewLoading => "Loading preview…",
        UiTextKey::FileFinderPreviewEmpty => "No preview available",
        UiTextKey::PaletteRun => "Run",
        UiTextKey::PaletteClose => "Close",
        UiTextKey::PaletteStatusOpen => "open",
        UiTextKey::PaletteStatusRecent => "recent",
        UiTextKey::PaletteStatusRemoteRecent => "SSH · recent",
        UiTextKey::PaletteStatusPaneSingular => "pane",
        UiTextKey::PaletteStatusPanePlural => "panes",
        UiTextKey::PaletteStatusActive => "active",
        UiTextKey::PaletteStatusLazy => "lazy",
        UiTextKey::PaletteStatusStarted => "started",
        UiTextKey::PaletteStatusIdle => "idle",
        UiTextKey::PaletteStatusRunning => "running",
        UiTextKey::PaletteStatusExited => "exited",
        UiTextKey::PaletteStatusAgent => "agent",
        UiTextKey::PaletteStatusAgentRunning => "agent running",
        UiTextKey::PaletteStatusAgentIdle => "agent idle",
        UiTextKey::PaletteStatusAgentWaiting => "agent waiting",
        UiTextKey::PaletteStatusAgentCompleted => "agent completed",
        UiTextKey::PaletteStatusAgentFailed => "agent failed",
        UiTextKey::PaletteStatusAgentInterrupted => "agent interrupted",
        UiTextKey::PaletteStatusAgentStale => "agent stale",
        UiTextKey::CommandDisabledVisibleProjectActions => "Use the visible project actions",
        UiTextKey::CommandDisabledOpenProjectFirst => "Open a project first",
        UiTextKey::CommandDisabledFocusProjectFileFirst => "Focus a project file first",
        UiTextKey::CommandDisabledOpenWorkItemFirst => "Open a terminal or file first",
        UiTextKey::CommandDisabledSwitchTerminalFirst => "Switch to a terminal tab first",
        UiTextKey::CommandUnavailable => "Command is unavailable",
        UiTextKey::CommandProjectCreateTitle => "Create Project",
        UiTextKey::CommandProjectCreateDescription => "Create and open a new project directory",
        UiTextKey::CommandProjectOpenTitle => "Open Project",
        UiTextKey::CommandProjectOpenDescription => "Choose a project directory",
        UiTextKey::CommandProjectOpenSshTitle => "Open SSH Project",
        UiTextKey::CommandProjectOpenSshDescription => "Connect to and open a remote project",
        UiTextKey::CommandProjectOpenRecentTitle => "Open Recent Project",
        UiTextKey::CommandProjectOpenRecentDescription => "Choose a recent project",
        UiTextKey::CommandProjectCloseTitle => "Close Project",
        UiTextKey::CommandProjectCloseDescription => "Close the selected project",
        UiTextKey::CommandProjectPaletteTitle => "Open Project Palette",
        UiTextKey::CommandProjectPaletteDescription => "Switch opened or recent projects",
        UiTextKey::CommandProjectOpenedPaletteTitle => "Switch Opened Project",
        UiTextKey::CommandProjectOpenedPaletteDescription => {
            "Choose from currently opened projects"
        }
        UiTextKey::CommandProjectPanelToggleTitle => "Toggle Project Panel",
        UiTextKey::CommandProjectPanelToggleDescription => "Show or hide the project side panel",
        UiTextKey::CommandProjectPanelRefreshTitle => "Refresh Project Files",
        UiTextKey::CommandProjectPanelRefreshDescription => "Refresh the project file tree",
        UiTextKey::CommandGitBranchSwitchTitle => "Switch Git Branch",
        UiTextKey::CommandGitBranchSwitchDescription => "Choose and check out a Git branch",
        UiTextKey::CommandGitDiffOpenTitle => "Show Git Changes",
        UiTextKey::CommandGitDiffOpenDescription => "Open the selected project's working tree diff",
        UiTextKey::CommandFileFindTitle => "Find Files",
        UiTextKey::CommandFileFindDescription => "Fuzzy-search files in opened projects",
        UiTextKey::CommandFileSaveTitle => "Save File",
        UiTextKey::CommandFileSaveDescription => "Save the active project file",
        UiTextKey::CommandTabNewTitle => "New Tab",
        UiTextKey::CommandTabNewDescription => "Create a shell tab in the selected project",
        UiTextKey::CommandTabCloseTitle => "Close Tab",
        UiTextKey::CommandTabCloseDescription => "Close the selected tab",
        UiTextKey::CommandTabRenameTitle => "Rename Tab",
        UiTextKey::CommandTabRenameDescription => "Rename the selected tab",
        UiTextKey::CommandTabNextTitle => "Next Tab",
        UiTextKey::CommandTabNextDescription => "Switch to the next project tab",
        UiTextKey::CommandTabPrevTitle => "Previous Tab",
        UiTextKey::CommandTabPrevDescription => "Switch to the previous project tab",
        UiTextKey::CommandTabPaletteTitle => "Open Tab Palette",
        UiTextKey::CommandTabPaletteDescription => "Switch tabs in the selected project",
        UiTextKey::CommandPaneSplitHorizontalTitle => "Split Pane Horizontally",
        UiTextKey::CommandPaneSplitHorizontalDescription => {
            "Split the focused pane into top and bottom panes"
        }
        UiTextKey::CommandPaneSplitVerticalTitle => "Split Pane Vertically",
        UiTextKey::CommandPaneSplitVerticalDescription => {
            "Split the focused pane into left and right panes"
        }
        UiTextKey::CommandPaneCloseTitle => "Close Pane",
        UiTextKey::CommandPaneCloseDescription => "Close the focused pane",
        UiTextKey::CommandPaneFocusLeftTitle => "Focus Pane Left",
        UiTextKey::CommandPaneFocusLeftDescription => "Move focus to the pane on the left",
        UiTextKey::CommandPaneFocusRightTitle => "Focus Pane Right",
        UiTextKey::CommandPaneFocusRightDescription => "Move focus to the pane on the right",
        UiTextKey::CommandPaneFocusUpTitle => "Focus Pane Up",
        UiTextKey::CommandPaneFocusUpDescription => "Move focus to the pane above",
        UiTextKey::CommandPaneFocusDownTitle => "Focus Pane Down",
        UiTextKey::CommandPaneFocusDownDescription => "Move focus to the pane below",
        UiTextKey::CommandPaneResizeLeftTitle => "Resize Pane Left",
        UiTextKey::CommandPaneResizeLeftDescription => "Resize the focused split toward the left",
        UiTextKey::CommandPaneResizeRightTitle => "Resize Pane Right",
        UiTextKey::CommandPaneResizeRightDescription => "Resize the focused split toward the right",
        UiTextKey::CommandPaneResizeUpTitle => "Resize Pane Up",
        UiTextKey::CommandPaneResizeUpDescription => "Resize the focused split upward",
        UiTextKey::CommandPaneResizeDownTitle => "Resize Pane Down",
        UiTextKey::CommandPaneResizeDownDescription => "Resize the focused split downward",
        UiTextKey::CommandPaneRenameTitle => "Rename Pane",
        UiTextKey::CommandPaneRenameDescription => "Rename the focused pane",
        UiTextKey::CommandPanePaletteTitle => "Open Pane Palette",
        UiTextKey::CommandPanePaletteDescription => "Focus panes in the selected tab",
        UiTextKey::CommandLayoutDefaultEditTitle => "Edit Default Layout",
        UiTextKey::CommandLayoutDefaultEditDescription => "Edit the global default layout TOML",
        UiTextKey::CommandLayoutDefaultResetTitle => "Reset Default Layout",
        UiTextKey::CommandLayoutDefaultResetDescription => {
            "Reset the global default layout to the built-in template"
        }
        UiTextKey::CommandLayoutDefaultReloadTitle => "Reload Default Layout",
        UiTextKey::CommandLayoutDefaultReloadDescription => {
            "Reload the global default layout from disk"
        }
        UiTextKey::CommandLayoutProjectEditTitle => "Edit Project Layout",
        UiTextKey::CommandLayoutProjectEditDescription => {
            "Edit the selected project's effective layout source"
        }
        UiTextKey::CommandLayoutSaveCurrentTitle => "Save Current Layout",
        UiTextKey::CommandLayoutSaveCurrentDescription => {
            "Save the current layout as a local override"
        }
        UiTextKey::CommandLayoutExportProjectConfigTitle => "Export Project Layout",
        UiTextKey::CommandLayoutExportProjectConfigDescription => {
            "Write the current layout to the project config"
        }
        UiTextKey::CommandLayoutResetLocalOverrideTitle => "Reset Personal Layout Override",
        UiTextKey::CommandLayoutResetLocalOverrideDescription => {
            "Remove the selected project's personal layout override"
        }
        UiTextKey::CommandLayoutOpenFileTitle => "Open Layout File",
        UiTextKey::CommandLayoutOpenFileDescription => {
            "Reveal the selected project's layout file path"
        }
        UiTextKey::LayoutEditorPlaceholder => "Edit layout TOML...",
        UiTextKey::LayoutEditorReadFailed => "Failed to read layout TOML",
        UiTextKey::LayoutEditorParseFailed => "Failed to parse layout TOML",
        UiTextKey::LayoutEditorValidationFailed => "Invalid layout TOML",
        UiTextKey::LayoutEditorPersonalInvalid => "Invalid personal layout",
        UiTextKey::LayoutEditorRequiresPatchMode => {
            "Personal layout editor requires mode = \"patch\""
        }
        UiTextKey::LayoutEditorRequiresReplaceMode => {
            "Personal layout editor requires mode = \"replace\""
        }
        UiTextKey::LayoutEditorSaveFailed => "Failed to save layout TOML",
        UiTextKey::BarsEditorTitle => "Edit bars TOML",
        UiTextKey::BarsEditorPlaceholder => "Edit Window Bar and Status Bar TOML...",
        UiTextKey::BarsEditorParseFailed => "Failed to parse bars TOML",
        UiTextKey::BarsEditorValidationFailed => "Invalid bars TOML",
        UiTextKey::BarsEditorSaveFailed => "Failed to save bars TOML",
        UiTextKey::BarsEditorComponents => "Components",
        UiTextKey::BarsEditorSearchComponents => "Search components...",
        UiTextKey::BarsEditorInsertInto => "Insert into",
        UiTextKey::BarsEditorPreview => "Preview",
        UiTextKey::BarsEditorPreviewUnavailable => "Fix the TOML to preview this layout.",
        UiTextKey::BarsEditorWindow => "Window",
        UiTextKey::BarsEditorStatus => "Status",
        UiTextKey::BarsEditorPresets => "Draft presets",
        UiTextKey::BarsEditorPresetRecommended => "Recommended",
        UiTextKey::BarsEditorPresetMinimal => "Minimal",
        UiTextKey::BarsEditorPresetDevelopment => "Development",
        UiTextKey::BarsEditorPresetAgent => "Agent",
        UiTextKey::BarsEditorRestoreDefaults => "Restore Defaults",
        UiTextKey::BarsEditorCatalogSample => "Example",
        UiTextKey::BarsEditorCatalogScope => "Scope",
        UiTextKey::BarsEditorPreviewStatusDisabled => "Status bar is disabled in this draft.",
        UiTextKey::BarsEditorPreviewUnavailableModules => "Unavailable components",
        UiTextKey::BarsEditorCategoryProject => "Project",
        UiTextKey::BarsEditorCategoryEditorVim => "Editor & Vim",
        UiTextKey::BarsEditorCategoryTerminal => "Terminal",
        UiTextKey::BarsEditorCategoryAgent => "Agent",
        UiTextKey::BarsEditorCategoryPerformance => "Performance",
        UiTextKey::BarsEditorCategoryLayoutActions => "Layout & actions",
        UiTextKey::BarsEditorReasonNoProject => "Open a project to show this component.",
        UiTextKey::BarsEditorReasonNoEditor => "Open an editor to show this component.",
        UiTextKey::BarsEditorReasonNoCodeEditor => {
            "This component is available only in a code editor."
        }
        UiTextKey::BarsEditorReasonNoSelection => "Select text to show this component.",
        UiTextKey::BarsEditorReasonNoTerminal => "Open a terminal to show this component.",
        UiTextKey::BarsEditorReasonTerminalNotExited => {
            "This component appears after the active terminal exits."
        }
        UiTextKey::BarsEditorReasonTerminalSizeUnavailable => {
            "The active terminal has not reported its size."
        }
        UiTextKey::BarsEditorReasonNoAgent => "The active terminal is not an Agent pane.",
        UiTextKey::BarsEditorReasonAgentNotWaiting => "The active Agent is not waiting for input.",
        UiTextKey::BarsEditorReasonAgentModelUnavailable => {
            "The active Agent has not reported a model."
        }
        UiTextKey::BarsEditorReasonNoActiveChildren => {
            "The active Agent has no active child tasks."
        }
        UiTextKey::BarsEditorReasonNoGit => "The selected project has no Git status.",
        UiTextKey::BarsEditorReasonGitClean => "The working tree has no changes.",
        UiTextKey::BarsEditorReasonNoSsh => "The selected project is not connected over SSH.",
        UiTextKey::BarsEditorReasonVimDisabled => "Enable Vim mode to show this component.",
        UiTextKey::BarsEditorReasonNoVimDetail => "The current Vim mode has no additional detail.",
        UiTextKey::BarsEditorReasonNoVimKeys => "The current Vim mode has no pending keys.",
        UiTextKey::BarsEditorReasonEditorClean => "The active editor has no unsaved changes.",
        UiTextKey::BarsEditorReasonNoDiagnostics => "The active editor has no diagnostics.",
        UiTextKey::BarsEditorReasonPerformanceUnavailable => {
            "Performance metrics are not available yet."
        }
        UiTextKey::BarsEditorReasonNoUpdate => "No update is available.",
        UiTextKey::BarsComponentProjectNameName => "Project name",
        UiTextKey::BarsComponentProjectNameDescription => "The selected project's display name.",
        UiTextKey::BarsComponentProjectPathName => "Project path",
        UiTextKey::BarsComponentProjectPathDescription => {
            "The selected project's local or remote path."
        }
        UiTextKey::BarsComponentGitBranchName => "Git branch",
        UiTextKey::BarsComponentGitBranchDescription => "Current branch for the selected project.",
        UiTextKey::BarsComponentGitChangesName => "Git changes",
        UiTextKey::BarsComponentGitChangesDescription => {
            "Working-tree change summary for the selected project."
        }
        UiTextKey::BarsComponentSshName => "SSH connection",
        UiTextKey::BarsComponentSshDescription => "Remote connection name and state.",
        UiTextKey::BarsComponentUpdateName => "Update status",
        UiTextKey::BarsComponentUpdateDescription => "Available application update status.",
        UiTextKey::BarsComponentActiveItemName => "Active item",
        UiTextKey::BarsComponentActiveItemDescription => {
            "Title and icon for the active file or terminal."
        }
        UiTextKey::BarsComponentSurfaceName => "Surface",
        UiTextKey::BarsComponentSurfaceDescription => {
            "Current work surface, such as editor or terminal."
        }
        UiTextKey::BarsComponentVimModeName => "Vim mode",
        UiTextKey::BarsComponentVimModeDescription => "Current Vim mode.",
        UiTextKey::BarsComponentVimDetailName => "Vim detail",
        UiTextKey::BarsComponentVimDetailDescription => {
            "Additional state for the current Vim mode."
        }
        UiTextKey::BarsComponentVimKeysName => "Vim keys",
        UiTextKey::BarsComponentVimKeysDescription => "Pending Vim key sequence.",
        UiTextKey::BarsComponentEditorLanguageName => "Editor language",
        UiTextKey::BarsComponentEditorLanguageDescription => "Language of the active code editor.",
        UiTextKey::BarsComponentEditorPositionName => "Editor position",
        UiTextKey::BarsComponentEditorPositionDescription => {
            "Cursor line and column in the active editor."
        }
        UiTextKey::BarsComponentEditorDirtyName => "Unsaved changes",
        UiTextKey::BarsComponentEditorDirtyDescription => {
            "Whether the active editor has unsaved changes."
        }
        UiTextKey::BarsComponentEditorDiagnosticsName => "Editor diagnostics",
        UiTextKey::BarsComponentEditorDiagnosticsDescription => {
            "Errors, warnings, and information from the active editor."
        }
        UiTextKey::BarsComponentEditorSelectionName => "Editor selection",
        UiTextKey::BarsComponentEditorSelectionDescription => "Selected character and line counts.",
        UiTextKey::BarsComponentEditorTabSizeName => "Tab size",
        UiTextKey::BarsComponentEditorTabSizeDescription => {
            "Indent width used by the active editor."
        }
        UiTextKey::BarsComponentEditorWrapName => "Line wrapping",
        UiTextKey::BarsComponentEditorWrapDescription => {
            "Whether soft wrapping is enabled in the active editor."
        }
        UiTextKey::BarsComponentTerminalTitleName => "Terminal title",
        UiTextKey::BarsComponentTerminalTitleDescription => "Title of the active terminal.",
        UiTextKey::BarsComponentTerminalStateName => "Terminal state",
        UiTextKey::BarsComponentTerminalStateDescription => {
            "Whether the active terminal is running."
        }
        UiTextKey::BarsComponentTerminalExitName => "Terminal exit",
        UiTextKey::BarsComponentTerminalExitDescription => {
            "Exit status after the active terminal stops."
        }
        UiTextKey::BarsComponentTerminalSizeName => "Terminal size",
        UiTextKey::BarsComponentTerminalSizeDescription => {
            "Columns and rows of the active terminal."
        }
        UiTextKey::BarsComponentAgentStateName => "Agent state",
        UiTextKey::BarsComponentAgentStateDescription => "Current state of the active Agent.",
        UiTextKey::BarsComponentAgentWaitingName => "Agent waiting",
        UiTextKey::BarsComponentAgentWaitingDescription => {
            "Shows when the active Agent awaits input."
        }
        UiTextKey::BarsComponentAgentModelName => "Agent model",
        UiTextKey::BarsComponentAgentModelDescription => "Model reported by the active Agent.",
        UiTextKey::BarsComponentAgentChildrenName => "Agent children",
        UiTextKey::BarsComponentAgentChildrenDescription => {
            "Active child-task count for the active Agent."
        }
        UiTextKey::BarsComponentAgentStateDurationName => "Agent state duration",
        UiTextKey::BarsComponentAgentStateDurationDescription => {
            "Time spent in the active Agent's current state."
        }
        UiTextKey::BarsComponentProjectsCountName => "Projects count",
        UiTextKey::BarsComponentProjectsCountDescription => "Number of open projects.",
        UiTextKey::BarsComponentTerminalsCountName => "Terminals count",
        UiTextKey::BarsComponentTerminalsCountDescription => "Number of open terminals.",
        UiTextKey::BarsComponentTabsCountName => "Tabs count",
        UiTextKey::BarsComponentTabsCountDescription => "Number of open tabs.",
        UiTextKey::BarsComponentEditorsCountName => "Editors count",
        UiTextKey::BarsComponentEditorsCountDescription => "Number of open editors.",
        UiTextKey::BarsComponentAppCpuName => "Application CPU",
        UiTextKey::BarsComponentAppCpuDescription => "CPU used by yttt.",
        UiTextKey::BarsComponentAppMemoryName => "Application memory",
        UiTextKey::BarsComponentAppMemoryDescription => "Memory used by yttt.",
        UiTextKey::BarsComponentSystemCpuName => "System CPU",
        UiTextKey::BarsComponentSystemCpuDescription => "Total system CPU usage.",
        UiTextKey::BarsComponentSystemMemoryName => "System memory",
        UiTextKey::BarsComponentSystemMemoryDescription => "Total system memory usage.",
        UiTextKey::BarsComponentCommandPaletteName => "Command palette",
        UiTextKey::BarsComponentCommandPaletteDescription => {
            "Button that opens the command palette."
        }
        UiTextKey::BarsComponentSettingsName => "Settings",
        UiTextKey::BarsComponentSettingsDescription => "Button that opens Settings.",
        UiTextKey::BarsComponentSpaceName => "Space",
        UiTextKey::BarsComponentSpaceDescription => {
            "Fixed blank space; change the count in the template."
        }
        UiTextKey::BarsComponentTextName => "Text",
        UiTextKey::BarsComponentTextDescription => {
            "Literal text; replace the example text in the template."
        }
        UiTextKey::BarsComponentIconName => "Icon",
        UiTextKey::BarsComponentIconDescription => {
            "Bundled icon; replace the example icon name in the template."
        }
        UiTextKey::BarsComponentSeparatorName => "Separator",
        UiTextKey::BarsComponentSeparatorDescription => {
            "Vertical separator between bar components."
        }
        UiTextKey::CommandPaletteOpenTitle => "Open Command Palette",
        UiTextKey::CommandPaletteOpenDescription => "Search and run commands",
        UiTextKey::CommandSettingsOpenTitle => "Open Settings",
        UiTextKey::SettingsWindowTitle => "Settings",
        UiTextKey::CommandSettingsOpenDescription => "Configure YTTT",
        UiTextKey::CommandSettingsKeybindingsTitle => "Open Keybindings File",
        UiTextKey::CommandSettingsKeybindingsDescription => {
            "Open or create the editable keybindings TOML"
        }
        UiTextKey::CommandSettingsNotificationsTitle => "Toggle Notifications",
        UiTextKey::CommandSettingsNotificationsDescription => {
            "Toggle system notifications for agent exits"
        }
        UiTextKey::CommandApplicationQuitTitle => "Quit yttt",
        UiTextKey::CommandApplicationQuitDescription => {
            "Stop all host resources and quit the application"
        }
        UiTextKey::GitBranchesLoading => "Loading Git branches…",
        UiTextKey::GitBranchSwitchFailed => "Could not switch Git branch",
        UiTextKey::GitBranchLocal => "Local branch",
        UiTextKey::GitBranchRemote => "Remote branch",
        UiTextKey::GitBranchAlreadyActive => "Already active",
        UiTextKey::GitDiffTitle => "Git Changes",
        UiTextKey::GitDiffLoading => "Loading working tree diff…",
        UiTextKey::GitDiffClean => "Working tree is clean",
        UiTextKey::GitDiffFile => "file",
        UiTextKey::GitDiffFiles => "files",
        UiTextKey::GitDiffFilesHeading => "Changed Files",
        UiTextKey::GitDiffWhitespace => "Ignore whitespace",
        UiTextKey::GitDiffUnified => "Unified",
        UiTextKey::GitDiffSplit => "Split",
        UiTextKey::GitDiffUnstaged => "Unstaged",
        UiTextKey::GitDiffStaged => "Staged",
        UiTextKey::GitDiffCloseHint => "close",
        UiTextKey::GitDiffStageHint => "staged / unstaged",
        UiTextKey::GitDiffSplitHint => "split / unified",
        UiTextKey::GitDiffNavigateHint => "navigate files",
        UiTextKey::GitDiffCopyHint => "copy",
        UiTextKey::GitDiffBinaryUnavailable => "Binary file — diff unavailable",
        UiTextKey::GitDiffSourceHead => "HEAD",
        UiTextKey::GitDiffSourceIndex => "Index",
        UiTextKey::GitDiffSourceWorkingTree => "Working Tree",
        UiTextKey::CloseProjectTitle => "Close project?",
        UiTextKey::CloseProjectBody => "Running terminal processes will be stopped.",
        UiTextKey::UnsavedChangesTitle => "Unsaved changes",
        UiTextKey::CloseWindowTitle => "Close YTTT?",
        UiTextKey::UnsavedFileSingular => "unsaved file",
        UiTextKey::UnsavedFilePlural => "unsaved files",
        UiTextKey::RunningProcessSingular => "running process",
        UiTextKey::RunningProcessPlural => "running processes",
        UiTextKey::SaveAllAndContinue => "Save All and Continue",
        UiTextKey::Discard => "Discard",
        UiTextKey::DiscardAndContinue => "Discard and Continue",
        UiTextKey::CloseSaveFailureGuidance => {
            "Fix the save error or discard the changes to continue."
        }
        UiTextKey::Cancel => "Cancel",
        UiTextKey::Retry => "Retry",
        UiTextKey::CloseProjectAction => "Close Project",
        UiTextKey::RenameTabTitle => "Rename tab",
        UiTextKey::RenameTabAction => "Rename",
        UiTextKey::RenameTabHint => "Enter to rename, Escape to cancel",
        UiTextKey::OpenNotificationTarget => "Open",
        UiTextKey::SettingsSearchPlaceholder => "Search settings...",
        UiTextKey::SettingsGroupGeneral => "General",
        UiTextKey::SettingsGroupGeneralDescription => "Application behavior and notifications",
        UiTextKey::SettingsGroupAppearance => "Appearance",
        UiTextKey::SettingsGroupAppearanceDescription => "UI font, UI and terminal themes",
        UiTextKey::SettingsGroupLanguages => "Languages",
        UiTextKey::SettingsGroupLanguagesDescription => {
            "Code language detection and language server defaults"
        }
        UiTextKey::SettingsGroupEditor => "Editor",
        UiTextKey::SettingsGroupEditorDescription => "Text editing and project file tree behavior",
        UiTextKey::SettingsGroupTerminal => "Terminal",
        UiTextKey::SettingsGroupTerminalDescription => "Shell, font, and terminal runtime defaults",
        UiTextKey::SettingsGroupAgent => "Agent",
        UiTextKey::SettingsGroupAgentDescription => "Primary agent and local session discovery",
        UiTextKey::SettingsGroupPermissions => "Permissions",
        UiTextKey::SettingsGroupPermissionsDescription => {
            "Desktop access for files, notifications, and developer workflows"
        }
        UiTextKey::SettingsGroupProjectLayout => "Project Layout",
        UiTextKey::SettingsGroupProjectLayoutDescription => "Project layout files and TOML editing",
        UiTextKey::SettingsGroupDefaultLayout => "Default Layout",
        UiTextKey::SettingsGroupDefaultLayoutDescription => {
            "Global layout inherited by projects without project config"
        }
        UiTextKey::SettingsGroupKeybindings => "Keybindings",
        UiTextKey::SettingsGroupKeybindingsDescription => {
            "Keyboard shortcuts and conflict diagnostics"
        }
        UiTextKey::SettingsNoSearchResults => "No settings found",
        UiTextKey::SettingsNoSearchResultsDescription => {
            "Try another setting name, description, or category."
        }
        UiTextKey::SettingsTargetDevice => "Local preferences",
        UiTextKey::SettingsTargetHost => "Environment",
        UiTextKey::SettingsTargetProject => "Project",
        UiTextKey::SettingsScopeDevice => "Local preferences",
        UiTextKey::SettingsScopeHost => "Environment · Shared by its clients",
        UiTextKey::SettingsScopeProject => "This project only",
        UiTextKey::SettingsLocalEnvironment => "Local",
        UiTextKey::SettingsRemoteEnvironment => "Remote",
        UiTextKey::SettingsDevelopmentEnvironment => "Development",
        UiTextKey::SettingsTestEnvironment => "Test",
        UiTextKey::SettingsEnvironmentDetails => "Configuration details",
        UiTextKey::SettingsLastKnownValue => "Last known value",
        UiTextKey::SettingsCustomizeProject => "Customize for this project",
        UiTextKey::SettingsCollapseOverride => "Collapse project settings",
        UiTextKey::SettingsSavePending => "Saving settings...",
        UiTextKey::SettingsApplyImmediate => "Applies immediately",
        UiTextKey::SettingsApplyNewSession => "Applies to new sessions",
        UiTextKey::SettingsApplyReopenFile => "Applies when files reopen",
        UiTextKey::SettingsSourceHost => "Using environment default",
        UiTextKey::SettingsSourceProject => "Project override",
        UiTextKey::SettingsReadOnlyDisconnected => {
            "Environment disconnected. Its settings are read-only; local preferences remain editable."
        }
        UiTextKey::SettingsReadOnlyProjectConfig => {
            "Project configuration is read-only in this environment. Use an environment with writable project configuration to customize this project."
        }
        UiTextKey::SettingsProjectTargetUnavailable => {
            "Open and select a project to edit project overrides."
        }
        UiTextKey::SettingsProjectLoading => "Loading project settings...",
        UiTextKey::SettingsProjectLoadFailed => "Failed to load project settings",
        UiTextKey::SettingsRemotePermissionsLocal => "Permissions are managed locally",
        UiTextKey::SettingsRemotePermissionsLocalDescription => {
            "This page manages desktop permissions on this device, not the remote Host."
        }
        UiTextKey::SettingsCopyDraft => "Copy draft",
        UiTextKey::WorkspaceStatusDisconnected => "Connection disconnected · not synchronized",
        UiTextKey::WorkspaceStatusRemoteIdentity => "Remote · {host}",
        UiTextKey::WorkspaceStatusLocalIdentity => "Local",
        UiTextKey::WorkspaceStatusRestoreFailed => " · Restore failed",
        UiTextKey::WorkspaceStatusRestoring => " · Restoring",
        UiTextKey::WorkspaceStatusSaving => " · Saving",
        UiTextKey::WorkspaceStatusHistory => "Workspace history…",
        UiTextKey::PersistenceDocumentSyncFailed => {
            "Could not store unsaved edits of {path} on the Host: {error}"
        }
        UiTextKey::PersistenceWorkspaceDeleted => "This workspace no longer exists on the Host",
        UiTextKey::PersistenceOpenFailed => "Could not open the workspace on the Host: {error}",
        UiTextKey::WorkspaceHistoryTitle => "Open workspace",
        UiTextKey::WorkspaceHistorySearch => "Search workspace names, paths or IDs…",
        UiTextKey::WorkspaceHistoryEmpty => "No closed workspaces",
        UiTextKey::WorkspaceHistoryLoading => "Loading workspace history…",
        UiTextKey::WorkspaceHistoryLoadFailed => "Could not load workspace history",
        UiTextKey::WorkspaceHistoryUnnamed => "Workspace",
        UiTextKey::WorkspaceHistoryNoProjects => "No saved projects",
        UiTextKey::WorkspaceHistoryMetadataUnavailable => "Workspace details unavailable",
        UiTextKey::WorkspaceHistorySaved => "Last saved",
        UiTextKey::WorkspaceHistoryUnknownDate => "Save time unknown",
        UiTextKey::WorkspaceHistoryToday => "Today",
        UiTextKey::WorkspaceHistoryYesterday => "Yesterday",
        UiTextKey::WorkspaceHistoryNavigation => "↑ ↓ Select    ↵ Open    Esc Close",
        UiTextKey::WorkspaceHistoryMore => "Workspace actions",
        UiTextKey::WorkspaceHistoryRemove => "Remove from history",
        UiTextKey::WorkspaceHistoryRemoveTitle => "Remove this workspace from history?",
        UiTextKey::WorkspaceHistoryRemoveWarning => {
            "This removes the saved layout and unsaved drafts. Project files and Agent history are not deleted."
        }
        UiTextKey::SettingsUseHostDefault => "Restore environment default",
        UiTextKey::SettingsSectionApplicationInteraction => "Application & interaction",
        UiTextKey::SettingsSectionStartupNotifications => "Startup & notifications",
        UiTextKey::SettingsSectionNewTabs => "New tabs",
        UiTextKey::SettingsSectionConnections => "Connections",
        UiTextKey::SettingsSectionUpdates => "Updates",
        UiTextKey::SettingsSectionWindow => "Window",
        UiTextKey::SettingsSectionInterface => "Interface",
        UiTextKey::SettingsSectionThemes => "Themes",
        UiTextKey::SettingsSectionBars => "Window & status bars",
        UiTextKey::SettingsSectionAdvanced => "Advanced",
        UiTextKey::SettingsSectionDetectionDefaults => "Detection & defaults",
        UiTextKey::SettingsSectionLanguageServices => "Language services",
        UiTextKey::SettingsSectionTypography => "Typography",
        UiTextKey::SettingsSectionEditingBehavior => "Editing behavior",
        UiTextKey::SettingsSectionSaving => "Saving",
        UiTextKey::SettingsSectionProjectPanels => "Project panels",
        UiTextKey::SettingsSectionShell => "Shell",
        UiTextKey::SettingsSectionEnvironment => "Environment",
        UiTextKey::SettingsSectionScrolling => "Scrolling",
        UiTextKey::SettingsSectionCursorMouse => "Cursor & mouse",
        UiTextKey::SettingsSectionCompatibility => "Compatibility & clipboard",
        UiTextKey::SettingsSectionAgentOverview => "Agent overview",
        UiTextKey::SettingsSectionProviders => "Session providers",
        UiTextKey::SettingsSectionCorePermissions => "Notifications and development access",
        UiTextKey::SettingsSectionOptionalPermissions => "Optional integrations",
        UiTextKey::SettingsSectionBackgroundHost => "Background Host",
        UiTextKey::SettingsLoginStartup => "Start Host at login",
        UiTextKey::SettingsLoginStartupDescription => {
            "Keep this profile's Host available after sign-in. Requires a persistent production profile and, on macOS, the packaged app. No network listener or secret is added."
        }
        UiTextKey::SettingsLoginStartupEnabled => "Enabled",
        UiTextKey::SettingsLoginStartupDisabled => "Disabled",
        UiTextKey::SettingsLoginStartupRequiresApproval => "Approval required",
        UiTextKey::SettingsLoginStartupUnavailable => "Unavailable",
        UiTextKey::SettingsLoginStartupChecking => "Checking...",
        UiTextKey::SettingsLoginStartupConfirmTitle => "Start the yttt Host when you sign in?",
        UiTextKey::SettingsLoginStartupConfirmDescription => {
            "This registers a per-user background startup item and keeps terminal, project, SSH, and Agent resources available without an open yttt window. It does not expose a network listener. You can disable it here or in system startup settings."
        }
        UiTextKey::SettingsLoginStartupConfirmAction => "Enable at login",
        UiTextKey::RemoteAccessTitle => "Remote access to this computer",
        UiTextKey::RemoteAccessDescription => {
            "Share this running Host through TLS and optional TCP forwarding. Connection information grants access to all workspaces in this profile; keep it private. Closing a window does not stop sharing."
        }
        UiTextKey::RemoteAccessListening => "Listening",
        UiTextKey::RemoteAccessStarting => "Starting…",
        UiTextKey::RemoteAccessStopping => "Stopping…",
        UiTextKey::RemoteAccessFailed => "Unable to start sharing",
        UiTextKey::RemoteAccessDisabled => "Remote access is off",
        UiTextKey::RemoteAccessUnavailable => "Waiting for the local Host",
        UiTextKey::RemoteAccessAddress => "Listen address",
        UiTextKey::RemoteAccessClients => "Connected devices",
        UiTextKey::RemoteAccessNoClients => "No remote devices connected",
        UiTextKey::RemoteAccessStreams => "streams",
        UiTextKey::RemoteAccessEnable => "Enable",
        UiTextKey::RemoteAccessDisable => "Disable",
        UiTextKey::RemoteAccessApply => "Apply address",
        UiTextKey::RemoteAccessCopy => "Copy connection information",
        UiTextKey::RemoteAccessReset => "Reset credentials",
        UiTextKey::RemoteAccessDisconnect => "Disconnect all remote clients",
        UiTextKey::RemoteAccessRefresh => "Refresh",
        UiTextKey::RemoteAccessConfirmTitle => "Change remote access?",
        UiTextKey::RemoteAccessConfirmDescription => {
            "This may disconnect remote clients. A non-loopback address exposes the port to your network. Forced control uses the last confirmed state; unpublished edits on the previous client may be lost. Continue only if you understand these consequences."
        }
        UiTextKey::RemoteAccessConfirm => "Confirm",
        UiTextKey::RemoteAccessCopied => {
            "Base64 connection code copied, including the address and access key. Keep it private."
        }
        UiTextKey::ConnectExistingHost => "Connect to existing yttt",
        UiTextKey::ExistingHostDescription => {
            "Paste a connection code from the other computer. Connect directly over TLS; no SSH or Server deployment required."
        }
        UiTextKey::ConnectionCodeLabel => "Connection code",
        UiTextKey::ConnectionCodePlaceholder => {
            "Paste Base64 connection code to fill in the address and credentials"
        }
        UiTextKey::ConnectionCodeSecurity => {
            "Contains the address, certificate and access key. Base64 is not encryption; share only with trusted devices."
        }
        UiTextKey::ConnectionCodeInvalid => {
            "Invalid connection code. Copy it again from the other computer's remote-access settings."
        }
        UiTextKey::ConnectionAddressLabel => "Connection address",
        UiTextKey::ConnectionAddressHint => {
            "Filled from the code. For another computer or port forwarding, replace this with a reachable host:port."
        }
        UiTextKey::ConnectionAddressInvalid => "Enter host:port or [IPv6]:port, not a URL.",
        UiTextKey::ConnectionRemember => "Remember credentials",
        UiTextKey::ConnectionRemembered => "Remember: system credential store",
        UiTextKey::RemoteConnections => "Remote connections",
        UiTextKey::RemoteAdd => "Add connection",
        UiTextKey::RemoteAddSsh => "SSH connection",
        UiTextKey::RemoteAddHost => "Network Host",
        UiTextKey::RemoteEdit => "Edit",
        UiTextKey::RemoteSaveConnect => "Save and connect",
        UiTextKey::RemoteEmpty => {
            "No remote connections yet. Add an SSH connection or paste a Host connection code."
        }
        UiTextKey::RemoteConnecting => "Connecting…",
        UiTextKey::RemoteConnectionName => "Connection name",
        UiTextKey::RemoteCredentialsRequired => "Credentials required",
        UiTextKey::RemoteRepairSshAgent => {
            "Unlock your SSH agent and load the saved connection's key, then retry. This authentication mode does not use a password."
        }
        UiTextKey::RemotePassphraseTemporary => {
            "The key passphrase is used only by this Client and is not saved."
        }
        UiTextKey::RemoteConnectTitle => "Connect to remote workspace",
        UiTextKey::RemoteConnectPreparing => "Preparing connection…",
        UiTextKey::RemoteConnectFailed => "Unable to connect",
        UiTextKey::RemoteConnectCancel => "Cancel connection",
        UiTextKey::RemoteConnectErrorDetails => "Technical details",
        UiTextKey::RemoteConnectVerifyingHost => "Verifying the Host certificate and environment…",
        UiTextKey::RemoteConnectCheckingServer => {
            "Checking the remote platform and Server installation…"
        }
        UiTextKey::RemoteConnectStartingHost => "Starting or attaching to the remote Host…",
        UiTextKey::RemoteConnectSshDisconnected => "SSH disconnected",
        UiTextKey::RemoteConnectSshConnecting => "Establishing SSH connection…",
        UiTextKey::RemoteConnectSshAuthenticating => "Authenticating SSH credentials…",
        UiTextKey::RemoteConnectSshConnected => "SSH connected",
        UiTextKey::RemoteConnectSshReconnecting => "Reconnecting to SSH…",
        UiTextKey::RemoteConnectSshRejected => "SSH host key rejected",
        UiTextKey::RemoteConnectContinuing => "Continuing connection…",
        UiTextKey::RemoteKeyAlgorithm => "Key algorithm",
        UiTextKey::RemoteManageLocally => {
            "Manage remote connections and credentials from the local yttt window."
        }
        UiTextKey::RemoteRecordMissing => "This saved connection no longer exists.",
        UiTextKey::RemoteInvalidPort => "The SSH port must be between 1 and 65535.",
        UiTextKey::RemoteInvalidRoot => "The remote root must be an absolute POSIX path.",
        UiTextKey::RemoteLaunchFailed => "Could not open the remote workspace",
        UiTextKey::RemoteWrongHostCode => "This connection code belongs to a different Host.",
        UiTextKey::RemoteCredentialStoreFailed => {
            "The OS credential store is unavailable. Nothing was saved in plaintext. Turn off Remember to save only the connection"
        }
        UiTextKey::RemoteRuntimeUnavailable => "The Host runtime is unavailable.",
        UiTextKey::RemoteDirectoryUnexpected => {
            "The Host returned an unexpected directory response."
        }
        UiTextKey::SettingsPermissionNotifications => "System notifications",
        UiTextKey::SettingsPermissionNotificationsDescription => {
            "Allow agent completion and attention alerts when yttt is not focused."
        }
        UiTextKey::SettingsPermissionFileSystem => "Protected file access",
        UiTextKey::SettingsPermissionFileSystemDescription => {
            "If protected project files cannot be accessed, check the Host or application that launches the terminal or agent. On macOS, opens Full Disk Access; ordinary project access does not require it."
        }
        UiTextKey::SettingsPermissionDeveloperTools => "Developer tools",
        UiTextKey::SettingsPermissionDeveloperToolsDescription => {
            "Manage system developer controls only when a development workflow requires them; not a prerequisite for ordinary terminal or agent use."
        }
        UiTextKey::SettingsPermissionAccessibility => "Accessibility",
        UiTextKey::SettingsPermissionAccessibilityDescription => {
            "Only needed by workflows that control other applications or send synthetic input."
        }
        UiTextKey::SettingsPermissionScreenCapture => "Screen capture",
        UiTextKey::SettingsPermissionScreenCaptureDescription => {
            "Only needed by workflows that capture a screen or application window."
        }
        UiTextKey::SettingsPermissionStatus => "Authorization status",
        UiTextKey::SettingsPermissionStatusDescription => {
            "Checks this local desktop application's authorization, not the access of a separate Host, terminal, or agent. Refreshes on return from system settings. System permissions are not profile preferences."
        }
        UiTextKey::SettingsPermissionRefresh => "Refresh",
        UiTextKey::SettingsPermissionChecking => "Checking...",
        UiTextKey::SettingsPermissionGranted => "Granted",
        UiTextKey::SettingsPermissionNotDetermined => "Not requested",
        UiTextKey::SettingsPermissionDenied => "Not granted",
        UiTextKey::SettingsPermissionUnavailable => "Status unavailable",
        UiTextKey::SettingsPermissionRequiresSystemSettings => "Check in system settings",
        UiTextKey::SettingsPermissionRequest => "Request",
        UiTextKey::SettingsPermissionRequesting => "Requesting...",
        UiTextKey::SettingsPermissionOpeningSettings => "Opening settings...",
        UiTextKey::SettingsPermissionManage => "Manage",
        UiTextKey::SettingsPermissionOpenSystemSettings => "Open system settings",
        UiTextKey::SettingsPermissionManagedBySystem => "Managed by desktop",
        UiTextKey::SettingsPermissionRequestedWhenNeeded => "Requested when needed",
        UiTextKey::SettingsPermissionNotRequired => "No approval required",
        UiTextKey::SettingsSectionCurrentLayout => "Current layout",
        UiTextKey::SettingsSectionLayoutActions => "Layout actions",
        UiTextKey::SettingsSectionDangerZone => "Danger zone",
        UiTextKey::SettingsSectionConfiguration => "Configuration",
        UiTextKey::SettingsSectionVim => "Vim",
        UiTextKey::SettingsSectionDiagnostics => "Diagnostics",
        UiTextKey::SettingsSectionBindings => "Bindings",
        UiTextKey::SettingsSectionKeymapProfile => "Keymap profile",
        UiTextKey::SettingsKeybindingProfileBase => "Base",
        UiTextKey::SettingsKeybindingProfileBaseDescription => {
            "Shortcuts used without Vim and inherited by Vim when they do not conflict."
        }
        UiTextKey::SettingsKeybindingProfileVim => "Vim",
        UiTextKey::SettingsKeybindingProfileVimDescription => {
            "The effective Vim keymap: inherited base shortcuts plus modal overrides."
        }
        UiTextKey::SettingsKeybindingOriginInherited => "Inherited",
        UiTextKey::SettingsKeybindingOriginBuiltin => "Built-in",
        UiTextKey::SettingsKeybindingOriginUser => "User",
        UiTextKey::SettingsKeybindingShadowed => "shadowed",
        UiTextKey::SettingsKeybindingPrefixConflict => "prefix",
        UiTextKey::SettingsAgentPrimary => "Primary agent",
        UiTextKey::SettingsAgentPrimaryDescription => {
            "The agent selected during onboarding and used for session discovery."
        }
        UiTextKey::SettingsAgentSessions => "Session list",
        UiTextKey::SettingsAgentSessionsDescription => {
            "Show the Agent Sessions tab. The primary agent is included by default."
        }
        UiTextKey::SettingsAgentSessionProviderDescription => {
            "Also scan and show this agent in the global session list."
        }
        UiTextKey::SettingsLanguage => "Language",
        UiTextKey::SettingsLanguageDescription => "Application display language.",
        UiTextKey::SettingsSelectLanguage => "Select language",
        UiTextKey::SettingsSystemNotifications => "System notifications",
        UiTextKey::SettingsSystemNotificationsDescription => {
            "Notify when agent terminal tasks complete or fail."
        }
        UiTextKey::SettingsUpdates => "Updates",
        UiTextKey::SettingsUpdatesDescription => {
            "View the installed version and check for available releases."
        }
        UiTextKey::SettingsAutoCheckUpdates => "Automatically check for updates",
        UiTextKey::SettingsAutoCheckUpdatesDescription => {
            "Check once per day after startup without blocking the application."
        }
        UiTextKey::SettingsCheckForUpdates => "Check for updates",
        UiTextKey::SettingsCheckingForUpdates => "Checking...",
        UiTextKey::SettingsUpdateAvailable => "Update available",
        UiTextKey::SettingsUpdateAvailableDescription => "A newer version is ready to download.",
        UiTextKey::SettingsUpToDate => "YTTT is up to date",
        UiTextKey::SettingsUpdateCheckFailed => "Unable to check for updates",
        UiTextKey::SettingsDownloadUpdate => "Download update",
        UiTextKey::SettingsRestoreLastSession => "Restore last session on startup",
        UiTextKey::SettingsRestoreLastSessionDescription => {
            "Restore saved projects, terminal and file tabs, and split layouts. Reconnect running processes, recreate shells, and resume saved Agent sessions without replaying other commands."
        }
        UiTextKey::PerformanceProjects => "Projects",
        UiTextKey::PerformanceTerminals => "Terminals",
        UiTextKey::PerformanceTabs => "Tabs",
        UiTextKey::PerformanceEditors => "Editors",
        UiTextKey::PerformanceCpu => "Application CPU",
        UiTextKey::PerformanceMemory => "Application memory",
        UiTextKey::PerformanceSystemCpu => "System CPU",
        UiTextKey::PerformanceSystemMemory => "System memory",
        UiTextKey::SettingsNewTabCommandPicker => "New tab command picker",
        UiTextKey::SettingsNewTabCommandPickerDescription => {
            "Show a command picker when clicking the new tab button."
        }
        UiTextKey::SettingsNewTabCommands => "New tab commands",
        UiTextKey::SettingsNewTabCommandsDescription => {
            "Commands available in the new tab picker. Each command runs in the selected project directory."
        }
        UiTextKey::SettingsNewTabCommandPlaceholder => "e.g. lazygit, nvim ., codex",
        UiTextKey::SettingsAddCommand => "Add command",
        UiTextKey::SettingsWindowEffect => "Window background effect",
        UiTextKey::SettingsWindowEffectDescription => {
            "Choose an opaque window, plain transparency, or frosted glass."
        }
        UiTextKey::SettingsWindowEffectNone => "None",
        UiTextKey::SettingsWindowEffectTransparent => "Transparent",
        UiTextKey::SettingsWindowEffectBlurred => "Frosted glass",
        UiTextKey::SettingsWindowOpacity => "Main window opacity",
        UiTextKey::SettingsWindowOpacityDescription => {
            "Shared opacity for the title bar, sidebars, editors, and terminals. 0.00 is fully transparent; 1.00 is opaque. Used by Transparent and Frosted glass."
        }
        UiTextKey::SettingsUiFontFamily => "UI font",
        UiTextKey::SettingsUiFontFamilyDescription => {
            "Font used across YTTT chrome, panels, and controls."
        }
        UiTextKey::SettingsUiFontSize => "UI font size",
        UiTextKey::SettingsUiFontSizeDescription => {
            "Base UI font size in pixels; controls scale proportionally."
        }
        UiTextKey::SettingsUiLineHeight => "UI line height",
        UiTextKey::SettingsUiLineHeightDescription => "Line-height multiplier for interface text.",
        UiTextKey::SettingsUiTheme => "UI theme",
        UiTextKey::SettingsUiThemeDescription => {
            "Theme used for YTTT chrome, panels, and controls."
        }
        UiTextKey::SettingsUiStyle => "UI style",
        UiTextKey::SettingsUiStyleDescription => {
            "Global shape, density, borders, hover treatment, and elevation."
        }
        UiTextKey::SettingsIconTheme => "Icon theme",
        UiTextKey::SettingsIconThemeDescription => {
            "File, folder, and editor icons from installed Zed-compatible themes."
        }
        UiTextKey::SettingsTerminalTheme => "Terminal theme",
        UiTextKey::SettingsTerminalThemeDescription => "Optional terminal colors override.",
        UiTextKey::SettingsSearchTheme => "Search theme...",
        UiTextKey::SettingsBarLeft => "Left",
        UiTextKey::SettingsBarCenter => "Center",
        UiTextKey::SettingsBarRight => "Right",
        UiTextKey::SettingsEditSettingsToml => "Edit settings TOML",
        UiTextKey::SettingsEditSettingsTomlDescription => {
            "Open the app settings file for advanced edits."
        }
        UiTextKey::SettingsEditBarsToml => "Bars",
        UiTextKey::SettingsEditBarsTomlDescription => {
            "Edit the Window Bar and Status Bar template in a dedicated editor."
        }
        UiTextKey::SettingsShowPath => "Show Path",
        UiTextKey::SettingsThemesDirectory => "Themes directory",
        UiTextKey::SettingsThemesDirectoryDescription => {
            "Open the folder containing user theme TOML files."
        }
        UiTextKey::SettingsImportZedThemes => "Import Zed themes",
        UiTextKey::SettingsImportZedThemesDescription => {
            "Detect and import compatible UI and icon themes from installed Zed extensions."
        }
        UiTextKey::SettingsImportZedThemesAction => "Import",
        UiTextKey::SettingsImportZedThemesComplete => "Zed themes imported",
        UiTextKey::SettingsImportZedThemesNone => "No compatible Zed themes were detected.",
        UiTextKey::SettingsImportZedThemesImported => "Already imported",
        UiTextKey::SettingsImportZedThemesConflictHint => "For already imported themes:",
        UiTextKey::SettingsImportZedThemesSkipExisting => "Skip existing",
        UiTextKey::SettingsImportZedThemesOverwriteExisting => "Overwrite existing",
        UiTextKey::SettingsLanguageDetection => "Language detection",
        UiTextKey::SettingsLanguageDetectionDescription => {
            "Detect code editor language from filename, extension, and first line."
        }
        UiTextKey::SettingsDefaultCodeLanguage => "Default code language",
        UiTextKey::SettingsDefaultCodeLanguageDescription => {
            "Fallback language used when automatic detection is disabled or unknown."
        }
        UiTextKey::SettingsSupportedLanguages => "Supported languages",
        UiTextKey::SettingsSupportedLanguagesDescription => {
            "Built-in languages available to the code editor."
        }
        UiTextKey::SettingsLanguageServer => "Language server",
        UiTextKey::SettingsLanguageServerDescription => {
            "Reserve an LSP launch point for future diagnostics and completion."
        }
        UiTextKey::SettingsLanguageServerCommand => "Language server command",
        UiTextKey::SettingsLanguageServerCommandDescription => {
            "Command reserved for the default language server integration."
        }
        UiTextKey::SettingsSearchCodeLanguage => "Search language...",
        UiTextKey::SettingsEditorFontFamily => "Font family",
        UiTextKey::SettingsEditorFontFamilyDescription => "Font used by project file editors.",
        UiTextKey::SettingsEditorFontSize => "Font size",
        UiTextKey::SettingsEditorFontSizeDescription => "Editor font size in pixels.",
        UiTextKey::SettingsEditorLineHeight => "Line height",
        UiTextKey::SettingsEditorLineHeightDescription => "Editor line height multiplier.",
        UiTextKey::SettingsEditorTabSize => "Tab size",
        UiTextKey::SettingsEditorTabSizeDescription => {
            "Number of spaces per tab. Reopen already-open files to apply this change."
        }
        UiTextKey::SettingsEditorSoftWrap => "Soft wrap",
        UiTextKey::SettingsEditorSoftWrapDescription => {
            "Wrap long editor lines to the available width."
        }
        UiTextKey::SettingsEditorLineNumbers => "Line numbers",
        UiTextKey::SettingsEditorLineNumbersDescription => "Show line numbers in project files.",
        UiTextKey::SettingsVimMode => "Vim mode",
        UiTextKey::SettingsVimModeDescription => {
            "Choose one Vim mode: Global unifies editors, terminals, project trees, settings, panes, tabs, and palettes; Editor only limits Vim to project editors; Disabled turns Vim off."
        }
        UiTextKey::SettingsVimQuickStart => "Vim quick start",
        UiTextKey::SettingsVimQuickStartDescription => {
            "Global mode: Ctrl-[ or Escape enters Normal; i returns to Insert or terminal input; Ctrl-W H/J/K/L focuses panes, work areas, and the project tree; tree j/k moves rows and h/l collapses, expands, or opens; gt/gT cycles tabs; Leader commands open files, palettes, and tabs."
        }
        UiTextKey::SettingsVimModeScope => "Modes and surfaces",
        UiTextKey::SettingsVimModeScopeDescription => {
            "Normal · Insert · Visual · Operator · Terminal · Settings · Palette · Project Tree"
        }
        UiTextKey::SettingsVimLeader => "Vim leader key",
        UiTextKey::SettingsVimLeaderDescription => {
            "Defaults to Space. Set top-level leader = \"...\" in keybindings.toml and use <leader> in multi-key bindings; changes reload automatically."
        }
        UiTextKey::SettingsEditorAutosave => "Autosave",
        UiTextKey::SettingsEditorAutosaveDescription => "Choose when edited files are saved.",
        UiTextKey::SettingsEditorAutosaveOff => "Off",
        UiTextKey::SettingsEditorAutosaveOnFocusChange => "On focus change",
        UiTextKey::SettingsEditorAutosaveAfterDelay => "After delay",
        UiTextKey::SettingsEditorAutosaveDelay => "Autosave delay",
        UiTextKey::SettingsEditorAutosaveDelayDescription => {
            "Delay in milliseconds used by delayed autosave."
        }
        UiTextKey::SettingsProjectPanelDefaultOpen => "Open file tree by default",
        UiTextKey::SettingsProjectPanelDefaultOpenDescription => {
            "Show the file tree when a project is opened for the first time."
        }
        UiTextKey::SettingsProjectPanelShowHidden => "Show hidden files",
        UiTextKey::SettingsProjectPanelShowHiddenDescription => {
            "Include hidden files and directories in project trees."
        }
        UiTextKey::SettingsProjectPanelWidth => "File tree width",
        UiTextKey::SettingsProjectPanelWidthDescription => {
            "Default width of the right project file tree."
        }
        UiTextKey::SettingsProjectSidebarWidth => "Project sidebar width",
        UiTextKey::SettingsProjectSidebarWidthDescription => {
            "Width of the left opened-project sidebar."
        }
        UiTextKey::SettingsDefaultShell => "Default shell",
        UiTextKey::SettingsDefaultShellDescription => {
            "Shell used by layout panes in shell execution mode and new terminal tabs."
        }
        UiTextKey::SettingsSelectShell => "Select shell",
        UiTextKey::SettingsCustomShell => "Custom shell",
        UiTextKey::SettingsCustomShellDescription => {
            "Add an executable path or command name to the saved shell list."
        }
        UiTextKey::SettingsCustomShellPlaceholder => "Shell path or command",
        UiTextKey::SettingsAddShell => "Add",
        UiTextKey::SettingsEnvironmentVariables => "Environment variables",
        UiTextKey::SettingsEnvironmentVariablesDescription => {
            "Injected into every newly started local or SSH shell and command. Setting an existing name replaces its value."
        }
        UiTextKey::SettingsEnvironmentNamePlaceholder => "Variable name",
        UiTextKey::SettingsEnvironmentValuePlaceholder => "Value",
        UiTextKey::SettingsSetEnvironmentVariable => "Add or update",
        UiTextKey::SettingsFontFamily => "Font family",
        UiTextKey::SettingsFontFamilyDescription => {
            "Terminal font family. System default uses bundled Hack Nerd Font Mono."
        }
        UiTextKey::SettingsSearchFont => "Search font...",
        UiTextKey::SettingsFontSize => "Font size",
        UiTextKey::SettingsFontSizeDescription => "Terminal font size in pixels.",
        UiTextKey::SettingsLineHeight => "Line height",
        UiTextKey::SettingsLineHeightDescription => "Terminal line height multiplier.",
        UiTextKey::SettingsPadding => "Padding",
        UiTextKey::SettingsPaddingDescription => "Terminal pane inner padding.",
        UiTextKey::SettingsScrollback => "Scrollback",
        UiTextKey::SettingsScrollbackDescription => "Number of terminal lines kept in memory.",
        UiTextKey::SettingsScrollbar => "Scrollbar",
        UiTextKey::SettingsScrollbarDescription => {
            "Show a thin scrollback indicator in terminal panes."
        }
        UiTextKey::SettingsTerminalCursorShape => "Cursor shape",
        UiTextKey::SettingsTerminalCursorShapeDescription => "Shape of the terminal cursor.",
        UiTextKey::SettingsTerminalCursorBlinking => "Blinking cursor",
        UiTextKey::SettingsTerminalCursorBlinkingDescription => {
            "Blink the terminal cursor while the terminal is focused."
        }
        UiTextKey::SettingsTerminalHideMouseWhenTyping => "Hide mouse while typing",
        UiTextKey::SettingsTerminalHideMouseWhenTypingDescription => {
            "Hide the pointer while keyboard input is sent to the terminal."
        }
        UiTextKey::SettingsTerminalCopyOnSelect => "Copy on select",
        UiTextKey::SettingsTerminalCopyOnSelectDescription => {
            "Copy completed terminal selections without an explicit Copy action."
        }
        UiTextKey::SettingsTerminalOsc52Policy => "OSC 52 clipboard access",
        UiTextKey::SettingsTerminalOsc52PolicyDescription => {
            "Control terminal application access to the system clipboard."
        }
        UiTextKey::SettingsTerminalKittyKeyboard => "Kitty keyboard protocol",
        UiTextKey::SettingsTerminalKittyKeyboardDescription => {
            "Allow terminal applications to negotiate the Kitty keyboard protocol."
        }
        UiTextKey::SettingsLayoutSource => "Layout source",
        UiTextKey::SettingsLayoutSourceDescription => "Current project layout source.",
        UiTextKey::SettingsOpenProjectFirst => "Open a project first",
        UiTextKey::SettingsSaveCurrentLayout => "Save current layout",
        UiTextKey::SettingsSaveCurrentLayoutDescription => {
            "Save current layout as an app-local override."
        }
        UiTextKey::SettingsExportProjectLayout => "Export project layout",
        UiTextKey::SettingsExportProjectLayoutDescription => {
            "Write current layout into the project config."
        }
        UiTextKey::SettingsEditLayoutToml => "Edit layout TOML",
        UiTextKey::SettingsEditLayoutTomlDescription => "Edit the selected project layout file.",
        UiTextKey::SettingsDefaultLayoutPath => "Default layout file",
        UiTextKey::SettingsDefaultLayoutPathDescription => {
            "Global default layout TOML used by projects without project config."
        }
        UiTextKey::SettingsEditDefaultLayout => "Edit default layout TOML",
        UiTextKey::SettingsEditDefaultLayoutDescription => "Edit the global default layout file.",
        UiTextKey::SettingsReloadDefaultLayout => "Reload default layout",
        UiTextKey::SettingsReloadDefaultLayoutDescription => {
            "Reload the global default layout from disk."
        }
        UiTextKey::SettingsResetDefaultLayout => "Reset default layout",
        UiTextKey::SettingsResetDefaultLayoutDescription => {
            "Replace the global default layout with the built-in template."
        }
        UiTextKey::SettingsEditKeybindingsToml => "Edit keybindings TOML",
        UiTextKey::SettingsEditKeybindingsTomlDescription => "Open the user keybindings file.",
        UiTextKey::KeybindingSettingsVimPreviousTitle => "Settings Vim: Previous Group",
        UiTextKey::KeybindingSettingsVimPreviousDescription => {
            "Select the previous visible settings group."
        }
        UiTextKey::KeybindingSettingsVimNextTitle => "Settings Vim: Next Group",
        UiTextKey::KeybindingSettingsVimNextDescription => {
            "Select the next visible settings group."
        }
        UiTextKey::KeybindingSettingsVimFirstTitle => "Settings Vim: First Group",
        UiTextKey::KeybindingSettingsVimFirstDescription => {
            "Select the first visible settings group."
        }
        UiTextKey::KeybindingSettingsVimLastTitle => "Settings Vim: Last Group",
        UiTextKey::KeybindingSettingsVimLastDescription => {
            "Select the last visible settings group."
        }
        UiTextKey::KeybindingVimNormalTitle => "Enter Vim Normal Mode",
        UiTextKey::KeybindingVimNormalDescription => {
            "Return the active yttt surface to Vim Normal mode."
        }
        UiTextKey::KeybindingVimInsertTitle => "Enter Vim Insert Mode",
        UiTextKey::KeybindingVimInsertDescription => {
            "Allow direct text input on the active yttt surface."
        }
        UiTextKey::KeybindingVimTerminalTitle => "Enter Vim Terminal Mode",
        UiTextKey::KeybindingVimTerminalDescription => {
            "Return the active terminal to direct process input."
        }
        UiTextKey::KeybindingProjectsVimPreviousTitle => "Projects Vim: Previous Project",
        UiTextKey::KeybindingProjectsVimPreviousDescription => {
            "Select the previous opened project."
        }
        UiTextKey::KeybindingProjectsVimNextTitle => "Projects Vim: Next Project",
        UiTextKey::KeybindingProjectsVimNextDescription => "Select the next opened project.",
        UiTextKey::KeybindingProjectsVimFirstTitle => "Projects Vim: First Project",
        UiTextKey::KeybindingProjectsVimFirstDescription => "Select the first opened project.",
        UiTextKey::KeybindingProjectsVimLastTitle => "Projects Vim: Last Project",
        UiTextKey::KeybindingProjectsVimLastDescription => "Select the last opened project.",
        UiTextKey::KeybindingProjectTreeVimUpTitle => "Project Tree Vim: Previous Entry",
        UiTextKey::KeybindingProjectTreeVimUpDescription => {
            "Move to the previous visible project-tree entry."
        }
        UiTextKey::KeybindingProjectTreeVimDownTitle => "Project Tree Vim: Next Entry",
        UiTextKey::KeybindingProjectTreeVimDownDescription => {
            "Move to the next visible project-tree entry."
        }
        UiTextKey::KeybindingProjectTreeVimLeftTitle => "Project Tree Vim: Collapse or Parent",
        UiTextKey::KeybindingProjectTreeVimLeftDescription => {
            "Collapse the current directory or select its parent."
        }
        UiTextKey::KeybindingProjectTreeVimRightTitle => "Project Tree Vim: Expand or Open",
        UiTextKey::KeybindingProjectTreeVimRightDescription => {
            "Expand a directory, select its first child, or open a file."
        }
        UiTextKey::KeybindingProjectTreeVimOpenTitle => "Project Tree Vim: Open",
        UiTextKey::KeybindingProjectTreeVimOpenDescription => {
            "Open a file or toggle the selected directory."
        }
        UiTextKey::KeybindingProjectTreeVimToggleTitle => "Project Tree Vim: Toggle Directory",
        UiTextKey::KeybindingProjectTreeVimToggleDescription => {
            "Expand or collapse the selected project-tree directory."
        }
        UiTextKey::KeybindingProjectTreeVimFirstTitle => "Project Tree Vim: First Entry",
        UiTextKey::KeybindingProjectTreeVimFirstDescription => {
            "Select the first visible project-tree entry."
        }
        UiTextKey::KeybindingProjectTreeVimLastTitle => "Project Tree Vim: Last Entry",
        UiTextKey::KeybindingProjectTreeVimLastDescription => {
            "Select the last visible project-tree entry."
        }
        UiTextKey::KeybindingTerminalViToggleTitle => "Toggle Terminal Vi Mode",
        UiTextKey::KeybindingTerminalViToggleDescription => "Enter or leave terminal Vi mode.",
        UiTextKey::KeybindingTerminalViLeaveTitle => "Exit Terminal Vi Mode",
        UiTextKey::KeybindingTerminalViLeaveDescription => {
            "Return the terminal to normal input mode."
        }
        UiTextKey::KeybindingTerminalViSelectionTitle => "Toggle Terminal Vi Selection",
        UiTextKey::KeybindingTerminalViSelectionDescription => {
            "Start or clear a terminal Vi selection."
        }
        UiTextKey::KeybindingTerminalViYankTitle => "Copy Terminal Vi Selection",
        UiTextKey::KeybindingTerminalViYankDescription => {
            "Copy and clear the terminal Vi selection."
        }
        UiTextKey::KeybindingTerminalViMoveLeftTitle => "Terminal Vi: Move Left",
        UiTextKey::KeybindingTerminalViMoveLeftDescription => "Move the terminal Vi cursor left.",
        UiTextKey::KeybindingTerminalViMoveDownTitle => "Terminal Vi: Move Down",
        UiTextKey::KeybindingTerminalViMoveDownDescription => "Move the terminal Vi cursor down.",
        UiTextKey::KeybindingTerminalViMoveUpTitle => "Terminal Vi: Move Up",
        UiTextKey::KeybindingTerminalViMoveUpDescription => "Move the terminal Vi cursor up.",
        UiTextKey::KeybindingTerminalViMoveRightTitle => "Terminal Vi: Move Right",
        UiTextKey::KeybindingTerminalViMoveRightDescription => "Move the terminal Vi cursor right.",
        UiTextKey::KeybindingTerminalViMoveFirstTitle => "Terminal Vi: First Column",
        UiTextKey::KeybindingTerminalViMoveFirstDescription => "Move to the first terminal column.",
        UiTextKey::KeybindingTerminalViMoveLastTitle => "Terminal Vi: Last Column",
        UiTextKey::KeybindingTerminalViMoveLastDescription => "Move to the last terminal column.",
        UiTextKey::KeybindingTerminalViMoveFirstOccupiedTitle => {
            "Terminal Vi: First Occupied Column"
        }
        UiTextKey::KeybindingTerminalViMoveFirstOccupiedDescription => {
            "Move to the first occupied terminal column."
        }
        UiTextKey::KeybindingTerminalViMoveHighTitle => "Terminal Vi: Viewport Top",
        UiTextKey::KeybindingTerminalViMoveHighDescription => {
            "Move to the top of the terminal viewport."
        }
        UiTextKey::KeybindingTerminalViMoveMiddleTitle => "Terminal Vi: Viewport Middle",
        UiTextKey::KeybindingTerminalViMoveMiddleDescription => {
            "Move to the middle of the terminal viewport."
        }
        UiTextKey::KeybindingTerminalViMoveLowTitle => "Terminal Vi: Viewport Bottom",
        UiTextKey::KeybindingTerminalViMoveLowDescription => {
            "Move to the bottom of the terminal viewport."
        }
        UiTextKey::KeybindingTerminalViMoveWordLeftTitle => "Terminal Vi: Previous Word",
        UiTextKey::KeybindingTerminalViMoveWordLeftDescription => {
            "Move to the previous terminal word."
        }
        UiTextKey::KeybindingTerminalViMoveWordRightTitle => "Terminal Vi: Next Word",
        UiTextKey::KeybindingTerminalViMoveWordRightDescription => {
            "Move to the next terminal word."
        }
        UiTextKey::KeybindingTerminalViMoveWordEndTitle => "Terminal Vi: Next Word End",
        UiTextKey::KeybindingTerminalViMoveWordEndDescription => {
            "Move to the end of the next terminal word."
        }
        UiTextKey::KeybindingTerminalViMoveBracketTitle => "Terminal Vi: Matching Bracket",
        UiTextKey::KeybindingTerminalViMoveBracketDescription => {
            "Move to the matching terminal bracket."
        }
        UiTextKey::KeybindingTerminalViMoveParagraphUpTitle => "Terminal Vi: Previous Paragraph",
        UiTextKey::KeybindingTerminalViMoveParagraphUpDescription => {
            "Move to the previous terminal paragraph."
        }
        UiTextKey::KeybindingTerminalViMoveParagraphDownTitle => "Terminal Vi: Next Paragraph",
        UiTextKey::KeybindingTerminalViMoveParagraphDownDescription => {
            "Move to the next terminal paragraph."
        }
        UiTextKey::SettingsKeybindingDiagnostics => "Keybinding diagnostics",
        UiTextKey::SettingsKeybindingDiagnosticsDescription => {
            "Show invalid commands and shortcut conflicts."
        }
        UiTextKey::SettingsNoKeybindingConflicts => "No keybinding conflicts",
        UiTextKey::SettingsUnbound => "Unbound",
        UiTextKey::SettingsConflict => "conflict",
        UiTextKey::SettingsKeybindingDialogTitle => "Edit keybinding",
        UiTextKey::SettingsKeybindingRecorderPrompt => "Press a key sequence",
        UiTextKey::SettingsKeybindingRecorderHint => {
            "Continue typing to build a sequence. Modifier keys alone are ignored."
        }
        UiTextKey::SettingsKeybindingCurrentBindings => "Current shortcuts",
        UiTextKey::SettingsKeybindingNoBindings => "No shortcuts assigned",
        UiTextKey::SettingsKeybindingReplace => "Replace shortcut",
        UiTextKey::SettingsKeybindingRecording => "Recording",
        UiTextKey::SettingsKeybindingFinishRecording => "Finish recording",
        UiTextKey::SettingsKeybindingRemove => "Remove",
        UiTextKey::SettingsAddKeybindingAlternative => "Add alternative",
        UiTextKey::SettingsClearKeybindings => "Clear",
        UiTextKey::SettingsConflictingKeybinding => "Conflicting keybinding",
        UiTextKey::SettingsInvalidCommandId => "Invalid command id",
        UiTextKey::SettingsSave => "Save",
        UiTextKey::SettingsExport => "Export",
        UiTextKey::SettingsOpen => "Open",
        UiTextKey::SettingsEdit => "Edit",
        UiTextKey::SettingsReset => "Reset",
        UiTextKey::SettingsDelete => "Delete",
        UiTextKey::StatusSystemNotificationsEnabled => "System notifications: enabled",
        UiTextKey::StatusSystemNotificationsDisabled => "System notifications: disabled",
        UiTextKey::StatusErrorContext => "Error",
        UiTextKey::StatusWarningContext => "Warning",
        UiTextKey::StatusKeybindingsFile => "Keybindings file",
        UiTextKey::StatusLayoutFile => "Layout file",
        UiTextKey::StatusSettingsFile => "Settings file",
        UiTextKey::StatusBarsFile => "Bars file",
        UiTextKey::StatusThemesDirectory => "Themes directory",
        UiTextKey::SshConnections => "SSH connections",
        UiTextKey::RemoteServices => "Remote services",
        UiTextKey::RemoteServicesDescription => {
            "Manage remote connections and access to this computer."
        }
        UiTextKey::SshConnectionsDescription => {
            "Manage SSH endpoints, credentials, and remote project roots."
        }
        UiTextKey::SshNoConnections => "No SSH connections configured.",
        UiTextKey::SshNewConnection => "New connection",
        UiTextKey::SshEditConnection => "Edit connection",
        UiTextKey::SshConnectionName => "Name",
        UiTextKey::SshCommand => "SSH command",
        UiTextKey::SshCommandPlaceholder => "ssh -p 22 -i ~/.ssh/id_ed25519 user@example.com",
        UiTextKey::SshCommandParse => "Apply command",
        UiTextKey::SshHost => "Host",
        UiTextKey::SshPort => "Port",
        UiTextKey::SshUser => "User",
        UiTextKey::SshRemoteRoot => "Remote root",
        UiTextKey::SshAuthentication => "Authentication",
        UiTextKey::SshIdentityFile => "Identity file",
        UiTextKey::SshKeyPassphrase => "Key passphrase",
        UiTextKey::SshPassword => "Password",
        UiTextKey::SshPasswordPromptTitle => "SSH password required",
        UiTextKey::SshPasswordPromptDescription => {
            "Enter the login password for this endpoint, then choose whether to save it."
        }
        UiTextKey::SshPasswordRequired => "Enter a password to continue.",
        UiTextKey::SshPasswordRejected => {
            "The server rejected the password. Check it and try again."
        }
        UiTextKey::SshPasswordUnavailable => "The saved password is unavailable. Enter it again.",
        UiTextKey::SshRememberPassword => "Save password in the system credential store",
        UiTextKey::SshAuthAuto => "Auto",
        UiTextKey::SshAuthAgent => "SSH agent",
        UiTextKey::SshAuthPassword => "Password",
        UiTextKey::SshAuthPrivateKey => "Private key",
        UiTextKey::SshConnect => "Connect",
        UiTextKey::SshDisconnect => "Disconnect",
        UiTextKey::SshDeleteConnection => "Delete connection",
        UiTextKey::SshHostKeyTitle => "Verify SSH host key",
        UiTextKey::SshHostKeyDescription => {
            "Confirm the server fingerprint before sending credentials."
        }
        UiTextKey::SshHostKeyChangedTitle => "SSH host key changed",
        UiTextKey::SshHostKeyChangedDescription => {
            "The saved key no longer matches this server. Verify the new fingerprint before replacing it."
        }
        UiTextKey::SshHostKeySavedFingerprint => "Saved fingerprint",
        UiTextKey::SshHostKeyReceivedFingerprint => "Received fingerprint",
        UiTextKey::SshHostKeyReject => "Reject",
        UiTextKey::SshHostKeyTrustOnce => "Trust once",
        UiTextKey::SshHostKeyTrustAndSave => "Trust and save",
        UiTextKey::SshHostKeyReplace => "Replace saved key",
        UiTextKey::SshConnectionSaved => "SSH connection saved",
        UiTextKey::SshConnectionDeleteInUse => {
            "Close this connection's remote projects before deleting it."
        }
        UiTextKey::SshOpenRemoteProject => "Open remote project",
        UiTextKey::SshProjectRecent => "Recent remote projects",
        UiTextKey::SshProjectPath => "Remote path",
        UiTextKey::SshProjectOpenCurrentFolder => "Open Current Folder",
        UiTextKey::SshProjectLoadingDirectory => "Loading remote directories…",
        UiTextKey::SshProjectEmptyDirectory => "This directory has no subdirectories.",
        UiTextKey::SshProjectNoMatchingDirectories => "No matching directories.",
        UiTextKey::SshProjectConnectionMissing => "The saved SSH connection no longer exists.",
        UiTextKey::SshProjectConnectionFailed => "Could not connect to the SSH host.",
        UiTextKey::SshRuntimeUnavailable => "SSH runtime is unavailable.",
        UiTextKey::SshDisconnected => "Disconnected",
        UiTextKey::SshConnecting => "Connecting…",
        UiTextKey::SshVerifyingHostKey => "Verifying host key…",
        UiTextKey::SshAuthenticating => "Authenticating…",
        UiTextKey::SshConnected => "Connected",
        UiTextKey::SshReconnecting => "Reconnecting…",
        UiTextKey::SshFailed => "Failed",
        UiTextKey::EditorVimMoveLeftTitle => "Editor Vim: Move Left",
        UiTextKey::EditorVimMoveLeftDescription => "Move the Vim cursor left.",
        UiTextKey::EditorVimMoveRightTitle => "Editor Vim: Move Right",
        UiTextKey::EditorVimMoveRightDescription => "Move the Vim cursor right.",
        UiTextKey::EditorVimMoveDownTitle => "Editor Vim: Move Down",
        UiTextKey::EditorVimMoveDownDescription => "Move the Vim cursor down by logical lines.",
        UiTextKey::EditorVimMoveUpTitle => "Editor Vim: Move Up",
        UiTextKey::EditorVimMoveUpDescription => "Move the Vim cursor up by logical lines.",
        UiTextKey::EditorVimMoveDisplayDownTitle => "Editor Vim: Move Display Line Down",
        UiTextKey::EditorVimMoveDisplayDownDescription => {
            "Move the Vim cursor down by display lines."
        }
        UiTextKey::EditorVimMoveDisplayUpTitle => "Editor Vim: Move Display Line Up",
        UiTextKey::EditorVimMoveDisplayUpDescription => "Move the Vim cursor up by display lines.",
        UiTextKey::EditorVimMoveNextWordStartTitle => "Editor Vim: Next Word",
        UiTextKey::EditorVimMoveNextWordStartDescription => "Move to the start of the next word.",
        UiTextKey::EditorVimMoveNextWordEndTitle => "Editor Vim: Next Word End",
        UiTextKey::EditorVimMoveNextWordEndDescription => "Move to the end of the next word.",
        UiTextKey::EditorVimMovePreviousWordStartTitle => "Editor Vim: Previous Word",
        UiTextKey::EditorVimMovePreviousWordStartDescription => {
            "Move to the start of the previous word."
        }
        UiTextKey::EditorVimMoveLineStartTitle => "Editor Vim: Line Start",
        UiTextKey::EditorVimMoveLineStartDescription => "Move to the start of the current line.",
        UiTextKey::EditorVimMoveFirstNonWhitespaceTitle => "Editor Vim: First Non-Whitespace",
        UiTextKey::EditorVimMoveFirstNonWhitespaceDescription => {
            "Move to the first non-whitespace character."
        }
        UiTextKey::EditorVimMoveLineEndTitle => "Editor Vim: Line End",
        UiTextKey::EditorVimMoveLineEndDescription => "Move to the end of the current line.",
        UiTextKey::EditorVimMoveDocumentStartTitle => "Editor Vim: Document Start",
        UiTextKey::EditorVimMoveDocumentStartDescription => "Move to the start of the document.",
        UiTextKey::EditorVimMoveDocumentEndTitle => "Editor Vim: Document End",
        UiTextKey::EditorVimMoveDocumentEndDescription => "Move to the end of the document.",
        UiTextKey::EditorVimCountZeroTitle => "Editor Vim: Zero",
        UiTextKey::EditorVimCountZeroDescription => "Enter zero in a count or move to line start.",
        UiTextKey::EditorVimCountOneTitle => "Editor Vim: Count 1",
        UiTextKey::EditorVimCountOneDescription => "Append 1 to the Vim count.",
        UiTextKey::EditorVimCountTwoTitle => "Editor Vim: Count 2",
        UiTextKey::EditorVimCountTwoDescription => "Append 2 to the Vim count.",
        UiTextKey::EditorVimCountThreeTitle => "Editor Vim: Count 3",
        UiTextKey::EditorVimCountThreeDescription => "Append 3 to the Vim count.",
        UiTextKey::EditorVimCountFourTitle => "Editor Vim: Count 4",
        UiTextKey::EditorVimCountFourDescription => "Append 4 to the Vim count.",
        UiTextKey::EditorVimCountFiveTitle => "Editor Vim: Count 5",
        UiTextKey::EditorVimCountFiveDescription => "Append 5 to the Vim count.",
        UiTextKey::EditorVimCountSixTitle => "Editor Vim: Count 6",
        UiTextKey::EditorVimCountSixDescription => "Append 6 to the Vim count.",
        UiTextKey::EditorVimCountSevenTitle => "Editor Vim: Count 7",
        UiTextKey::EditorVimCountSevenDescription => "Append 7 to the Vim count.",
        UiTextKey::EditorVimCountEightTitle => "Editor Vim: Count 8",
        UiTextKey::EditorVimCountEightDescription => "Append 8 to the Vim count.",
        UiTextKey::EditorVimCountNineTitle => "Editor Vim: Count 9",
        UiTextKey::EditorVimCountNineDescription => "Append 9 to the Vim count.",
        UiTextKey::EditorVimDeleteOperatorTitle => "Editor Vim: Delete Operator",
        UiTextKey::EditorVimDeleteOperatorDescription => "Begin a Vim delete operation.",
        UiTextKey::EditorVimChangeOperatorTitle => "Editor Vim: Change Operator",
        UiTextKey::EditorVimChangeOperatorDescription => "Begin a Vim change operation.",
        UiTextKey::EditorVimYankOperatorTitle => "Editor Vim: Yank Operator",
        UiTextKey::EditorVimYankOperatorDescription => "Begin a Vim yank operation.",
        UiTextKey::EditorVimInsertCurrentTitle => "Editor Vim: Insert",
        UiTextKey::EditorVimInsertCurrentDescription => "Enter insert mode at the cursor.",
        UiTextKey::EditorVimInsertAfterTitle => "Editor Vim: Append",
        UiTextKey::EditorVimInsertAfterDescription => "Enter insert mode after the cursor.",
        UiTextKey::EditorVimInsertFirstNonWhitespaceTitle => "Editor Vim: Insert at Indentation",
        UiTextKey::EditorVimInsertFirstNonWhitespaceDescription => {
            "Enter insert mode at the first non-whitespace character."
        }
        UiTextKey::EditorVimInsertLineEndTitle => "Editor Vim: Append at Line End",
        UiTextKey::EditorVimInsertLineEndDescription => "Enter insert mode at the end of the line.",
        UiTextKey::EditorVimInsertLineBelowTitle => "Editor Vim: Open Line Below",
        UiTextKey::EditorVimInsertLineBelowDescription => {
            "Open a new line below and enter insert mode."
        }
        UiTextKey::EditorVimInsertLineAboveTitle => "Editor Vim: Open Line Above",
        UiTextKey::EditorVimInsertLineAboveDescription => {
            "Open a new line above and enter insert mode."
        }
        UiTextKey::EditorVimToggleVisualTitle => "Editor Vim: Toggle Visual",
        UiTextKey::EditorVimToggleVisualDescription => "Enter or leave character-wise visual mode.",
        UiTextKey::EditorVimToggleVisualLineTitle => "Editor Vim: Toggle Visual Line",
        UiTextKey::EditorVimToggleVisualLineDescription => "Enter or leave line-wise visual mode.",
        UiTextKey::EditorVimDeleteCharactersTitle => "Editor Vim: Delete Characters",
        UiTextKey::EditorVimDeleteCharactersDescription => "Delete characters under the cursor.",
        UiTextKey::EditorVimSubstituteCharactersTitle => "Editor Vim: Substitute Characters",
        UiTextKey::EditorVimSubstituteCharactersDescription => {
            "Delete characters and enter insert mode."
        }
        UiTextKey::EditorVimReplaceCharactersTitle => "Editor Vim: Replace Characters",
        UiTextKey::EditorVimReplaceCharactersDescription => "Replace characters under the cursor.",
        UiTextKey::EditorVimPasteAfterTitle => "Editor Vim: Paste After",
        UiTextKey::EditorVimPasteAfterDescription => "Paste after the cursor.",
        UiTextKey::EditorVimPasteBeforeTitle => "Editor Vim: Paste Before",
        UiTextKey::EditorVimPasteBeforeDescription => "Paste before the cursor.",
        UiTextKey::EditorVimUndoTitle => "Editor Vim: Undo",
        UiTextKey::EditorVimUndoDescription => "Undo the previous edit.",
        UiTextKey::EditorVimRedoTitle => "Editor Vim: Redo",
        UiTextKey::EditorVimRedoDescription => "Redo the previous edit.",
        UiTextKey::EditorVimSearchForwardTitle => "Editor Vim: Search",
        UiTextKey::EditorVimSearchForwardDescription => "Open forward search.",
        UiTextKey::EditorVimSearchNextTitle => "Editor Vim: Next Search Match",
        UiTextKey::EditorVimSearchNextDescription => "Move to the next search match.",
        UiTextKey::EditorVimSearchPreviousTitle => "Editor Vim: Previous Search Match",
        UiTextKey::EditorVimSearchPreviousDescription => "Move to the previous search match.",
        UiTextKey::EditorVimEscapeTitle => "Editor Vim: Escape",
        UiTextKey::EditorVimEscapeDescription => "Return to Vim normal mode.",
        UiTextKey::SettingsLanguageSystem => "System",
        UiTextKey::SettingsCursorBlock => "Block",
        UiTextKey::SettingsCursorUnderline => "Underline",
        UiTextKey::SettingsCursorBeam => "Beam",
        UiTextKey::SettingsOptionDisabled => "Disabled",
        UiTextKey::SettingsOsc52CopyOnly => "Copy only",
        UiTextKey::SettingsOsc52ReadWrite => "Read and write",
        UiTextKey::SettingsVimGlobal => "Global",
        UiTextKey::SettingsVimEditorOnly => "Editor only",
        UiTextKey::DialogOk => "OK",
        UiTextKey::DesktopRestoreClientsFailed => "Cannot restore remote Clients",
        UiTextKey::DesktopQuitTitle => "Quit yttt?",
        UiTextKey::DesktopQuitOwnedHostBody => {
            "This Host is owned by the desktop. Quitting stops its network listener, disconnects remote clients, and terminates running terminals and Agent tasks. Confirmed workspaces and drafts remain saved."
        }
        UiTextKey::DesktopQuitIndependentHostBody => {
            "This independently running Host will keep its network listener, remote clients, terminals and Agent tasks. Only this desktop client will disconnect."
        }
        UiTextKey::DesktopStopHostAndQuit => "Stop Host and quit",
        UiTextKey::DesktopDisconnect => "Disconnect desktop",
        UiTextKey::DesktopPublishTimeout => {
            "Some work windows could not finish publishing. Keep them open and resolve their save errors before quitting."
        }
        UiTextKey::DesktopUnpublishedTitle => "Some changes could not be published",
        UiTextKey::DesktopUnpublishedBody => {
            "{error}\nUnpublished changes may be lost. Confirmed Host workspaces remain saved."
        }
        UiTextKey::DesktopKeepOpen => "Keep desktop open",
        UiTextKey::DesktopQuitUnpublished => "Quit without publishing",
        UiTextKey::DesktopStopFailed => "Host stop failed",
        UiTextKey::DesktopStopFailedBody => {
            "The desktop remains connected. Retry after checking Host status."
        }
        UiTextKey::WorkspaceRestoreLoading => "Restoring workspace…",
        UiTextKey::WorkspaceRestoreFailedBody => "The saved workspace state could not be restored.",
        UiTextKey::WorkspaceRestoreFailedTitle => "Workspace restore failed",
        UiTextKey::WorkspaceUnsavedTitle => "Changes have not been saved",
        UiTextKey::WorkspaceKeepOpen => "Keep window open",
        UiTextKey::WorkspaceCloseUnsaved => "Close without saving",
        UiTextKey::WorkspaceSettingsPending => {
            "Settings are still pending or failed to save; keep this window open to retry or copy the draft"
        }
        UiTextKey::WorkspaceFlushHostUnavailable => {
            "Remote workspace cannot be flushed because the Host is unavailable"
        }
        UiTextKey::PersistenceDraftDocumentLimit => {
            "Document draft exceeds the 6 MiB recovery limit"
        }
        UiTextKey::PersistenceRemoveHistory => "Cannot remove workspace history: {detail}",
        UiTextKey::PersistenceOpenInitialProject => {
            "Could not open the requested initial project: {error}"
        }
        UiTextKey::PersistenceStateRestoreFailed => {
            "Remote workspace state could not be restored: {error}"
        }
        UiTextKey::PersistenceCloseSavedWorkspace => "Cannot close saved workspace: {detail}",
        UiTextKey::PersistenceUpdateClientRestore => {
            "Cannot update remote Client restore state: {error}"
        }
        UiTextKey::SettingsIconThemeBuiltin => "Built-in",
        UiTextKey::SettingsTerminalThemeFollowUi => "Follow UI theme",
        UiTextKey::SettingsShellAuto => "Auto",
        UiTextKey::SettingsDeviceUnbound => {
            "Device preferences are not bound for this Client profile."
        }
        UiTextKey::SettingsProjectChanged => {
            "The selected project changed before this setting could be saved."
        }
        UiTextKey::SettingsTabSizeInvalid => "Tab size must be between 1 and 16.",
        UiTextKey::SettingsLanguageEmpty => "Default language cannot be empty.",
        UiTextKey::SettingsHostUnavailable => "Host is unavailable",
        UiTextKey::SettingsUnexpectedDeviceResponse => {
            "Host returned an unexpected device settings response"
        }
        UiTextKey::SettingsDraftSelectOwner => {
            "Select the project that owns this draft before retrying it."
        }
        UiTextKey::SettingsDraftProjectControl => {
            "Host control is required to retry this project settings draft."
        }
        UiTextKey::SettingsDraftStale => {
            "Settings changed since this draft was created. The draft is retained; review the current values before discarding it and applying your changes again."
        }
        UiTextKey::SettingsDraftControl => "Host control is required to retry this settings draft.",
        UiTextKey::OnboardingConfirmImport => {
            "confirm whether to import Zed themes before completing onboarding"
        }
        UiTextKey::OnboardingNeedsControl => {
            "Host control is required to initialize the default layout and Agent."
        }
        UiTextKey::OnboardingWaitSettings => {
            "Wait for the current settings save before completing onboarding."
        }
        UiTextKey::OnboardingControlLost => {
            "Host control was lost before onboarding could be saved."
        }
        UiTextKey::SharedEditingRequired => "Shared editing control is required",
        UiTextKey::ConfigurationTargetUnavailable => {
            "The selected configuration target is unavailable."
        }
        UiTextKey::ConnectionsLocalWindow => "Open connections from your local yttt window.",
        UiTextKey::ProjectOpenRequiresControl => {
            "Shared editing control is required to open a project"
        }
        UiTextKey::PreviousSshProjects => {
            "Previous SSH projects remain in Recent Projects; open them in an isolated remote workspace."
        }
        UiTextKey::PersistenceWorkspaceAlreadyOpen => "Workspace is open in this Client",
        UiTextKey::SettingsDraftCopyFailed => "Settings draft could not be copied: {error}",
        UiTextKey::SettingsSaveFailed => "Settings were not saved: {error}",
        UiTextKey::SettingsProjectSaveFailed => "Project settings were not saved: {error}",
        UiTextKey::SettingsRefreshFailed => "Settings could not be refreshed: {error}",
        UiTextKey::HostProjectRegistrationFailed => "Host project registration failed: {error}",
        UiTextKey::SettingsUnknownGroup => "Unknown settings group: {group_id}",
        UiTextKey::LayoutSourceGlobal => "global default",
        UiTextKey::LayoutSourceGlobalPatched => "global default + personal patch",
        UiTextKey::LayoutSourceProject => "project config",
        UiTextKey::LayoutSourceProjectPatched => "project config + personal patch",
        UiTextKey::LayoutSourcePersonal => "personal replacement",
        UiTextKey::LayoutSourceLabel => "Layout source: {source}",
        UiTextKey::SettingsWarningFile => "Settings {path}: {error}",
        UiTextKey::SettingsWarningBars => {
            "Bars {field} contains invalid module {value}; ignoring it"
        }
        UiTextKey::SettingsWarningInvalidValue => {
            "Settings {group}.{field} is invalid; using default"
        }
        UiTextKey::ThemeWarningFile => "Theme {path}: {error}",
        UiTextKey::ThemeWarningColor => "Theme {theme} has invalid color {field}; using fallback",
        UiTextKey::DevicePreferencesUnbound => "Device preferences are not bound.",
        UiTextKey::PersistenceDraftConflict => {
            "Recovered draft for {path} conflicts with the current remote file; review before saving"
        }
        UiTextKey::SettingsInspectKeybindings => {
            "Failed to inspect keybindings directory {path}: {error}"
        }
        UiTextKey::SettingsResolveKeybindings => {
            "Failed to resolve keybindings directory {path}: {error}"
        }
        UiTextKey::SettingsWatchKeybindings => "Failed to watch keybindings at {path}: {error}",
        UiTextKey::HostTerminalCloseRejected => "Host rejected terminal close: {details}",
        UiTextKey::RevealPathFailed => "Failed to reveal {path}: {error}",
        UiTextKey::KeybindingUiCommandCenterTitle => "Toggle Dispatch Center",
        UiTextKey::KeybindingUiCommandCenterDescription => {
            "Show or hide the single-screen agent dispatch center."
        }
        UiTextKey::KeybindingUiCommandCenterQuestionPreviousTitle => {
            "Previous Dispatch Center Question"
        }
        UiTextKey::KeybindingUiCommandCenterQuestionPreviousDescription => {
            "Select the previous pending question in the dispatch center."
        }
        UiTextKey::KeybindingUiCommandCenterQuestionNextTitle => "Next Dispatch Center Question",
        UiTextKey::KeybindingUiCommandCenterQuestionNextDescription => {
            "Select the next pending question in the dispatch center."
        }
        UiTextKey::KeybindingUiCommandCenterQuestionSubmitTitle => {
            "Submit Dispatch Center Question"
        }
        UiTextKey::KeybindingUiCommandCenterQuestionSubmitDescription => {
            "Submit the selected dispatch center question response."
        }
        UiTextKey::KeybindingUiPaletteNextTitle => "Select Next Palette Item",
        UiTextKey::KeybindingUiPaletteNextDescription => {
            "Move selection to the next item in an open palette."
        }
        UiTextKey::KeybindingUiPalettePreviousTitle => "Select Previous Palette Item",
        UiTextKey::KeybindingUiPalettePreviousDescription => {
            "Move selection to the previous item in an open palette."
        }
        UiTextKey::KeybindingUiPaletteAcceptTitle => "Confirm Palette Selection",
        UiTextKey::KeybindingUiPaletteAcceptDescription => {
            "Open the selected item in an active palette."
        }
        UiTextKey::KeybindingUiPaletteDismissTitle => "Close Palette",
        UiTextKey::KeybindingUiPaletteDismissDescription => {
            "Close the active palette without selecting an item."
        }
        UiTextKey::KeybindingUiProjectPanelPreviousPageTitle => "Project Panel: Previous Page",
        UiTextKey::KeybindingUiProjectPanelPreviousPageDescription => {
            "Switch to the previous page in the project panel."
        }
        UiTextKey::KeybindingUiProjectPanelNextPageTitle => "Project Panel: Next Page",
        UiTextKey::KeybindingUiProjectPanelNextPageDescription => {
            "Switch to the next page in the project panel."
        }
        UiTextKey::KeybindingUiProjectPanelFilesPageTitle => "Project Panel: Files",
        UiTextKey::KeybindingUiProjectPanelFilesPageDescription => {
            "Show and focus the Files page in the project panel."
        }
        UiTextKey::KeybindingUiProjectsFocusListTitle => "Focus Projects List",
        UiTextKey::KeybindingUiProjectsFocusListDescription => {
            "Move keyboard focus to the opened-projects list."
        }
        UiTextKey::KeybindingUiProjectTreeCollapseAllNodesTitle => {
            "Collapse All Project Directories"
        }
        UiTextKey::KeybindingUiProjectTreeCollapseAllNodesDescription => {
            "Collapse every expanded directory in the project tree."
        }
        UiTextKey::KeybindingUiProjectTreeToggleHiddenFilesTitle => "Toggle Hidden Project Files",
        UiTextKey::KeybindingUiProjectTreeToggleHiddenFilesDescription => {
            "Show or hide dotfiles in the project tree."
        }
        UiTextKey::KeybindingUiProjectTreeCreateFileTitle => "New Project File",
        UiTextKey::KeybindingUiProjectTreeCreateFileDescription => {
            "Create a file in the selected project directory."
        }
        UiTextKey::KeybindingUiProjectTreeCreateDirectoryTitle => "New Project Directory",
        UiTextKey::KeybindingUiProjectTreeCreateDirectoryDescription => {
            "Create a directory in the selected project directory."
        }
        UiTextKey::KeybindingUiProjectTreeRenameEntryTitle => "Rename Project Entry",
        UiTextKey::KeybindingUiProjectTreeRenameEntryDescription => {
            "Rename the selected project file or directory."
        }
        UiTextKey::KeybindingUiProjectTreeDeleteEntryTitle => "Delete Project Entry",
        UiTextKey::KeybindingUiProjectTreeDeleteEntryDescription => {
            "Delete the selected project file or directory."
        }
        UiTextKey::KeybindingUiProjectTreeCopyEntryTitle => "Copy Project Entry",
        UiTextKey::KeybindingUiProjectTreeCopyEntryDescription => {
            "Copy the selected project file or directory."
        }
        UiTextKey::KeybindingUiProjectTreeCutEntryTitle => "Cut Project Entry",
        UiTextKey::KeybindingUiProjectTreeCutEntryDescription => {
            "Cut the selected project file or directory."
        }
        UiTextKey::KeybindingUiProjectTreePasteEntryTitle => "Paste Project Entry",
        UiTextKey::KeybindingUiProjectTreePasteEntryDescription => {
            "Paste a copied project entry into the selected directory."
        }
        UiTextKey::KeybindingUiGitDiffDismissTitle => "Close Git Diff",
        UiTextKey::KeybindingUiGitDiffDismissDescription => "Close the active Git diff panel.",
        UiTextKey::KeybindingUiGitDiffStageModeTitle => "Toggle Git Diff Stage",
        UiTextKey::KeybindingUiGitDiffStageModeDescription => {
            "Switch between staged and unstaged changes."
        }
        UiTextKey::KeybindingUiGitDiffViewModeTitle => "Toggle Git Diff View",
        UiTextKey::KeybindingUiGitDiffViewModeDescription => {
            "Switch between unified and split diff views."
        }
        UiTextKey::KeybindingUiGitDiffWhitespaceTitle => "Toggle Git Diff Whitespace",
        UiTextKey::KeybindingUiGitDiffWhitespaceDescription => {
            "Include or ignore whitespace changes."
        }
        UiTextKey::KeybindingUiGitDiffPreviousFileTitle => "Previous Git Diff File",
        UiTextKey::KeybindingUiGitDiffPreviousFileDescription => {
            "Select the previous changed file."
        }
        UiTextKey::KeybindingUiGitDiffNextFileTitle => "Next Git Diff File",
        UiTextKey::KeybindingUiGitDiffNextFileDescription => "Select the next changed file.",
        UiTextKey::KeybindingUiGitDiffCopyTitle => "Copy Git Diff",
        UiTextKey::KeybindingUiGitDiffCopyDescription => "Copy the selected file diff.",
        UiTextKey::KeybindingUiTerminalTabTitle => "Send Tab to Terminal",
        UiTextKey::KeybindingUiTerminalTabDescription => "Send a Tab key to the focused terminal.",
        UiTextKey::KeybindingUiTerminalBacktabTitle => "Send Backtab to Terminal",
        UiTextKey::KeybindingUiTerminalBacktabDescription => {
            "Send a reverse Tab key to the focused terminal."
        }
        UiTextKey::KeybindingUiTerminalClipboardCopyTitle => "Copy from Terminal",
        UiTextKey::KeybindingUiTerminalClipboardCopyDescription => {
            "Copy the current terminal selection."
        }
        UiTextKey::KeybindingUiTerminalClipboardPasteTitle => "Paste into Terminal",
        UiTextKey::KeybindingUiTerminalClipboardPasteDescription => {
            "Paste clipboard text into the focused terminal."
        }
        UiTextKey::KeybindingUiTerminalSearchOpenTitle => "Open Terminal Search",
        UiTextKey::KeybindingUiTerminalSearchOpenDescription => {
            "Open search in the focused terminal."
        }
        UiTextKey::KeybindingUiTerminalSearchForwardTitle => "Next Terminal Search Match",
        UiTextKey::KeybindingUiTerminalSearchForwardDescription => {
            "Select the next terminal search match."
        }
        UiTextKey::KeybindingUiTerminalSearchBackwardTitle => "Previous Terminal Search Match",
        UiTextKey::KeybindingUiTerminalSearchBackwardDescription => {
            "Select the previous terminal search match."
        }
        UiTextKey::KeybindingUiTerminalSearchHistoryUpTitle => "Previous Terminal Search",
        UiTextKey::KeybindingUiTerminalSearchHistoryUpDescription => {
            "Recall the previous terminal search query."
        }
        UiTextKey::KeybindingUiTerminalSearchHistoryDownTitle => "Next Terminal Search",
        UiTextKey::KeybindingUiTerminalSearchHistoryDownDescription => {
            "Recall the next terminal search query."
        }
        UiTextKey::KeybindingUiTerminalSearchDismissTitle => "Close Terminal Search or Hint",
        UiTextKey::KeybindingUiTerminalSearchDismissDescription => {
            "Close terminal search or keyboard hint mode."
        }
        UiTextKey::KeybindingUiTerminalHintOpenTitle => "Open Terminal Keyboard Hints",
        UiTextKey::KeybindingUiTerminalHintOpenDescription => {
            "Open keyboard-selectable links and text hints."
        }
        UiTextKey::KeybindingUiCloseAllTabsTitle => "Close All Tabs",
        UiTextKey::KeybindingUiCloseAllTabsDescription => "Close every tab in the active pane.",
        UiTextKey::KeybindingUiCloseTabsBeforeTitle => "Close Tabs Before",
        UiTextKey::KeybindingUiCloseTabsBeforeDescription => "Close tabs before the active tab.",
        UiTextKey::KeybindingUiCloseTabsAfterTitle => "Close Tabs After",
        UiTextKey::KeybindingUiCloseTabsAfterDescription => "Close tabs after the active tab.",
        UiTextKey::KeybindingUiCloseAllFileTabsTitle => "Close All File Tabs",
        UiTextKey::KeybindingUiCloseAllFileTabsDescription => "Close every file tab.",
        UiTextKey::KeybindingUiCloseAllTerminalTabsTitle => "Close All Terminal Tabs",
        UiTextKey::KeybindingUiCloseAllTerminalTabsDescription => "Close every terminal tab.",
        UiTextKey::TerminalI18nIoError => "Terminal {operation} error: {message}",
        UiTextKey::TerminalI18nIoRead => "read",
        UiTextKey::TerminalI18nIoWrite => "write",
        UiTextKey::TerminalI18nIoResize => "resize",
        UiTextKey::TerminalI18nIoMailbox => "event mailbox",
        UiTextKey::TerminalI18nIoInputQueue => "input queue",
        UiTextKey::TerminalI18nIoClipboard => "clipboard",
        UiTextKey::TerminalI18nHostConnectionNotReady => "Host terminal connection is not ready.",
        UiTextKey::TerminalI18nInputTerminalNotRunning => {
            "Terminal is not running; inspect the panes list before sending input."
        }
        UiTextKey::TerminalI18nInputNoHostChannel => "Terminal has no Host input channel.",
        UiTextKey::TerminalI18nInputControlUnavailable => "Terminal input control is unavailable.",
        UiTextKey::TerminalI18nInputViewNotReady => "Terminal view is not ready.",
        UiTextKey::TerminalI18nInputEmpty => "Input is empty.",
        UiTextKey::TerminalI18nHostRuntimeUnavailable => "Host runtime is unavailable.",
        UiTextKey::TerminalI18nHostRuntimeNotConnected => "Host runtime is not connected.",
        UiTextKey::TerminalI18nHostTerminalRequestClosed => "Host terminal request channel closed.",
        UiTextKey::TerminalI18nHostTerminalResponseUnexpected => {
            "Unexpected Host terminal response: {response}"
        }
        UiTextKey::TerminalI18nSessionMissingFromCatalog => {
            "Terminal session is no longer present in the Host catalog."
        }
        UiTextKey::TerminalI18nLeaseRevoked => {
            "Terminal input lease was revoked by another client."
        }
        UiTextKey::TerminalI18nLeaseReleased => "Terminal input lease was released.",
        UiTextKey::TerminalI18nLeaseExpired => "Terminal input lease expired after idle takeover.",
        UiTextKey::TerminalI18nControlRequested => {
            "Another client ({client}) requested terminal control."
        }
        UiTextKey::TerminalI18nSessionUnavailable => "Terminal session unavailable",
        UiTextKey::TerminalI18nCommand => "command: {command}",
        UiTextKey::TerminalI18nWorkingDirectory => "cwd: {cwd}",
        UiTextKey::TerminalI18nStatus => "status: {status}",
        UiTextKey::TerminalI18nProcessLifecycle => "Process {state}",
        UiTextKey::TerminalI18nResumeSavedSession => "Resume saved session",
        UiTextKey::TerminalI18nStartNewProcess => "Start a new process",
        UiTextKey::TerminalI18nRetry => "Retry",
        UiTextKey::TerminalI18nRestart => "Restart",
        UiTextKey::TerminalI18nLifecycleIdle => "idle",
        UiTextKey::TerminalI18nLifecycleStarting => "starting",
        UiTextKey::TerminalI18nLifecycleRunning => "running",
        UiTextKey::TerminalI18nLifecycleStopping => "stopping",
        UiTextKey::TerminalI18nLifecycleExitedCode => "exited {code}",
        UiTextKey::TerminalI18nLifecycleKilled => "killed",
        UiTextKey::TerminalI18nLifecycleExited => "exited",
        UiTextKey::TerminalI18nLifecycleSpawnFailed => "spawn failed",
        UiTextKey::TerminalI18nLifecycleLost => "lost",
        UiTextKey::TerminalI18nStartFailed => "Failed to start terminal",
        UiTextKey::TerminalI18nError => "error: {error}",
        UiTextKey::TerminalI18nDidNotStart => "terminal did not start",
        UiTextKey::TerminalI18nAgentStarting => "agent starting",
        UiTextKey::TerminalI18nAgentIdle => "agent idle",
        UiTextKey::TerminalI18nAgentWorking => "agent working",
        UiTextKey::TerminalI18nAgentWaiting => "agent waiting",
        UiTextKey::TerminalI18nAgentCompleted => "agent completed",
        UiTextKey::TerminalI18nAgentFailed => "agent failed",
        UiTextKey::TerminalI18nAgentInterrupted => "agent interrupted",
        UiTextKey::TerminalI18nAgentStatusUnknown => "agent status unknown",
        UiTextKey::BarI18nEditorPosition => "Line {line}, column {column}",
        UiTextKey::BarI18nEditorModified => "● modified",
        UiTextKey::BarI18nEditorSaved => "saved",
        UiTextKey::BarI18nEditorNoDiagnostics => "no diagnostics",
        UiTextKey::BarI18nEditorSelectionSingle => "{characters} chars · {lines} line",
        UiTextKey::BarI18nEditorSelectionMultiple => "{characters} chars · {lines} lines",
        UiTextKey::BarI18nEditorSelectionTooltipSingle => {
            "{characters} Unicode scalar characters across {lines} selected line"
        }
        UiTextKey::BarI18nEditorSelectionTooltipMultiple => {
            "{characters} Unicode scalar characters across {lines} selected lines"
        }
        UiTextKey::BarI18nEditorNoSelection => "no selection",
        UiTextKey::BarI18nEditorTabSize => "tab {size}",
        UiTextKey::BarI18nEditorTabSizeTooltip => "Tab size: {size} spaces",
        UiTextKey::BarI18nEditorWrap => "wrap",
        UiTextKey::BarI18nEditorNoWrap => "no wrap",
        UiTextKey::BarI18nEditorWrapEnabled => "Soft wrapping enabled",
        UiTextKey::BarI18nEditorWrapDisabled => "Soft wrapping disabled",
        UiTextKey::BarI18nTerminalRunning => "running",
        UiTextKey::BarI18nTerminalStopped => "stopped",
        UiTextKey::BarI18nTerminalExitCode => "exit {code} · {reason}",
        UiTextKey::BarI18nTerminalExit => "exit · {reason}",
        UiTextKey::BarI18nTerminalExitCompleted => "completed",
        UiTextKey::BarI18nTerminalExitFailed => "failed",
        UiTextKey::BarI18nTerminalExitKilled => "killed",
        UiTextKey::BarI18nTerminalSizeTooltip => "{columns} columns × {rows} rows",
        UiTextKey::BarI18nAgentWaiting => "waiting",
        UiTextKey::BarI18nAgentWaitingReason => "waiting · {reason}",
        UiTextKey::BarI18nAgentWaitApproval => "approval",
        UiTextKey::BarI18nAgentWaitInput => "input",
        UiTextKey::BarI18nAgentWaitExternal => "external",
        UiTextKey::BarI18nAgentModel => "model {model}",
        UiTextKey::BarI18nAgentModelTooltip => "Agent model: {model}",
        UiTextKey::BarI18nAgentChildren => "children {count}",
        UiTextKey::BarI18nAgentChildrenTooltip => "{count} active child agents",
        UiTextKey::BarI18nAgentStateDurationTooltip => "Current selected agent state duration",
        UiTextKey::BarI18nDurationSeconds => "{seconds}s",
        UiTextKey::BarI18nDurationMinutesSeconds => "{minutes}m {seconds}s",
        UiTextKey::BarI18nDurationHoursMinutes => "{hours}h {minutes}m",
        UiTextKey::BarI18nDurationDaysHours => "{days}d {hours}h",
        UiTextKey::BarI18nUpdateNotChecked => "Update status has not been checked",
        UiTextKey::BarI18nUpdateAvailable => "v{version} available",
        UiTextKey::CliLocalizationCtlHelp => {
            "Usage: yttt ctl <resource> <action> [options]\n\n  windows list\n  projects list\n  tabs list | create | focus | rename | close\n  panes list | split | focus | rename | close | resize | send | read\n  agents list | create | send\n  board show                   Opens the native multi-project Dispatch center\n\nTargets:\n  --window ID                  Window ID from windows list (required if ambiguous)\n  --project ID                 Project ID from projects list\n  --tab ID                     Terminal tab ID from tabs list\n  --pane ID                    Pane ID within the tab\n\nCreate / manage:\n  tabs create --project ID [--command 'cargo test'] [--title NAME]\n  agents create --project ID --provider codex [--title NAME] [-- <provider arguments>]\n  panes split --project ID --tab ID --pane ID --direction horizontal|vertical [--command CMD]\n  panes resize --project ID --tab ID --pane ID --direction left|right|up|down [--percent 5]\n  tabs|panes rename <targets> --title NAME\n  tabs|panes close <targets>   Terminates processes in the target\n\nInput:\n  panes send <targets> --text TEXT [--enter] [--raw]\n  agents send <targets> --text TEXT\n  agents send <targets> --prompt-file PATH\n  panes|agents send <targets> --stdin\n  panes read <targets>        Reads the current terminal viewport\n  board answer <targets> --request ID --answers-json JSON\n                              Queues structured answers; inspect Agent confirmation\n\n  --json                      Machine-readable response (errors go to stderr)\n  --help                      Show this help\n\nThe desktop must already be running. Set YTTT_PROFILE_ROOT for a custom profile.\nLists cover open desktop windows and terminal tabs. Mutations select their target.\nCreated means the layout exists and startup was requested; list panes to check readiness.\nInputAccepted means Host accepted the bytes, not that a command or Agent turn completed.\nAgent send submits text plus Enter and rejects working/waiting/stale Agent states.\nUse panes send for deliberate raw interaction with a busy Agent or an approval prompt.\nRequests are never automatically retried. After a timeout inspect state before retrying.\n"
        }
        UiTextKey::CliLocalizationArgumentsUtf8 => "CLI arguments must be valid UTF-8",
        UiTextKey::CliLocalizationExpectedResource => "Expected a resource; see yttt ctl --help",
        UiTextKey::CliLocalizationExpectedAction => "Expected an action; see yttt ctl --help",
        UiTextKey::CliLocalizationUnexpectedArgument => "Unexpected argument: {argument}",
        UiTextKey::CliLocalizationMissingOptionValue => "Missing value for {option}",
        UiTextKey::CliLocalizationDuplicateOption => "Duplicate option: {option}",
        UiTextKey::CliLocalizationRequestRequired => "--request is required",
        UiTextKey::CliLocalizationAnswersJsonRequired => "--answers-json is required",
        UiTextKey::CliLocalizationInvalidStructuredAnswers => "Invalid structured answers: {error}",
        UiTextKey::CliLocalizationProviderRequired => "--provider is required",
        UiTextKey::CliLocalizationSplitDirection => "--direction must be horizontal or vertical",
        UiTextKey::CliLocalizationTitleRequired => "--title is required",
        UiTextKey::CliLocalizationResizeDirection => "--direction must be left, right, up or down",
        UiTextKey::CliLocalizationPercentRange => "--percent must be a number between 1 and 90",
        UiTextKey::CliLocalizationInputSourceExactlyOne => {
            "Supply exactly one of --text, --prompt-file or --stdin"
        }
        UiTextKey::CliLocalizationAgentRawInput => "Use panes send --raw for raw terminal input",
        UiTextKey::CliLocalizationUnknownCommand => {
            "Unknown command: {resource} {action}; see yttt ctl --help"
        }
        UiTextKey::CliLocalizationUnknownOption => "Unknown option for this command: {option}",
        UiTextKey::CliLocalizationProviderArgumentsOnly => {
            "Provider arguments are only accepted by agents create"
        }
        UiTextKey::CliLocalizationTabsRejectPane => "Tab commands do not accept --pane",
        UiTextKey::CliLocalizationPaneRequired => "--pane is required",
        UiTextKey::CliLocalizationRequestValidationError => "{error}",
        UiTextKey::CliLocalizationControlFailure => {
            "{error}. Ensure the matching desktop is running; inspect state before retrying mutations"
        }
        UiTextKey::CliLocalizationTargetPane => " pane={pane}",
        UiTextKey::CliLocalizationTarget => "window={window} project={project} tab={tab}{pane}",
        UiTextKey::CliLocalizationWindowItem => {
            "{id}\twritable={writable}\tloading={loading}\tproject={project}"
        }
        UiTextKey::CliLocalizationBooleanTrue => "true",
        UiTextKey::CliLocalizationBooleanFalse => "false",
        UiTextKey::CliLocalizationTabItem => "{target}\t{title}\tpanes={panes}",
        UiTextKey::CliLocalizationCreated => "Created {target} ({state})",
        UiTextKey::CliLocalizationUpdated => "Updated {target}",
        UiTextKey::CliLocalizationClosed => "Closed {target}",
        UiTextKey::CliLocalizationInputAccepted => "Host accepted {bytes} bytes for {target}",
        UiTextKey::CliLocalizationCommandCenterShown => "Dispatch center shown in window={window}",
        UiTextKey::CliLocalizationAnswerQueued => {
            "Answer queued for {target} request={request}; awaiting Agent confirmation"
        }
        UiTextKey::CliLocalizationLifecycleCommandExclusive => {
            "only one Host lifecycle command may be used at a time"
        }
        UiTextKey::CliLocalizationLoginStartupConfirmationRequired => {
            "--enable-login-startup requires --confirm-remote-access after reviewing the background Host access prompt"
        }
        UiTextKey::CliLocalizationProfileIdDuplicate => "--profile-id may be specified only once",
        UiTextKey::CliLocalizationProfileMismatch => {
            "login startup requested profile {requested_profile}, but this executable owns profile {profile}"
        }
        UiTextKey::CliLocalizationLoginStartupDisabled => "disabled",
        UiTextKey::CliLocalizationLoginStartupEnabled => "enabled",
        UiTextKey::CliLocalizationLoginStartupRequiresApproval => {
            "requires approval in system settings"
        }
        UiTextKey::CliLocalizationLoginStartupUnavailable => "unavailable",
        UiTextKey::CliLocalizationLoginStartupStatus => "Login startup {status} ({method})",
        UiTextKey::CliLocalizationHostForceStopping => "force stopping",
        UiTextKey::CliLocalizationOpenLogsFailure => "Failed to open yttt logs: {error}",
        UiTextKey::CliLocalizationLoginStartupFailure => "Login startup command failed: {error}",
        UiTextKey::CliLocalizationDetachedHostStdioFailure => {
            "Failed to isolate detached Host from desktop CLI stdio: {error}"
        }
        UiTextKey::CliLocalizationLifecycleRuntimeFailure => {
            "Failed to initialize Host lifecycle runtime: {error}"
        }
        UiTextKey::CliLocalizationHostLauncherFailure => {
            "Failed to initialize Host launcher: {error}"
        }
        UiTextKey::CliLocalizationHostStopped => "Host stopped",
        UiTextKey::CliLocalizationHostStatus => {
            "Host {state}: {terminals} terminals, {clients} clients, {projects} projects, {ssh_connections} SSH connections, {jobs} jobs"
        }
        UiTextKey::CliLocalizationUnexpectedHostStatusResponse => {
            "Unexpected Host status response: {response}"
        }
        UiTextKey::CliLocalizationHostStarted => "Host started",
        UiTextKey::CliLocalizationHostAlreadyRunning => "Host already running",
        UiTextKey::CliLocalizationHostBusy => "Host is busy with {count} blocker(s): {blockers}",
        UiTextKey::CliLocalizationUnexpectedHostStopResponse => {
            "Unexpected Host stop response: {response}"
        }
        UiTextKey::CliLocalizationHostRestarted => "Host restarted",
        UiTextKey::CliLocalizationHostForceStopped => "Host force-stopped",
        UiTextKey::CliLocalizationUnexpectedHostForceStopResponse => {
            "Unexpected Host force-stop response: {response}"
        }
        UiTextKey::CliLocalizationHostLifecycleFailure => "Host lifecycle command failed: {error}",
        UiTextKey::CliLocalizationHostExitTimeout => "Timed out waiting for Host exit",
        UiTextKey::CliLocalizationRemoteClientLaunchInvalid => {
            "Invalid remote Client launch: {error}"
        }
        UiTextKey::CliLocalizationInstallerPreflightArgumentError => {
            "Installer Host preflight argument error: {error}"
        }
        UiTextKey::CliLocalizationDesktopActivationFailure => {
            "Failed to activate yttt desktop shell: {error}"
        }
        UiTextKey::CliLocalizationHostRuntimeFailure => {
            "Failed to initialize Host runtime: {error}"
        }
        UiTextKey::CliLocalizationHostProcessFailure => "yttt Host failed: {error}",
        UiTextKey::CliLocalizationRegistrationPermissionPrompt => {
            "yttt needs permission to install its terminal command at /usr/local/bin/yttt."
        }
        UiTextKey::CliLocalizationRegistrationFailure => {
            "The yttt terminal command could not be installed. The application can still be used.\n\n{error}\n\nResolve the conflict or allow installation, then restart yttt to retry. You can also run the application's Contents/MacOS/yttt executable directly."
        }
        UiTextKey::CliLocalizationRegistrationAlertTitle => "yttt command-line installation",
        UiTextKey::CliLocalizationPromptInputReadFailure => "Failed to read prompt input: {error}",
        UiTextKey::CliLocalizationControlRuntimeFailure => {
            "Failed to initialize desktop control runtime: {error}"
        }
        UiTextKey::CliLocalizationStatePending => "pending",
        UiTextKey::CliLocalizationErrorInvalidRequest => "InvalidRequest",
        UiTextKey::CliLocalizationErrorNotFound => "NotFound",
        UiTextKey::CliLocalizationErrorAmbiguousTarget => "AmbiguousTarget",
        UiTextKey::CliLocalizationErrorNotReady => "NotReady",
        UiTextKey::CliLocalizationErrorPermissionDenied => "PermissionDenied",
        UiTextKey::CliLocalizationErrorBusy => "Busy",
        UiTextKey::CliLocalizationErrorOutcomeUnknown => "OutcomeUnknown",
        UiTextKey::CliLocalizationErrorFailed => "Failed",
        UiTextKey::WorkbenchI18nConnectionAddressPlaceholder => "host:port",
        UiTextKey::WorkbenchI18nExistingHostOperationFailed => "Connection operation failed",
        UiTextKey::WorkbenchI18nSharedEditingControlRequired => {
            "Shared editing control is required."
        }
        UiTextKey::WorkbenchI18nDirectoryPickerFailed => "Could not open the directory picker",
        UiTextKey::WorkbenchI18nDirectoryPickerInterrupted => "Directory selection was interrupted",
        UiTextKey::WorkbenchI18nAgentSnapshotProjectMismatch => {
            "Host Agent snapshot project does not match terminal {terminal}"
        }
        UiTextKey::WorkbenchI18nAgentSnapshotApplyFailed => "Failed to apply Agent snapshot",
        UiTextKey::WorkbenchI18nAgentSessionResumeFailedRetained => {
            "The Agent session could not be resumed; the original session is retained. Retry or explicitly open a new session."
        }
        UiTextKey::WorkbenchI18nAgentSessionsLoadError => "Could not load Agent sessions",
        UiTextKey::WorkbenchI18nAgentSessionUnavailable => "Agent session is no longer available.",
        UiTextKey::WorkbenchI18nAgentSessionProviderNoResumeCommand => {
            "{provider} does not provide a resume command for this session."
        }
        UiTextKey::WorkbenchI18nAgentSessionResumeFailed => "Could not resume Agent session",
        UiTextKey::WorkbenchI18nAgentSessionsMinutesAgo => "{count}m",
        UiTextKey::WorkbenchI18nAgentSessionsHoursAgo => "{count}h",
        UiTextKey::WorkbenchI18nAgentSessionsDaysAgo => "{count}d",
        UiTextKey::WorkbenchI18nAuxiliaryWindowOpenFailed => "Could not open window",
        UiTextKey::WorkbenchI18nCloseProjectKeyboardHint => "Enter to close, Escape to cancel",
        UiTextKey::WorkbenchI18nGitOperationFailed => "Git operation failed",
        UiTextKey::WorkbenchI18nProjectServicesUnavailable => "Project services are unavailable",
        UiTextKey::WorkbenchI18nDevicePreferencesUnbound => "Device preferences are not bound.",
        UiTextKey::WorkbenchI18nBarsEditorSerializeFailed => "Failed to serialize bars TOML",
        UiTextKey::WorkbenchI18nLayoutEditorPrepareFailed => "Failed to prepare layout TOML",
        UiTextKey::WorkbenchI18nLayoutEditorSerializeFailed => "Failed to serialize layout TOML",
        UiTextKey::WorkbenchI18nLayoutEditorParsePosition => "line {line}, column {column}",
        UiTextKey::WorkbenchI18nCommandUnavailableDetail => "Command unavailable: {reason}",
        UiTextKey::WorkbenchI18nProjectHostConfigurationUnavailable => {
            "The selected project has no Host configuration path."
        }
        UiTextKey::WorkbenchI18nRemoteAccessInvalidAddress => "Invalid listen address",
        UiTextKey::WorkbenchI18nRemoteAccessNotListening => "Remote access is not listening",
        UiTextKey::WorkbenchI18nRemoteAccessActionFailed => "Remote access action failed",
        UiTextKey::WorkbenchI18nRemoteConnectionKindHost => "Host",
        UiTextKey::WorkbenchI18nSshConnectionOperationFailed => "SSH connection operation failed",
        UiTextKey::WorkbenchI18nSshRequiredFields => "Name, host, and user are required.",
        UiTextKey::WorkbenchI18nFieldRequired => "{field} is required.",
        UiTextKey::WorkbenchI18nRemoteRestoreRemovalFailed => {
            "Could not remove remote restore entry"
        }
        UiTextKey::WorkbenchI18nSshConnectionNotConfigured => {
            "SSH connection {connection} is not configured"
        }
        UiTextKey::WorkbenchI18nSshProjectConnectionRequired => {
            "Connect the SSH endpoint before opening its remote project."
        }
        UiTextKey::WorkbenchI18nSshProjectPickerOperationFailed => "SSH project operation failed",
        UiTextKey::WorkbenchI18nSshProjectHomeUnavailable => {
            "Remote home directory is not available yet."
        }
        UiTextKey::WorkbenchI18nWorkspaceHistoryLoadError => "Could not load workspace history",
        UiTextKey::WorkbenchI18nWorkspaceHistoryOpenError => "Could not open workspace",
    }
}
