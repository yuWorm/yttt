use super::UiTextKey;

pub(super) fn text(key: UiTextKey) -> &'static str {
    match key {
        UiTextKey::AppName => "yttt",
        UiTextKey::FileOpen => "打开",
        UiTextKey::FileOpenExternal => "使用默认应用打开",
        UiTextKey::FileDownloadOpen => "下载并使用默认应用打开…",
        UiTextKey::FileDownloadedCopy => "打开的是本地副本，外部修改不会上传。",
        UiTextKey::FilePreview => "预览",
        UiTextKey::FilePreviewLoading => "正在加载预览…",
        UiTextKey::FilePreviewUnavailable => "无法预览",
        UiTextKey::FilePreviewFit => "适应窗口",
        UiTextKey::FilePreviewActualSize => "原始尺寸",
        UiTextKey::FilePreviewSource => "源码",
        UiTextKey::FilePreviewReload => "重新加载",
        UiTextKey::TrayOpen => "打开 yttt",
        UiTextKey::TrayNewWindow => "新建窗口",
        UiTextKey::TrayOpenLogs => "打开日志",
        UiTextKey::TrayStartHost => "启动 Host",
        UiTextKey::TrayStopHost => "空闲时停止 Host",
        UiTextKey::TrayRestartHost => "空闲时重启 Host",
        UiTextKey::TrayQuitDesktop => "退出桌面端",
        UiTextKey::TrayQuitAll => "全部退出",
        UiTextKey::TrayHost => "Host",
        UiTextKey::TrayConnecting => "正在连接",
        UiTextKey::TrayStopped => "已停止",
        UiTextKey::TrayUnavailable => "不可用",
        UiTextKey::TrayRunning => "运行中",
        UiTextKey::TrayDraining => "正在排空",
        UiTextKey::TrayStopping => "正在停止",
        UiTextKey::TrayUnexpectedResponse => "非预期响应",
        UiTextKey::TrayStatusSummary => {
            "{host}：{state} · {terminals} 个终端 · {clients} 个客户端 · {jobs} 个任务"
        }
        UiTextKey::OnboardingTitle => "欢迎使用 yttt",
        UiTextKey::OnboardingSubtitle => {
            "选择语言、终端字体、默认布局和编程 Agent，然后可选择从 Zed 导入主题。"
        }
        UiTextKey::OnboardingLanguageHeading => "选择语言",
        UiTextKey::OnboardingLanguageSubtitle => {
            "在此预览语言，完成引导后保存。之后也可以在设置中修改。"
        }
        UiTextKey::OnboardingFontHeading => "选择终端字体",
        UiTextKey::OnboardingFontSubtitle => {
            "终端默认使用内置的 Hack Nerd Font Mono。未配置字体时，会自动选中检测到的等宽 Nerd Font；你可以在这里或之后的设置中修改。"
        }
        UiTextKey::OnboardingFontRecommendation => {
            "终端默认使用内置的 Hack Nerd Font Mono。也可以安装其他等宽 Nerd Font，例如 Maple Mono NF："
        }
        UiTextKey::OnboardingFontDetectedRecommendation => {
            "检测到推荐的等宽 Nerd Font（已有选择会保留）："
        }
        UiTextKey::OnboardingAgentHeading => "选择编程 Agent",
        UiTextKey::OnboardingAgentSubtitle => "所选 Agent 将用于刚才选择的默认布局。",
        UiTextKey::OnboardingLayoutHeading => "选择默认布局",
        UiTextKey::OnboardingSplitLayoutTitle => "单标签分屏",
        UiTextKey::OnboardingSplitLayoutDescription => "一个标签页内并排显示 Agent 和 Shell。",
        UiTextKey::OnboardingTabsLayoutTitle => "独立标签页",
        UiTextKey::OnboardingTabsLayoutDescription => "Agent 和 Shell 分别使用独立标签页。",
        UiTextKey::OnboardingAgentPane => "Agent",
        UiTextKey::OnboardingShellPane => "Shell",
        UiTextKey::OnboardingCommandPaletteHint => "随时打开命令面板",
        UiTextKey::OnboardingNext => "下一步",
        UiTextKey::OnboardingBack => "上一步",
        UiTextKey::OnboardingContinue => "使用此布局",
        UiTextKey::OnboardingZedHeading => "从 Zed 导入主题",
        UiTextKey::OnboardingZedSubtitle => "已在安装的 Zed 扩展中探测到兼容主题，请确认是否导入。",
        UiTextKey::OnboardingZedUiThemes => "UI 主题",
        UiTextKey::OnboardingZedIconThemes => "图标主题",
        UiTextKey::OnboardingZedNoThemes => "未探测到兼容的 Zed 主题。",
        UiTextKey::OnboardingZedDetectionWarnings => "部分 Zed 扩展无法完成探测。",
        UiTextKey::OnboardingZedSkip => "跳过导入",
        UiTextKey::OnboardingZedImport => "导入并完成",
        UiTextKey::EmptySubtitle => "打开目录，或从最近项目中选择。",
        UiTextKey::EmptySidebarNote => "侧边栏只显示已打开的项目。",
        UiTextKey::CreateProjectDirectoryError => "创建项目目录失败",
        UiTextKey::CreateProjectPickerError => "选择项目位置失败",
        UiTextKey::OpenDirectory => "打开目录",
        UiTextKey::OpenRecent => "打开最近项目",
        UiTextKey::RestoreLastSession => "恢复上次打开",
        UiTextKey::CommandPalette => "命令面板",
        UiTextKey::NewTab => "新建标签页",
        UiTextKey::NoTerminalTabs => "暂无终端标签页",
        UiTextKey::Projects => "项目",
        UiTextKey::ProjectFiles => "文件",
        UiTextKey::AgentSessions => "Agent 会话",
        UiTextKey::AgentSessionsRefresh => "刷新会话",
        UiTextKey::AgentSessionsLoading => "正在扫描 Agent 会话…",
        UiTextKey::AgentSessionsPartial => "部分历史未能刷新；已有会话仍可搜索。",
        UiTextKey::AgentSessionsEmpty => "未找到当前项目的会话",
        UiTextKey::AgentSessionsSearchPlaceholder => "搜索会话…",
        UiTextKey::AgentSessionsNoMatches => "没有匹配的会话",
        UiTextKey::AgentSessionsRemoteUnavailable => "暂不支持扫描远程项目的会话",
        UiTextKey::AgentSessionsResumeHint => "双击恢复；右键重命名或删除会话记录",
        UiTextKey::AgentSessionsJustNow => "刚刚",
        UiTextKey::AgentSessionsSessionId => "会话 ID",
        UiTextKey::AgentSessionsModel => "模型",
        UiTextKey::AgentSessionsTranscript => "会话文件",
        UiTextKey::AgentSessionsUntitled => "未命名会话",
        UiTextKey::AgentSessionsRename => "重命名会话",
        UiTextKey::AgentSessionsRenameDescription => {
            "修改 Agent 原生会话名称，而非仅修改 yttt 中的显示名称。"
        }
        UiTextKey::AgentSessionsDelete => "删除会话记录",
        UiTextKey::AgentSessionsDeleteDescription => {
            "永久删除此 Agent 保存的对话及会话专属数据，无法撤销。不会删除项目文件。请先在所有终端中关闭此会话。"
        }
        UiTextKey::AgentSessionsGrokDeleteDescription => {
            "通过 Grok 永久删除此对话及会话专属数据。若 Grok 已登录，还可能删除同步的云端副本。此操作无法撤销，不会删除项目文件。请先在所有终端中关闭此会话。"
        }
        UiTextKey::AgentSessionsMutating => "正在更新会话记录…",
        UiTextKey::AgentSessionsMutationFailed => "会话操作失败",
        UiTextKey::AgentSessionsSelectionChanged => "所选项目已改变，请重新打开会话菜单。",
        UiTextKey::ProjectFilesShow => "显示项目面板",
        UiTextKey::ProjectFilesHide => "隐藏项目面板",
        UiTextKey::ProjectFilesRefresh => "刷新",
        UiTextKey::ProjectFilesLoading => "正在加载项目文件…",
        UiTextKey::CommandCenterTitle => "调度中心",
        UiTextKey::CommandCenterHostLive => "Host 在线",
        UiTextKey::CommandCenterHostLiveReadOnly => "Host 在线 · 只读",
        UiTextKey::CommandCenterHostDisconnectedRetained => "Host 已断开 · 保留历史",
        UiTextKey::CommandCenterHostUnavailableRetained => "Host 不可用 · 保留历史",
        UiTextKey::CommandCenterNoRecords => "此 Host 或工作区中没有可用的 yttt 管理 Agent 记录。",
        UiTextKey::CommandCenterProjectsCount => "{count} 个项目",
        UiTextKey::CommandCenterTasksCount => "{count} 个任务",
        UiTextKey::CommandCenterQuestionsCount => "{count} 个问题",
        UiTextKey::CommandCenterExecutionsCount => "{count} 条执行记录",
        UiTextKey::CommandCenterTaskAccepted => "已接受",
        UiTextKey::CommandCenterTaskRetired => "已退役",
        UiTextKey::CommandCenterPlanPending => "待处理",
        UiTextKey::CommandCenterPlanWorking => "进行中",
        UiTextKey::CommandCenterPlanCompleted => "已完成",
        UiTextKey::CommandCenterPlanBlocked => "受阻",
        UiTextKey::CommandCenterPlanAbandoned => "已放弃",
        UiTextKey::CommandCenterPlanUnknown => "未知",
        UiTextKey::CommandCenterQuestions => "待处理",
        UiTextKey::CommandCenterRecordsCount => "{count} 条记录",
        UiTextKey::CommandCenterNoQuestions => "暂无未解决的问题记录。",
        UiTextKey::CommandCenterOptionPreview => "预览",
        UiTextKey::CommandCenterQueueAnswer => "排队提交回答",
        UiTextKey::CommandCenterQueuedHint => "已排队不代表提供方已接受。",
        UiTextKey::CommandCenterLinkedConversation => "关联对话",
        UiTextKey::CommandCenterOpenSession => "打开会话",
        UiTextKey::CommandCenterOpenSessionHint => {
            "打开所属项目、标签页和终端；子 Agent 跳转到承载它的会话。"
        }
        UiTextKey::CommandCenterSessionNotOpen => "对应会话未在当前窗口打开。",
        UiTextKey::CommandCenterOpenSessionFailed => "无法打开会话",
        UiTextKey::CommandCenterNoSelection => "未选择",
        UiTextKey::CommandCenterSelectRecord => "请选择任务、问题或执行记录。",
        UiTextKey::CommandCenterNoConversation => "未报告此执行记录的对话。",
        UiTextKey::CommandCenterYou => "你",
        UiTextKey::CommandCenterAgent => "Agent",
        UiTextKey::CommandCenterMessageTruncated => "提供方截断了这条保留消息。",
        UiTextKey::CommandCenterPromptHint => "向此空闲 Agent 安全发送提示",
        UiTextKey::CommandCenterSend => "发送",
        UiTextKey::CommandCenterAcceptCompletedTask => "接受已完成任务",
        UiTextKey::CommandCenterAcceptHint => "人工接受仍独立于提供方完成状态。",
        UiTextKey::CommandCenterAcceptRetainedReadOnly => "保留历史为只读，无法在此接受此任务。",
        UiTextKey::CommandCenterAcceptDesktopReadOnly => "此桌面端为只读，已禁用任务接受。",
        UiTextKey::CommandCenterConfirmTerminateTerminal => "确认终止终端",
        UiTextKey::CommandCenterTerminateTerminal => "终止终端（窗格中的所有工作）",
        UiTextKey::CommandCenterTerminateHint => "这会关闭整个终端，而非某一条派生执行记录。",
        UiTextKey::CommandCenterTerminateDesktopReadOnly => "此桌面端为只读，已禁用终端终止。",
        UiTextKey::CommandCenterExecutionSummary => "会话摘要（提供方未报告执行记录）",
        UiTextKey::CommandCenterExecutionRecords => "Agent 活动",
        UiTextKey::CommandCenterNoCurrentExecutions => "暂无当前活动。",
        UiTextKey::CommandCenterActivityCount => "当前 {count}",
        UiTextKey::CommandCenterExecutionHistoryShow => "展开执行历史（{count}）",
        UiTextKey::CommandCenterExecutionHistoryHide => "收起执行历史（{count}）",
        UiTextKey::CommandCenterModel => "模型",
        UiTextKey::CommandCenterTokenCount => "{value} token",
        UiTextKey::CommandCenterLastRequestTokens => "{value} token（末次请求）",
        UiTextKey::CommandCenterLastRequestTokensHint => {
            "Claude Code 上报的是子 Agent 最后一次 API 请求的 token，并非整段执行的累计用量。"
        }
        UiTextKey::CommandCenterExecutionUsage => "执行用量 · 非单项 Todo 消耗",
        UiTextKey::CommandCenterTaskExecutionUsage => "执行 {value} tok",
        UiTextKey::CommandCenterTaskLastRequestUsage => "末次 {value} tok",
        UiTextKey::CommandCenterTaskUsageHint => {
            "这是关联执行的用量，不是单个计划条目的独立消耗。同一执行下的任务共享此数据，不可相加。"
        }
        UiTextKey::CommandCenterTaskUsageUnavailable => {
            "关联执行尚未上报 Token 用量；缺失不代表零消耗。"
        }
        UiTextKey::CommandCenterInputTokens => "输入 token",
        UiTextKey::CommandCenterOutputTokens => "输出 token",
        UiTextKey::CommandCenterCacheReadTokens => "缓存读取 token",
        UiTextKey::CommandCenterCacheWriteTokens => "缓存写入 token",
        UiTextKey::CommandCenterContextTokens => "上下文 token",
        UiTextKey::CommandCenterRequests => "请求次数",
        UiTextKey::CommandCenterToolCalls => "工具调用",
        UiTextKey::CommandCenterReportedDuration => "上报耗时",
        UiTextKey::CommandCenterCost => "上报费用（USD）",
        UiTextKey::CommandCenterSelfEstimate => "自估 {value}%",
        UiTextKey::CommandCenterSelfEstimateHint => {
            "Agent 自行估算，不代表已验收完成。未上报不等于 0%。"
        }
        UiTextKey::CommandCenterTelemetryObserved => "{value}前观测",
        UiTextKey::CommandCenterCurrentAction => "当前动作",
        UiTextKey::CommandCenterParentExecution => "父执行",
        UiTextKey::CommandCenterSessionActions => "会话操作…",
        UiTextKey::CommandCenterHideSessionActions => "收起会话操作",
        UiTextKey::CommandCenterShowResolvedQuestions => "展开已解决历史",
        UiTextKey::CommandCenterHideResolvedQuestions => "收起已解决历史",
        UiTextKey::CommandCenterSelectedQuestion => "已选问题",
        UiTextKey::CommandCenterPreviousQuestion => "上一个",
        UiTextKey::CommandCenterNextQuestion => "下一个",
        UiTextKey::CommandCenterQuestionShortcutsHint => {
            "Alt+↑/↓ 切换 · Ctrl/⌘+Enter 提交 · 聚焦问题后按 1–9 选择"
        }
        UiTextKey::CommandCenterOnlineAgentsCount => "在线主 Agent {count}",
        UiTextKey::CommandCenterActiveChildrenCount => "运行中子 Agent {count}",
        UiTextKey::CommandCenterAgentStarting => "启动中",
        UiTextKey::CommandCenterAgentOnlineIdle => "在线 · 空闲",
        UiTextKey::CommandCenterAgentOnlineWorking => "在线 · 工作中",
        UiTextKey::CommandCenterAgentOnlineWaiting => "在线 · 等待中",
        UiTextKey::CommandCenterAgentOnlineCompleted => "在线 · 本轮结束",
        UiTextKey::CommandCenterAgentOnlineFailed => "在线 · 本轮失败",
        UiTextKey::CommandCenterAgentOnlineInterrupted => "在线 · 本轮中断",
        UiTextKey::CommandCenterAgentOnlineUnknown => "在线 · 轮次未知",
        UiTextKey::CommandCenterAgentDisconnected => "观测中断",
        UiTextKey::CommandCenterAgentExited => "已退出",
        UiTextKey::CommandCenterAgentRetained => "保留记录 · 非实时",
        UiTextKey::CommandCenterPendingQuestionsCount => "待回答 {count}",
        UiTextKey::CommandCenterTaskHistoryShow => "展开历史记录（{count}）",
        UiTextKey::CommandCenterTaskHistoryHide => "收起历史记录（{count}）",
        UiTextKey::CommandCenterHistoryCompact => "历史 · {count}",
        UiTextKey::CommandCenterProjectUsageUnreported => {
            "项目累计 Token：尚无可累计的用量上报。未上报不等于 0。"
        }
        UiTextKey::CommandCenterProjectUsagePartial => {
            "项目累计 Token：至少 {tokens} · 部分覆盖（{reported}/{observed} 个观测来源已上报）"
        }
        UiTextKey::CommandCenterProjectUsageComplete => {
            "项目累计 Token：{tokens} · 已覆盖全部 {observed} 个已知计量来源"
        }
        UiTextKey::CommandCenterProjectUsageScope => {
            "累计提供方已上报的历史与新用量，按稳定来源去重并排除主子重叠；关闭会话、裁剪历史或重启不会清零。覆盖仅针对已知计量来源，不代表完整会话或项目自创建以来的总消耗；离线时保留最后记录。"
        }
        UiTextKey::CommandCenterDurationHours => "{value}小时",
        UiTextKey::CommandCenterDurationMinutes => "{value}分钟",
        UiTextKey::CommandCenterDurationSeconds => "{value}秒",
        UiTextKey::CommandCenterRoot => "根",
        UiTextKey::CommandCenterOptionalAnswerPlaceholder => "可选的补充回答",
        UiTextKey::CommandCenterPromptPlaceholder => "向选定的空闲 Agent 发送后续提示",
        UiTextKey::CommandCenterQuestionNoLongerRetained => "Host 不再保留此问题。",
        UiTextKey::CommandCenterAnswerAllQuestions => "请先回答所有问题，再将请求加入队列。",
        UiTextKey::CommandCenterAnswerRequired => "请为每个问题选择选项或输入补充文本。",
        UiTextKey::CommandCenterSendingToHost => "正在发送给 Host…",
        UiTextKey::CommandCenterQueuedWaitingProvider => "Host 已加入队列，等待提供方确认。",
        UiTextKey::CommandCenterUnexpectedCommandResponse => "Host 返回了意外的命令响应。",
        UiTextKey::CommandCenterHostRejectedCommand => "Host 拒绝了命令",
        UiTextKey::CommandCenterCommandChannelClosed => "Host 命令通道已关闭，不会重放命令。",
        UiTextKey::CommandCenterHostUnavailableReadOnly => "Host 不可用；保留历史为只读。",
        UiTextKey::CommandCenterLiveSnapshotRequired => "只有实时 Host 快照可以接收此命令。",
        UiTextKey::CommandCenterDesktopReadOnlyCommands => "此桌面端为只读，已禁用 Host 命令。",
        UiTextKey::CommandCenterQuestionNotInSelectedPane => "此问题不属于选中的实时 Agent 窗格。",
        UiTextKey::CommandCenterQuestionNotAwaiting => "问题当前为“{state}”，未等待回答。",
        UiTextKey::CommandCenterProviderNoAnswerChannel => "此提供方未公开活动回答通道。",
        UiTextKey::CommandCenterAnswerRejected => "回答被拒绝",
        UiTextKey::CommandCenterPromptRequired => "请输入提示后再发送。",
        UiTextKey::CommandCenterSessionNoLongerRetained => "不再保留所选会话。",
        UiTextKey::CommandCenterPromptOtherWindow => {
            "此 Agent 在另一个窗口中运行，请从其所属终端发送提示。"
        }
        UiTextKey::CommandCenterAwaitingHostInput => "正在等待 Host 确认输入…",
        UiTextKey::CommandCenterPromptAccepted => "Host 已接受终端输入；Agent 处理状态会单独显示。",
        UiTextKey::CommandCenterUnexpectedInputResponse => "Host 返回了意外的输入响应。",
        UiTextKey::CommandCenterHostRejectedInput => "Host 拒绝了终端输入",
        UiTextKey::CommandCenterInputFailed => "无法发送终端输入",
        UiTextKey::CommandCenterInputAcknowledgementLost => "已丢失 Host 确认，不会重放输入。",
        UiTextKey::CommandCenterTerminationConfirmed => {
            "Host 已确认终止终端；此窗格中的所有工作都将结束。"
        }
        UiTextKey::CommandCenterHostRejectedTermination => "Host 拒绝了终端终止请求",
        UiTextKey::CommandCenterQuestionSubmitted => "已由 Host 加入队列，等待提供方确认。",
        UiTextKey::CommandCenterQuestionAnswered => "提供方已记录此回答；保留卡片为只读。",
        UiTextKey::CommandCenterQuestionCancelled => "提供方已取消此问题；保留卡片为只读。",
        UiTextKey::CommandCenterQuestionUnavailable => "提供方回答通道不可用；保留历史为只读。",
        UiTextKey::CommandCenterQuestionLiveSnapshotRequired => {
            "只有实时 Host 快照可以回答这条保留问题。"
        }
        UiTextKey::CommandCenterDesktopReadOnlyAnswer => "此桌面端为只读，已禁用回答。",
        UiTextKey::CommandCenterPromptRetainedReadOnly => "保留历史为只读；原始终端未在此处运行。",
        UiTextKey::CommandCenterDesktopReadOnlyPrompt => "此桌面端为只读，已禁用提示。",
        UiTextKey::CommandCenterAgentNotRunning => "Agent 进程未运行。",
        UiTextKey::CommandCenterAgentBusy => "Agent 正忙；请等待空闲或完成的回合后再发送提示。",
        UiTextKey::CommandCenterTurnIdle => "空闲",
        UiTextKey::CommandCenterTurnWorking => "工作中",
        UiTextKey::CommandCenterTurnWaiting => "等待中",
        UiTextKey::CommandCenterTurnCompleted => "已完成",
        UiTextKey::CommandCenterTurnFailed => "失败",
        UiTextKey::CommandCenterTurnInterrupted => "已中断",
        UiTextKey::CommandCenterTurnUnknown => "未知",
        UiTextKey::CommandCenterQuestionAwaitingAnswer => "等待回答",
        UiTextKey::CommandCenterQuestionQueued => "已排队",
        UiTextKey::CommandCenterQuestionAnsweredStatus => "已回答",
        UiTextKey::CommandCenterQuestionCancelledStatus => "已取消",
        UiTextKey::CommandCenterQuestionUnavailableStatus => "不可用",
        UiTextKey::ProjectFilesEmptyDirectory => "空目录",
        UiTextKey::ProjectFilesDirectoryError => "无法加载目录",
        UiTextKey::ProjectFilesRetry => "重试",
        UiTextKey::ProjectFilesNewFile => "新建文件",
        UiTextKey::ProjectFilesNewDirectory => "新建文件夹",
        UiTextKey::ProjectFilesCreateProjectLayout => "创建项目布局",
        UiTextKey::ProjectFilesProjectLayoutReady => "项目布局已就绪",
        UiTextKey::ProjectFilesRename => "重命名",
        UiTextKey::ProjectFilesDelete => "删除",
        UiTextKey::ProjectFilesCopy => "复制",
        UiTextKey::ProjectFilesCut => "剪切",
        UiTextKey::ProjectFilesPaste => "粘贴",
        UiTextKey::ProjectFilesShowHidden => "显示隐藏文件",
        UiTextKey::ProjectFilesHideHidden => "隐藏隐藏文件",
        UiTextKey::ProjectFilesEntryPlaceholder => "输入名称或路径；以 / 结尾创建文件夹",
        UiTextKey::ProjectFilesDeleteConfirmTitle => "删除项目条目？",
        UiTextKey::ProjectFilesDeleteConfirmMessage => "所选文件或文件夹将被永久删除，且无法撤销。",
        UiTextKey::ProjectFileUnsupportedBinary => "暂不支持二进制文件",
        UiTextKey::ProjectFileInvalidEncoding => "仅支持 UTF-8 文件",
        UiTextKey::ProjectFileTooLarge => "文件超过编辑器 6 MiB 限制",
        UiTextKey::ProjectFileOutsideProject => "文件位于项目目录之外",
        UiTextKey::FileSaveAction => "保存",
        UiTextKey::FileSaving => "正在保存…",
        UiTextKey::FileSaved => "已保存",
        UiTextKey::FileSaveFailed => "保存失败",
        UiTextKey::FileChangedOnDisk => "文件已在磁盘上更改",
        UiTextKey::FileOverwrite => "覆盖",
        UiTextKey::FileReload => "重新加载",
        UiTextKey::MarkdownShowRendered => "显示 Markdown 渲染视图",
        UiTextKey::MarkdownShowSource => "编辑 Markdown 源码",
        UiTextKey::Tabs => "标签页",
        UiTextKey::TabCloseCurrent => "关闭当前",
        UiTextKey::TabCloseAll => "关闭全部",
        UiTextKey::TabCloseBefore => "关闭左侧标签页",
        UiTextKey::TabCloseAfter => "关闭右侧标签页",
        UiTextKey::TabCloseAllFiles => "关闭所有文件",
        UiTextKey::TabCloseAllTerminals => "关闭所有终端",
        UiTextKey::Lazy => "未启动",
        UiTextKey::Started => "已启动",
        UiTextKey::Active => "当前",
        UiTextKey::NoResults => "无结果",
        UiTextKey::TypeToFilter => "输入以筛选",
        UiTextKey::PalettePlaceholderCommand => "执行命令...",
        UiTextKey::PalettePlaceholderNewTabCommand => "选择新标签页要运行的命令...",
        UiTextKey::PalettePlaceholderFile => "搜索文件...",
        UiTextKey::PalettePlaceholderProject => "切换项目...",
        UiTextKey::PalettePlaceholderOpenedProject => "切换已打开项目...",
        UiTextKey::PalettePlaceholderRecentProject => "打开最近项目...",
        UiTextKey::PalettePlaceholderTab => "切换标签页...",
        UiTextKey::PalettePlaceholderPane => "切换面板...",
        UiTextKey::PalettePlaceholderGitBranch => "切换 Git 分支...",
        UiTextKey::FileFinderLoading => "正在索引项目文件…",
        UiTextKey::FileFinderLoadFailed => "无法索引项目文件",
        UiTextKey::FileFinderPreviewTitle => "文件预览",
        UiTextKey::FileFinderPreviewLoading => "正在加载预览…",
        UiTextKey::FileFinderPreviewEmpty => "无可用预览",
        UiTextKey::PaletteRun => "运行",
        UiTextKey::PaletteClose => "关闭",
        UiTextKey::PaletteStatusOpen => "已打开",
        UiTextKey::PaletteStatusRecent => "最近",
        UiTextKey::PaletteStatusRemoteRecent => "SSH 远程 · 最近",
        UiTextKey::PaletteStatusPaneSingular => "个面板",
        UiTextKey::PaletteStatusPanePlural => "个面板",
        UiTextKey::PaletteStatusActive => "当前",
        UiTextKey::PaletteStatusLazy => "未启动",
        UiTextKey::PaletteStatusStarted => "已启动",
        UiTextKey::PaletteStatusIdle => "空闲",
        UiTextKey::PaletteStatusRunning => "运行中",
        UiTextKey::PaletteStatusExited => "已退出",
        UiTextKey::PaletteStatusAgent => "Agent",
        UiTextKey::PaletteStatusAgentRunning => "Agent 运行中",
        UiTextKey::PaletteStatusAgentIdle => "Agent 空闲",
        UiTextKey::PaletteStatusAgentWaiting => "Agent 等待中",
        UiTextKey::PaletteStatusAgentCompleted => "Agent 已完成",
        UiTextKey::PaletteStatusAgentFailed => "Agent 失败",
        UiTextKey::PaletteStatusAgentInterrupted => "Agent 已中断",
        UiTextKey::PaletteStatusAgentStale => "Agent 状态过期",
        UiTextKey::CommandDisabledVisibleProjectActions => "请使用可见的项目操作",
        UiTextKey::CommandDisabledOpenProjectFirst => "请先打开项目",
        UiTextKey::CommandDisabledFocusProjectFileFirst => "请先聚焦一个项目文件",
        UiTextKey::CommandDisabledOpenWorkItemFirst => "请先打开终端或文件",
        UiTextKey::CommandDisabledSwitchTerminalFirst => "请先切换到终端标签页",
        UiTextKey::CommandUnavailable => "命令不可用",
        UiTextKey::CommandProjectCreateTitle => "创建项目",
        UiTextKey::CommandProjectCreateDescription => "创建并打开一个新的项目目录",
        UiTextKey::CommandProjectOpenTitle => "打开项目",
        UiTextKey::CommandProjectOpenDescription => "选择一个项目目录",
        UiTextKey::CommandProjectOpenSshTitle => "打开 SSH 项目",
        UiTextKey::CommandProjectOpenSshDescription => "连接并打开一个远程项目",
        UiTextKey::CommandProjectOpenRecentTitle => "打开最近项目",
        UiTextKey::CommandProjectOpenRecentDescription => "选择一个最近项目",
        UiTextKey::CommandProjectCloseTitle => "关闭项目",
        UiTextKey::CommandProjectCloseDescription => "关闭当前项目",
        UiTextKey::CommandProjectPaletteTitle => "打开项目面板",
        UiTextKey::CommandProjectPaletteDescription => "切换已打开或最近项目",
        UiTextKey::CommandProjectOpenedPaletteTitle => "切换已打开项目",
        UiTextKey::CommandProjectOpenedPaletteDescription => "只显示当前已打开的项目",
        UiTextKey::CommandProjectPanelToggleTitle => "切换项目面板",
        UiTextKey::CommandProjectPanelToggleDescription => "显示或隐藏项目侧边面板",
        UiTextKey::CommandProjectPanelRefreshTitle => "刷新项目文件树",
        UiTextKey::CommandProjectPanelRefreshDescription => "刷新项目文件树内容",
        UiTextKey::CommandGitBranchSwitchTitle => "切换 Git 分支",
        UiTextKey::CommandGitBranchSwitchDescription => "选择并检出一个 Git 分支",
        UiTextKey::CommandGitDiffOpenTitle => "查看 Git 更改",
        UiTextKey::CommandGitDiffOpenDescription => "打开当前项目的工作区差异",
        UiTextKey::CommandFileFindTitle => "查找文件",
        UiTextKey::CommandFileFindDescription => "在已打开项目中模糊搜索文件",
        UiTextKey::CommandFileSaveTitle => "保存文件",
        UiTextKey::CommandFileSaveDescription => "保存当前项目文件",
        UiTextKey::CommandTabNewTitle => "新建标签页",
        UiTextKey::CommandTabNewDescription => "在当前项目中新建 Shell 标签页",
        UiTextKey::CommandTabCloseTitle => "关闭标签页",
        UiTextKey::CommandTabCloseDescription => "关闭当前标签页",
        UiTextKey::CommandTabRenameTitle => "重命名标签页",
        UiTextKey::CommandTabRenameDescription => "重命名当前标签页",
        UiTextKey::CommandTabNextTitle => "下一个标签页",
        UiTextKey::CommandTabNextDescription => "切换到下一个项目标签页",
        UiTextKey::CommandTabPrevTitle => "上一个标签页",
        UiTextKey::CommandTabPrevDescription => "切换到上一个项目标签页",
        UiTextKey::CommandTabPaletteTitle => "打开标签页面板",
        UiTextKey::CommandTabPaletteDescription => "切换当前项目的标签页",
        UiTextKey::CommandPaneSplitHorizontalTitle => "水平拆分面板",
        UiTextKey::CommandPaneSplitHorizontalDescription => "将当前面板拆分为上下两个面板",
        UiTextKey::CommandPaneSplitVerticalTitle => "垂直拆分面板",
        UiTextKey::CommandPaneSplitVerticalDescription => "将当前面板拆分为左右两个面板",
        UiTextKey::CommandPaneCloseTitle => "关闭面板",
        UiTextKey::CommandPaneCloseDescription => "关闭当前面板",
        UiTextKey::CommandPaneFocusLeftTitle => "聚焦左侧面板",
        UiTextKey::CommandPaneFocusLeftDescription => "将焦点移动到左侧面板",
        UiTextKey::CommandPaneFocusRightTitle => "聚焦右侧面板",
        UiTextKey::CommandPaneFocusRightDescription => "将焦点移动到右侧面板",
        UiTextKey::CommandPaneFocusUpTitle => "聚焦上方面板",
        UiTextKey::CommandPaneFocusUpDescription => "将焦点移动到上方面板",
        UiTextKey::CommandPaneFocusDownTitle => "聚焦下方面板",
        UiTextKey::CommandPaneFocusDownDescription => "将焦点移动到下方面板",
        UiTextKey::CommandPaneResizeLeftTitle => "向左调整面板",
        UiTextKey::CommandPaneResizeLeftDescription => "将当前分屏向左调整",
        UiTextKey::CommandPaneResizeRightTitle => "向右调整面板",
        UiTextKey::CommandPaneResizeRightDescription => "将当前分屏向右调整",
        UiTextKey::CommandPaneResizeUpTitle => "向上调整面板",
        UiTextKey::CommandPaneResizeUpDescription => "将当前分屏向上调整",
        UiTextKey::CommandPaneResizeDownTitle => "向下调整面板",
        UiTextKey::CommandPaneResizeDownDescription => "将当前分屏向下调整",
        UiTextKey::CommandPaneRenameTitle => "重命名面板",
        UiTextKey::CommandPaneRenameDescription => "重命名当前面板",
        UiTextKey::CommandPanePaletteTitle => "打开面板切换器",
        UiTextKey::CommandPanePaletteDescription => "聚焦当前标签页中的面板",
        UiTextKey::CommandLayoutDefaultEditTitle => "编辑默认布局",
        UiTextKey::CommandLayoutDefaultEditDescription => "编辑全局默认布局 TOML",
        UiTextKey::CommandLayoutDefaultResetTitle => "重置默认布局",
        UiTextKey::CommandLayoutDefaultResetDescription => "将全局默认布局重置为内置模板",
        UiTextKey::CommandLayoutDefaultReloadTitle => "重新加载默认布局",
        UiTextKey::CommandLayoutDefaultReloadDescription => "从磁盘重新加载全局默认布局",
        UiTextKey::CommandLayoutProjectEditTitle => "编辑项目布局",
        UiTextKey::CommandLayoutProjectEditDescription => "编辑当前项目的有效布局来源",
        UiTextKey::CommandLayoutSaveCurrentTitle => "保存当前布局",
        UiTextKey::CommandLayoutSaveCurrentDescription => "将当前布局保存为本地覆盖",
        UiTextKey::CommandLayoutExportProjectConfigTitle => "导出项目布局",
        UiTextKey::CommandLayoutExportProjectConfigDescription => "将当前布局写入项目配置",
        UiTextKey::CommandLayoutResetLocalOverrideTitle => "重置个人布局覆盖",
        UiTextKey::CommandLayoutResetLocalOverrideDescription => "删除当前项目的个人布局覆盖",
        UiTextKey::CommandLayoutOpenFileTitle => "打开布局文件",
        UiTextKey::CommandLayoutOpenFileDescription => "显示当前项目的布局文件路径",
        UiTextKey::LayoutEditorPlaceholder => "编辑布局 TOML…",
        UiTextKey::LayoutEditorReadFailed => "读取布局 TOML 失败",
        UiTextKey::LayoutEditorParseFailed => "解析布局 TOML 失败",
        UiTextKey::LayoutEditorValidationFailed => "布局 TOML 无效",
        UiTextKey::LayoutEditorPersonalInvalid => "个人布局无效",
        UiTextKey::LayoutEditorRequiresPatchMode => "个人布局编辑器要求 mode = \"patch\"",
        UiTextKey::LayoutEditorRequiresReplaceMode => "个人布局编辑器要求 mode = \"replace\"",
        UiTextKey::LayoutEditorSaveFailed => "保存布局 TOML 失败",
        UiTextKey::BarsEditorTitle => "编辑栏位 TOML",
        UiTextKey::BarsEditorPlaceholder => "编辑窗口栏与状态栏 TOML…",
        UiTextKey::BarsEditorParseFailed => "解析栏位 TOML 失败",
        UiTextKey::BarsEditorValidationFailed => "栏位 TOML 无效",
        UiTextKey::BarsEditorSaveFailed => "保存栏位 TOML 失败",
        UiTextKey::BarsEditorComponents => "组件",
        UiTextKey::BarsEditorSearchComponents => "搜索组件…",
        UiTextKey::BarsEditorInsertInto => "插入到",
        UiTextKey::BarsEditorPreview => "预览",
        UiTextKey::BarsEditorPreviewUnavailable => "修正 TOML 后可预览此布局。",
        UiTextKey::BarsEditorWindow => "窗口栏",
        UiTextKey::BarsEditorStatus => "状态栏",
        UiTextKey::BarsEditorPresets => "草稿预设",
        UiTextKey::BarsEditorPresetRecommended => "推荐",
        UiTextKey::BarsEditorPresetMinimal => "精简",
        UiTextKey::BarsEditorPresetDevelopment => "开发",
        UiTextKey::BarsEditorPresetAgent => "Agent",
        UiTextKey::BarsEditorRestoreDefaults => "恢复默认值",
        UiTextKey::BarsEditorCatalogSample => "示例",
        UiTextKey::BarsEditorCatalogScope => "范围",
        UiTextKey::BarsEditorPreviewStatusDisabled => "此草稿已禁用状态栏。",
        UiTextKey::BarsEditorPreviewUnavailableModules => "当前不可用的组件",
        UiTextKey::BarsEditorCategoryProject => "项目",
        UiTextKey::BarsEditorCategoryEditorVim => "编辑器与 Vim",
        UiTextKey::BarsEditorCategoryTerminal => "终端",
        UiTextKey::BarsEditorCategoryAgent => "Agent",
        UiTextKey::BarsEditorCategoryPerformance => "性能",
        UiTextKey::BarsEditorCategoryLayoutActions => "布局与操作",
        UiTextKey::BarsEditorReasonNoProject => "打开项目后即可显示此组件。",
        UiTextKey::BarsEditorReasonNoEditor => "打开编辑器后即可显示此组件。",
        UiTextKey::BarsEditorReasonNoCodeEditor => "此组件仅在代码编辑器中可用。",
        UiTextKey::BarsEditorReasonNoSelection => "选择文本后即可显示此组件。",
        UiTextKey::BarsEditorReasonNoTerminal => "打开终端后即可显示此组件。",
        UiTextKey::BarsEditorReasonTerminalNotExited => "活动终端退出后才会显示此组件。",
        UiTextKey::BarsEditorReasonTerminalSizeUnavailable => "活动终端尚未报告其尺寸。",
        UiTextKey::BarsEditorReasonNoAgent => "活动终端不是 Agent 面板。",
        UiTextKey::BarsEditorReasonAgentNotWaiting => "活动 Agent 当前未等待输入。",
        UiTextKey::BarsEditorReasonAgentModelUnavailable => "活动 Agent 尚未报告所用模型。",
        UiTextKey::BarsEditorReasonNoActiveChildren => "活动 Agent 没有正在运行或等待的子任务。",
        UiTextKey::BarsEditorReasonNoGit => "所选项目没有 Git 状态。",
        UiTextKey::BarsEditorReasonGitClean => "工作区没有文件变更。",
        UiTextKey::BarsEditorReasonNoSsh => "所选项目未通过 SSH 连接。",
        UiTextKey::BarsEditorReasonVimDisabled => "启用 Vim 模式后即可显示此组件。",
        UiTextKey::BarsEditorReasonNoVimDetail => "当前 Vim 模式没有额外信息。",
        UiTextKey::BarsEditorReasonNoVimKeys => "当前 Vim 模式没有待处理按键。",
        UiTextKey::BarsEditorReasonEditorClean => "活动编辑器没有未保存的更改。",
        UiTextKey::BarsEditorReasonNoDiagnostics => "活动编辑器没有诊断信息。",
        UiTextKey::BarsEditorReasonPerformanceUnavailable => "性能指标暂时不可用。",
        UiTextKey::BarsEditorReasonNoUpdate => "没有可用更新。",
        UiTextKey::BarsComponentProjectNameName => "项目名称",
        UiTextKey::BarsComponentProjectNameDescription => "所选项目的显示名称。",
        UiTextKey::BarsComponentProjectPathName => "项目路径",
        UiTextKey::BarsComponentProjectPathDescription => "所选项目的本地或远程路径。",
        UiTextKey::BarsComponentGitBranchName => "Git 分支",
        UiTextKey::BarsComponentGitBranchDescription => "所选项目的当前分支。",
        UiTextKey::BarsComponentGitChangesName => "Git 更改",
        UiTextKey::BarsComponentGitChangesDescription => "所选项目的工作区更改摘要。",
        UiTextKey::BarsComponentSshName => "SSH 连接",
        UiTextKey::BarsComponentSshDescription => "远程连接名称和状态。",
        UiTextKey::BarsComponentUpdateName => "更新状态",
        UiTextKey::BarsComponentUpdateDescription => "可用的应用更新状态。",
        UiTextKey::BarsComponentActiveItemName => "活动项目",
        UiTextKey::BarsComponentActiveItemDescription => "活动文件或终端的标题和图标。",
        UiTextKey::BarsComponentSurfaceName => "工作区",
        UiTextKey::BarsComponentSurfaceDescription => "当前工作区，如编辑器或终端。",
        UiTextKey::BarsComponentVimModeName => "Vim 模式",
        UiTextKey::BarsComponentVimModeDescription => "当前 Vim 模式。",
        UiTextKey::BarsComponentVimDetailName => "Vim 详情",
        UiTextKey::BarsComponentVimDetailDescription => "当前 Vim 模式的额外状态。",
        UiTextKey::BarsComponentVimKeysName => "Vim 按键",
        UiTextKey::BarsComponentVimKeysDescription => "待处理的 Vim 按键序列。",
        UiTextKey::BarsComponentEditorLanguageName => "编辑器语言",
        UiTextKey::BarsComponentEditorLanguageDescription => "活动代码编辑器的语言。",
        UiTextKey::BarsComponentEditorPositionName => "编辑器位置",
        UiTextKey::BarsComponentEditorPositionDescription => "活动编辑器中的光标行和列。",
        UiTextKey::BarsComponentEditorDirtyName => "未保存的更改",
        UiTextKey::BarsComponentEditorDirtyDescription => "活动编辑器是否有未保存的更改。",
        UiTextKey::BarsComponentEditorDiagnosticsName => "编辑器诊断",
        UiTextKey::BarsComponentEditorDiagnosticsDescription => "活动编辑器中的错误、警告和信息。",
        UiTextKey::BarsComponentEditorSelectionName => "编辑器选择",
        UiTextKey::BarsComponentEditorSelectionDescription => "所选字符数和行数。",
        UiTextKey::BarsComponentEditorTabSizeName => "制表符宽度",
        UiTextKey::BarsComponentEditorTabSizeDescription => "活动编辑器使用的缩进宽度。",
        UiTextKey::BarsComponentEditorWrapName => "自动换行",
        UiTextKey::BarsComponentEditorWrapDescription => "活动编辑器是否启用软换行。",
        UiTextKey::BarsComponentTerminalTitleName => "终端标题",
        UiTextKey::BarsComponentTerminalTitleDescription => "活动终端的标题。",
        UiTextKey::BarsComponentTerminalStateName => "终端状态",
        UiTextKey::BarsComponentTerminalStateDescription => "活动终端是否正在运行。",
        UiTextKey::BarsComponentTerminalExitName => "终端退出",
        UiTextKey::BarsComponentTerminalExitDescription => "活动终端停止后的退出状态。",
        UiTextKey::BarsComponentTerminalSizeName => "终端尺寸",
        UiTextKey::BarsComponentTerminalSizeDescription => "活动终端的列数和行数。",
        UiTextKey::BarsComponentAgentStateName => "Agent 状态",
        UiTextKey::BarsComponentAgentStateDescription => "活动 Agent 的当前状态。",
        UiTextKey::BarsComponentAgentWaitingName => "Agent 等待",
        UiTextKey::BarsComponentAgentWaitingDescription => "活动 Agent 等待输入时显示。",
        UiTextKey::BarsComponentAgentModelName => "Agent 模型",
        UiTextKey::BarsComponentAgentModelDescription => "活动 Agent 报告的模型。",
        UiTextKey::BarsComponentAgentChildrenName => "Agent 子任务",
        UiTextKey::BarsComponentAgentChildrenDescription => "活动 Agent 的活动子任务数量。",
        UiTextKey::BarsComponentAgentStateDurationName => "Agent 状态时长",
        UiTextKey::BarsComponentAgentStateDurationDescription => {
            "活动 Agent 在当前状态已持续的时间。"
        }
        UiTextKey::BarsComponentProjectsCountName => "项目数",
        UiTextKey::BarsComponentProjectsCountDescription => "已打开项目的数量。",
        UiTextKey::BarsComponentTerminalsCountName => "终端数",
        UiTextKey::BarsComponentTerminalsCountDescription => "已打开终端的数量。",
        UiTextKey::BarsComponentTabsCountName => "标签页数",
        UiTextKey::BarsComponentTabsCountDescription => "已打开标签页的数量。",
        UiTextKey::BarsComponentEditorsCountName => "编辑器数",
        UiTextKey::BarsComponentEditorsCountDescription => "已打开编辑器的数量。",
        UiTextKey::BarsComponentAppCpuName => "应用 CPU",
        UiTextKey::BarsComponentAppCpuDescription => "yttt 使用的 CPU。",
        UiTextKey::BarsComponentAppMemoryName => "应用内存",
        UiTextKey::BarsComponentAppMemoryDescription => "yttt 使用的内存。",
        UiTextKey::BarsComponentSystemCpuName => "系统 CPU",
        UiTextKey::BarsComponentSystemCpuDescription => "系统总 CPU 使用率。",
        UiTextKey::BarsComponentSystemMemoryName => "系统内存",
        UiTextKey::BarsComponentSystemMemoryDescription => "系统总内存使用量。",
        UiTextKey::BarsComponentCommandPaletteName => "命令面板",
        UiTextKey::BarsComponentCommandPaletteDescription => "打开命令面板的按钮。",
        UiTextKey::BarsComponentSettingsName => "设置",
        UiTextKey::BarsComponentSettingsDescription => "打开设置的按钮。",
        UiTextKey::BarsComponentSpaceName => "空白",
        UiTextKey::BarsComponentSpaceDescription => "固定空白；在模板中修改数量。",
        UiTextKey::BarsComponentTextName => "文本",
        UiTextKey::BarsComponentTextDescription => "字面文本；在模板中替换示例文本。",
        UiTextKey::BarsComponentIconName => "图标",
        UiTextKey::BarsComponentIconDescription => "内置图标；在模板中替换示例图标名。",
        UiTextKey::BarsComponentSeparatorName => "分隔符",
        UiTextKey::BarsComponentSeparatorDescription => "栏位组件之间的竖直分隔符。",
        UiTextKey::CommandPaletteOpenTitle => "打开命令面板",
        UiTextKey::CommandPaletteOpenDescription => "搜索并运行命令",
        UiTextKey::CommandSettingsOpenTitle => "打开设置",
        UiTextKey::SettingsWindowTitle => "设置",
        UiTextKey::CommandSettingsOpenDescription => "配置 YTTT",
        UiTextKey::CommandSettingsKeybindingsTitle => "打开快捷键文件",
        UiTextKey::CommandSettingsKeybindingsDescription => "打开或创建可编辑的快捷键 TOML",
        UiTextKey::CommandSettingsNotificationsTitle => "切换通知",
        UiTextKey::CommandSettingsNotificationsDescription => "切换 Agent 退出的系统通知",
        UiTextKey::CommandApplicationQuitTitle => "退出 yttt",
        UiTextKey::CommandApplicationQuitDescription => "停止所有 Host 资源并退出应用",
        UiTextKey::GitBranchesLoading => "正在加载 Git 分支…",
        UiTextKey::GitBranchSwitchFailed => "无法切换 Git 分支",
        UiTextKey::GitBranchLocal => "本地分支",
        UiTextKey::GitBranchRemote => "远程分支",
        UiTextKey::GitBranchAlreadyActive => "当前分支",
        UiTextKey::GitDiffTitle => "Git 更改",
        UiTextKey::GitDiffLoading => "正在加载工作区差异…",
        UiTextKey::GitDiffClean => "工作区没有更改",
        UiTextKey::GitDiffFile => "个文件",
        UiTextKey::GitDiffFiles => "个文件",
        UiTextKey::GitDiffFilesHeading => "更改的文件",
        UiTextKey::GitDiffWhitespace => "忽略空白",
        UiTextKey::GitDiffUnified => "统一",
        UiTextKey::GitDiffSplit => "拆分",
        UiTextKey::GitDiffUnstaged => "未暂存",
        UiTextKey::GitDiffStaged => "已暂存",
        UiTextKey::GitDiffCloseHint => "关闭",
        UiTextKey::GitDiffStageHint => "已暂存 / 未暂存",
        UiTextKey::GitDiffSplitHint => "拆分 / 统一",
        UiTextKey::GitDiffNavigateHint => "切换文件",
        UiTextKey::GitDiffCopyHint => "复制",
        UiTextKey::GitDiffBinaryUnavailable => "二进制文件无法显示差异",
        UiTextKey::GitDiffSourceHead => "HEAD",
        UiTextKey::GitDiffSourceIndex => "暂存区",
        UiTextKey::GitDiffSourceWorkingTree => "工作区",
        UiTextKey::CloseProjectTitle => "关闭项目？",
        UiTextKey::CloseProjectBody => "正在运行的终端进程会被停止。",
        UiTextKey::UnsavedChangesTitle => "未保存的更改",
        UiTextKey::CloseWindowTitle => "关闭 YTTT？",
        UiTextKey::UnsavedFileSingular => "个未保存文件",
        UiTextKey::UnsavedFilePlural => "个未保存文件",
        UiTextKey::RunningProcessSingular => "个运行中进程",
        UiTextKey::RunningProcessPlural => "个运行中进程",
        UiTextKey::SaveAllAndContinue => "全部保存并继续",
        UiTextKey::Discard => "丢弃",
        UiTextKey::DiscardAndContinue => "丢弃并继续",
        UiTextKey::CloseSaveFailureGuidance => "请修复保存错误，或丢弃更改后继续。",
        UiTextKey::Cancel => "取消",
        UiTextKey::Retry => "重试",
        UiTextKey::CloseProjectAction => "关闭项目",
        UiTextKey::RenameTabTitle => "重命名标签页",
        UiTextKey::RenameTabAction => "重命名",
        UiTextKey::RenameTabHint => "回车重命名，Esc 取消",
        UiTextKey::OpenNotificationTarget => "打开",
        UiTextKey::SettingsSearchPlaceholder => "搜索设置...",
        UiTextKey::SettingsGroupGeneral => "通用",
        UiTextKey::SettingsGroupGeneralDescription => "应用行为和通知",
        UiTextKey::SettingsGroupAppearance => "外观",
        UiTextKey::SettingsGroupAppearanceDescription => "界面字体以及界面和终端主题",
        UiTextKey::SettingsGroupLanguages => "语言",
        UiTextKey::SettingsGroupLanguagesDescription => "代码语言检测和语言服务器默认值",
        UiTextKey::SettingsGroupEditor => "编辑器",
        UiTextKey::SettingsGroupEditorDescription => "文本编辑和项目文件树行为",
        UiTextKey::SettingsGroupTerminal => "终端",
        UiTextKey::SettingsGroupTerminalDescription => "Shell、字体和终端运行默认值",
        UiTextKey::SettingsGroupAgent => "Agent",
        UiTextKey::SettingsGroupAgentDescription => "主 Agent 与本地会话扫描",
        UiTextKey::SettingsGroupPermissions => "权限",
        UiTextKey::SettingsGroupPermissionsDescription => {
            "管理文件、通知和开发工作流所需的系统访问权限"
        }
        UiTextKey::SettingsGroupProjectLayout => "项目布局",
        UiTextKey::SettingsGroupProjectLayoutDescription => "项目布局文件和 TOML 编辑",
        UiTextKey::SettingsGroupDefaultLayout => "默认布局",
        UiTextKey::SettingsGroupDefaultLayoutDescription => "没有项目配置时继承的全局布局",
        UiTextKey::SettingsGroupKeybindings => "快捷键",
        UiTextKey::SettingsGroupKeybindingsDescription => "键盘快捷键和冲突诊断",
        UiTextKey::SettingsNoSearchResults => "没有找到设置",
        UiTextKey::SettingsNoSearchResultsDescription => "请尝试其他设置名称、说明或分类。",
        UiTextKey::SettingsTargetDevice => "本机偏好",
        UiTextKey::SettingsTargetHost => "运行环境",
        UiTextKey::SettingsTargetProject => "项目",
        UiTextKey::SettingsScopeDevice => "本机偏好",
        UiTextKey::SettingsScopeHost => "运行环境 · 此环境共用",
        UiTextKey::SettingsScopeProject => "仅此项目",
        UiTextKey::SettingsLocalEnvironment => "本机",
        UiTextKey::SettingsRemoteEnvironment => "远程",
        UiTextKey::SettingsDevelopmentEnvironment => "开发配置",
        UiTextKey::SettingsTestEnvironment => "测试配置",
        UiTextKey::SettingsEnvironmentDetails => "配置详情",
        UiTextKey::SettingsLastKnownValue => "最后已知值",
        UiTextKey::SettingsCustomizeProject => "为此项目单独设置",
        UiTextKey::SettingsCollapseOverride => "收起项目设置",
        UiTextKey::SettingsSavePending => "正在保存设置…",
        UiTextKey::SettingsApplyImmediate => "立即生效",
        UiTextKey::SettingsApplyNewSession => "在新会话中生效",
        UiTextKey::SettingsApplyReopenFile => "重新打开文件时生效",
        UiTextKey::SettingsSourceHost => "使用运行环境默认值",
        UiTextKey::SettingsSourceProject => "项目覆盖",
        UiTextKey::SettingsReadOnlyObserver => {
            "其他客户端正在控制此环境，运行环境和项目设置为只读；本机偏好仍可编辑。"
        }
        UiTextKey::SettingsReadOnlyPreparingTransfer => {
            "控制权正在交接，运行环境和项目设置暂时只读；本机偏好仍可编辑。"
        }
        UiTextKey::SettingsReadOnlyDisconnected => {
            "运行环境未连接，其配置暂时只读；本机偏好仍可编辑。"
        }
        UiTextKey::SettingsReadOnlyProjectConfig => {
            "此环境的项目配置为只读。请使用允许写入项目配置的环境进行项目定制。"
        }
        UiTextKey::SettingsProjectTargetUnavailable => "请打开并选中一个项目以编辑项目覆盖。",
        UiTextKey::SettingsProjectLoading => "正在加载项目设置…",
        UiTextKey::SettingsProjectLoadFailed => "项目设置加载失败",
        UiTextKey::SettingsRemotePermissionsLocal => "权限在本地管理",
        UiTextKey::SettingsRemotePermissionsLocalDescription => {
            "此页面管理本设备的桌面权限，不管理远程 Host。"
        }
        UiTextKey::SettingsCopyDraft => "复制草稿",
        UiTextKey::RemoteDeviceDraftRecoveryAvailable => " · 本机恢复草稿可用",
        UiTextKey::RemoteRecoverDeviceDrafts => "恢复本机草稿",
        UiTextKey::ProfileControlDisconnected => "连接已断开 · 未同步",
        UiTextKey::ProfileControlRemoteIdentity => "远程 · {host}",
        UiTextKey::ProfileControlLocalIdentity => "本地",
        UiTextKey::ProfileControlRestoreFailed => " · 恢复失败",
        UiTextKey::ProfileControlLost => " · 未同步，编辑已保留",
        UiTextKey::ProfileControlTransferring => " · 正在交接",
        UiTextKey::ProfileControlRestoring => " · 正在恢复",
        UiTextKey::ProfileControlReadOnly => " · 只读",
        UiTextKey::ProfileControlSaving => " · 正在保存",
        UiTextKey::ProfileControlRecoverWorkspace => "恢复工作区…",
        UiTextKey::ProfileControlRecoverFullWorkspace => "恢复完整工作区…",
        UiTextKey::ProfileControlWorkspaceHistory => "工作区历史…",
        UiTextKey::WorkspaceHistoryTitle => "打开工作区",
        UiTextKey::WorkspaceHistorySearch => "搜索工作区名称、路径或 ID…",
        UiTextKey::WorkspaceHistoryEmpty => "没有已关闭的工作区",
        UiTextKey::WorkspaceHistoryLoading => "正在加载工作区历史…",
        UiTextKey::WorkspaceHistoryLoadFailed => "无法加载工作区历史",
        UiTextKey::WorkspaceHistoryUnnamed => "工作区",
        UiTextKey::WorkspaceHistoryNoProjects => "没有保存的项目",
        UiTextKey::WorkspaceHistoryMetadataUnavailable => "无法读取工作区信息",
        UiTextKey::WorkspaceHistorySaved => "最近保存",
        UiTextKey::WorkspaceHistoryUnknownDate => "保存时间未知",
        UiTextKey::WorkspaceHistoryToday => "今天",
        UiTextKey::WorkspaceHistoryYesterday => "昨天",
        UiTextKey::WorkspaceHistoryNavigation => "↑ ↓ 选择    ↵ 打开    Esc 关闭",
        UiTextKey::WorkspaceHistoryMore => "工作区操作",
        UiTextKey::WorkspaceHistoryRemove => "从历史移除",
        UiTextKey::WorkspaceHistoryRemoveTitle => "从历史移除此工作区？",
        UiTextKey::WorkspaceHistoryRemoveWarning => {
            "此操作移除保存的布局和未保存草稿，不删除项目文件或 Agent 历史。"
        }
        UiTextKey::SettingsUseHostDefault => "恢复使用环境默认值",
        UiTextKey::SettingsSectionApplicationInteraction => "应用与交互",
        UiTextKey::SettingsSectionStartupNotifications => "启动与通知",
        UiTextKey::SettingsSectionNewTabs => "新标签页",
        UiTextKey::SettingsSectionConnections => "连接",
        UiTextKey::SettingsSectionUpdates => "更新",
        UiTextKey::SettingsSectionWindow => "窗口",
        UiTextKey::SettingsSectionInterface => "界面",
        UiTextKey::SettingsSectionThemes => "主题",
        UiTextKey::SettingsSectionBars => "窗口栏与状态栏",
        UiTextKey::SettingsSectionAdvanced => "高级",
        UiTextKey::SettingsSectionDetectionDefaults => "检测与默认值",
        UiTextKey::SettingsSectionLanguageServices => "语言服务",
        UiTextKey::SettingsSectionTypography => "字体与排版",
        UiTextKey::SettingsSectionEditingBehavior => "编辑行为",
        UiTextKey::SettingsSectionSaving => "保存",
        UiTextKey::SettingsSectionProjectPanels => "项目面板",
        UiTextKey::SettingsSectionShell => "Shell",
        UiTextKey::SettingsSectionEnvironment => "环境变量",
        UiTextKey::SettingsSectionScrolling => "滚动",
        UiTextKey::SettingsSectionCursorMouse => "光标与鼠标",
        UiTextKey::SettingsSectionCompatibility => "兼容性与剪贴板",
        UiTextKey::SettingsSectionAgentOverview => "Agent 概览",
        UiTextKey::SettingsSectionProviders => "会话提供方",
        UiTextKey::SettingsSectionCorePermissions => "通知与开发访问",
        UiTextKey::SettingsSectionOptionalPermissions => "可选集成",
        UiTextKey::SettingsSectionBackgroundHost => "后台 Host",
        UiTextKey::SettingsLoginStartup => "登录时启动 Host",
        UiTextKey::SettingsLoginStartupDescription => {
            "登录后保持当前 profile 的 Host 可用。仅支持持久化的生产 profile；macOS 还需打包的应用。启动项不会添加网络监听地址或秘密。"
        }
        UiTextKey::SettingsLoginStartupEnabled => "已启用",
        UiTextKey::SettingsLoginStartupDisabled => "已停用",
        UiTextKey::SettingsLoginStartupRequiresApproval => "需要系统批准",
        UiTextKey::SettingsLoginStartupUnavailable => "不可用",
        UiTextKey::SettingsLoginStartupChecking => "检测中...",
        UiTextKey::SettingsLoginStartupConfirmTitle => "登录时启动 yttt Host？",
        UiTextKey::SettingsLoginStartupConfirmDescription => {
            "这会注册当前用户的后台启动项，让终端、项目、SSH 和 Agent 资源在没有打开 yttt 窗口时仍可用。它不会开放网络监听。你可以随时在此处或系统启动项设置中关闭。"
        }
        UiTextKey::SettingsLoginStartupConfirmAction => "启用登录启动",
        UiTextKey::RemoteAccessTitle => "远程访问此电脑",
        UiTextKey::RemoteAccessDescription => {
            "通过 TLS 和可选的 TCP 端口转发共享当前 Host。连接信息可访问此 profile 的全部工作区，请保密。关闭工作窗口不会停止共享。"
        }
        UiTextKey::RemoteAccessListening => "正在监听",
        UiTextKey::RemoteAccessStarting => "正在启动…",
        UiTextKey::RemoteAccessStopping => "正在停止…",
        UiTextKey::RemoteAccessFailed => "无法启动共享",
        UiTextKey::RemoteAccessDisabled => "远程访问已关闭",
        UiTextKey::RemoteAccessUnavailable => "正在等待本机 Host",
        UiTextKey::RemoteAccessAddress => "监听地址",
        UiTextKey::RemoteAccessClients => "已连接设备",
        UiTextKey::RemoteAccessNoClients => "尚无远程设备连接",
        UiTextKey::RemoteAccessStreams => "条数据流",
        UiTextKey::RemoteAccessControlOwner => "当前控制端",
        UiTextKey::RemoteAccessEnable => "开启",
        UiTextKey::RemoteAccessDisable => "关闭",
        UiTextKey::RemoteAccessApply => "应用地址",
        UiTextKey::RemoteAccessCopy => "复制连接信息",
        UiTextKey::RemoteAccessReset => "重置凭据",
        UiTextKey::RemoteAccessReclaim => "收回控制",
        UiTextKey::RemoteAccessDisconnect => "断开全部远程会话",
        UiTextKey::RemoteAccessRefresh => "刷新",
        UiTextKey::RemoteAccessConfirmTitle => "更改远程访问？",
        UiTextKey::RemoteAccessConfirmDescription => {
            "此操作可能断开远程会话。非回环地址会向网络开放端口。强制收回使用最后确认状态，原控制端未发布的编辑可能丢失。请确认已了解这些后果。"
        }
        UiTextKey::RemoteAccessConfirm => "确认",
        UiTextKey::RemoteAccessNotReady => "请等待全部工作窗口完成状态发布，再开启远程访问。",
        UiTextKey::RemoteAccessCopied => "已复制 Base64 连接码，包含地址和访问密钥，请妥善保密。",
        UiTextKey::ConnectExistingHost => "连接已有 yttt",
        UiTextKey::ExistingHostDescription => {
            "粘贴另一台电脑导出的连接码，通过 TLS 直接连接，无需 SSH 或部署 Server。"
        }
        UiTextKey::ConnectionCodeLabel => "连接码",
        UiTextKey::ConnectionCodePlaceholder => "粘贴 Base64 连接码，自动填入地址和凭据",
        UiTextKey::ConnectionCodeSecurity => {
            "包含地址、证书和访问密钥。Base64 不加密，请仅分享给可信设备。"
        }
        UiTextKey::ConnectionCodeInvalid => "连接码无效，请从另一台电脑的远程访问设置重新复制。",
        UiTextKey::ConnectionAddressLabel => "连接地址",
        UiTextKey::ConnectionAddressHint => {
            "从连接码自动填入。跨电脑访问或使用端口转发时，请改为可达的主机:端口。"
        }
        UiTextKey::ConnectionAddressInvalid => "请输入主机:端口或 [IPv6]:端口，不要填写 URL。",
        UiTextKey::ConnectionRemember => "记住凭据",
        UiTextKey::ConnectionRemembered => "已选择记住凭据（系统安全存储）",
        UiTextKey::RemoteConnections => "远程连接",
        UiTextKey::RemoteAdd => "添加连接",
        UiTextKey::RemoteAddSsh => "SSH 连接",
        UiTextKey::RemoteAddHost => "网络 Host",
        UiTextKey::RemoteEdit => "编辑",
        UiTextKey::RemoteSaveConnect => "保存并连接",
        UiTextKey::RemoteEmpty => "暂无远程连接。添加 SSH 连接，或粘贴 Host 连接码即可开始。",
        UiTextKey::RemoteConnecting => "正在连接…",
        UiTextKey::RemoteConnectionName => "连接名称",
        UiTextKey::RemoteCredentialsRequired => "补充连接凭据",
        UiTextKey::RemoteRepairSshAgent => {
            "请解锁 SSH Agent 并加载此连接使用的密钥，然后重试。此认证方式不使用密码。"
        }
        UiTextKey::RemotePassphraseTemporary => "私钥口令仅供当前 Client 使用，不会保存。",
        UiTextKey::RemoteConnectTitle => "连接远程工作区",
        UiTextKey::RemoteConnectPreparing => "正在准备连接…",
        UiTextKey::RemoteConnectFailed => "无法建立连接",
        UiTextKey::RemoteConnectCancel => "取消连接",
        UiTextKey::RemoteConnectDetails => "连接详情",
        UiTextKey::RemoteConnectHideDetails => "收起详情",
        UiTextKey::RemoteConnectErrorDetails => "技术详情",
        UiTextKey::RemoteConnectVerifyingHost => "正在验证 Host 证书和环境…",
        UiTextKey::RemoteConnectCheckingServer => "正在检查远程平台和 Server 安装…",
        UiTextKey::RemoteConnectStartingHost => "正在启动或连接远程 Host…",
        UiTextKey::RemoteConnectSshDisconnected => "SSH 连接已断开",
        UiTextKey::RemoteConnectSshConnecting => "正在建立 SSH 连接…",
        UiTextKey::RemoteConnectSshAuthenticating => "正在验证 SSH 凭据…",
        UiTextKey::RemoteConnectSshConnected => "SSH 已连接",
        UiTextKey::RemoteConnectSshReconnecting => "正在重新连接 SSH…",
        UiTextKey::RemoteConnectSshRejected => "已拒绝 SSH 主机密钥",
        UiTextKey::RemoteConnectContinuing => "正在继续连接…",
        UiTextKey::RemoteConnectTakingControl => "正在移交工作区控制权…",
        UiTextKey::RemoteConnectObserving => "正在打开只读工作区…",
        UiTextKey::RemoteTakeoverTitle => "在此继续远程工作区？",
        UiTextKey::RemoteTakeoverDescription => {
            "在此继续会将全部工作区的控制权移交到本窗口。原客户端会保存状态并转为只读，正在运行的终端和任务不会停止。"
        }
        UiTextKey::RemoteTakeoverForceTitle => "原客户端未能保存状态",
        UiTextKey::RemoteTakeoverForceDescription => {
            "强制继续只会恢复最后确认的状态，原客户端尚未发布的编辑可能丢失。取消可将控制权保留在原客户端。"
        }
        UiTextKey::RemoteObserveOnly => "只读打开",
        UiTextKey::RemoteContinueHere => "在此继续",
        UiTextKey::RemoteForceContinue => "强制继续",
        UiTextKey::RemoteCancelTransfer => "取消接管",
        UiTextKey::RemoteProfile => "配置档",
        UiTextKey::RemoteWorkspaceCount => "工作区数量",
        UiTextKey::RemoteEnvironment => "环境标识",
        UiTextKey::RemoteClient => "当前客户端",
        UiTextKey::RemoteUnowned => "暂无控制端",
        UiTextKey::RemoteKeyAlgorithm => "密钥算法",
        UiTextKey::RemoteManageLocally => "请在本地 yttt 窗口中管理远程连接和凭据。",
        UiTextKey::RemoteRecordMissing => "此连接记录已不存在。",
        UiTextKey::RemoteInvalidPort => "SSH 端口必须介于 1 和 65535 之间。",
        UiTextKey::RemoteInvalidRoot => "远程根目录必须是绝对 POSIX 路径。",
        UiTextKey::RemoteLaunchFailed => "无法打开远程工作区",
        UiTextKey::RemoteWrongHostCode => "此连接码属于另一个 Host。",
        UiTextKey::RemoteCredentialStoreFailed => {
            "系统凭据库不可用，未保存任何明文凭据。关闭「记住凭据」可仅保存连接"
        }
        UiTextKey::RemoteRuntimeUnavailable => "Host 运行时不可用。",
        UiTextKey::RemoteDirectoryUnexpected => "Host 返回了非预期的目录响应。",
        UiTextKey::SettingsPermissionNotifications => "系统通知",
        UiTextKey::SettingsPermissionNotificationsDescription => {
            "允许 yttt 在后台发送 Agent 任务完成或等待处理的提醒。"
        }
        UiTextKey::SettingsPermissionFileSystem => "受保护的文件访问",
        UiTextKey::SettingsPermissionFileSystemDescription => {
            "受保护项目文件无法访问时，请检查实际启动终端或 Agent 的 Host 或应用。在 macOS 上打开「完全磁盘访问」；普通项目访问无需开启。"
        }
        UiTextKey::SettingsPermissionDeveloperTools => "开发者工具",
        UiTextKey::SettingsPermissionDeveloperToolsDescription => {
            "仅在开发工作流确实需要时配置系统开发者控制；不是普通终端或 Agent 使用的前提。"
        }
        UiTextKey::SettingsPermissionAccessibility => "辅助功能",
        UiTextKey::SettingsPermissionAccessibilityDescription => {
            "仅在工作流需要控制其他应用或模拟输入时使用。"
        }
        UiTextKey::SettingsPermissionScreenCapture => "屏幕录制",
        UiTextKey::SettingsPermissionScreenCaptureDescription => {
            "仅在工作流需要捕获屏幕或应用窗口时使用。"
        }
        UiTextKey::SettingsPermissionStatus => "授权状态",
        UiTextKey::SettingsPermissionStatusDescription => {
            "检测本机桌面应用的授权，不代表独立 Host、终端或 Agent 的实际访问能力。从系统设置返回时刷新，也可手动刷新。系统授权不是 profile 偏好设置。"
        }
        UiTextKey::SettingsPermissionRefresh => "刷新",
        UiTextKey::SettingsPermissionChecking => "检测中...",
        UiTextKey::SettingsPermissionGranted => "已授权",
        UiTextKey::SettingsPermissionNotDetermined => "尚未申请",
        UiTextKey::SettingsPermissionDenied => "未授权",
        UiTextKey::SettingsPermissionUnavailable => "暂时无法获取状态",
        UiTextKey::SettingsPermissionRequiresSystemSettings => "需在系统设置确认",
        UiTextKey::SettingsPermissionRequest => "申请",
        UiTextKey::SettingsPermissionRequesting => "申请中...",
        UiTextKey::SettingsPermissionOpeningSettings => "正在打开设置...",
        UiTextKey::SettingsPermissionManage => "管理",
        UiTextKey::SettingsPermissionOpenSystemSettings => "打开系统设置",
        UiTextKey::SettingsPermissionManagedBySystem => "由桌面环境管理",
        UiTextKey::SettingsPermissionRequestedWhenNeeded => "使用时申请",
        UiTextKey::SettingsPermissionNotRequired => "无需单独授权",
        UiTextKey::SettingsSectionCurrentLayout => "当前布局",
        UiTextKey::SettingsSectionLayoutActions => "布局操作",
        UiTextKey::SettingsSectionDangerZone => "危险操作",
        UiTextKey::SettingsSectionConfiguration => "配置",
        UiTextKey::SettingsSectionVim => "Vim",
        UiTextKey::SettingsSectionDiagnostics => "诊断",
        UiTextKey::SettingsSectionBindings => "快捷键列表",
        UiTextKey::SettingsSectionKeymapProfile => "快捷键配置",
        UiTextKey::SettingsKeybindingProfileBase => "基础",
        UiTextKey::SettingsKeybindingProfileBaseDescription => {
            "非 Vim 模式使用的快捷键；无冲突时由 Vim 自动继承。"
        }
        UiTextKey::SettingsKeybindingProfileVim => "Vim",
        UiTextKey::SettingsKeybindingProfileVimDescription => {
            "Vim 的有效快捷键：继承基础快捷键，并叠加模态覆盖。"
        }
        UiTextKey::SettingsKeybindingOriginInherited => "继承",
        UiTextKey::SettingsKeybindingOriginBuiltin => "内置",
        UiTextKey::SettingsKeybindingOriginUser => "用户",
        UiTextKey::SettingsKeybindingShadowed => "已遮蔽",
        UiTextKey::SettingsKeybindingPrefixConflict => "前缀冲突",
        UiTextKey::SettingsAgentPrimary => "主 Agent",
        UiTextKey::SettingsAgentPrimaryDescription => "引导时选择、用于扫描会话的 Agent。",
        UiTextKey::AgentInitializationPreparing => "正在 Host 上准备 Agent 钩子…",
        UiTextKey::AgentInitializationControllerRequired => {
            "此 Agent 尚未启动。控制客户端必须先初始化它。"
        }
        UiTextKey::AgentInitializationRetry => "重试 Agent 初始化",
        UiTextKey::SettingsAgentSessions => "会话列表",
        UiTextKey::SettingsAgentSessionsDescription => {
            "显示 Agent 会话标签页；默认仅包含主 Agent。"
        }
        UiTextKey::SettingsAgentSessionProviderDescription => {
            "同时扫描此 Agent，并在全局会话列表中显示。"
        }
        UiTextKey::SettingsLanguage => "语言",
        UiTextKey::SettingsLanguageDescription => "应用显示语言。",
        UiTextKey::SettingsSelectLanguage => "选择语言",
        UiTextKey::SettingsSystemNotifications => "系统通知",
        UiTextKey::SettingsSystemNotificationsDescription => "Agent 终端任务完成或失败时通知。",
        UiTextKey::SettingsUpdates => "版本更新",
        UiTextKey::SettingsUpdatesDescription => "查看当前版本并检查可用的新版本。",
        UiTextKey::SettingsAutoCheckUpdates => "自动检查更新",
        UiTextKey::SettingsAutoCheckUpdatesDescription => {
            "应用启动后每天最多检查一次，不阻塞应用启动。"
        }
        UiTextKey::SettingsCheckForUpdates => "检查更新",
        UiTextKey::SettingsCheckingForUpdates => "正在检查...",
        UiTextKey::SettingsUpdateAvailable => "发现新版本",
        UiTextKey::SettingsUpdateAvailableDescription => "新版本已可下载。",
        UiTextKey::SettingsUpToDate => "YTTT 已是最新版本",
        UiTextKey::SettingsUpdateCheckFailed => "无法检查更新",
        UiTextKey::SettingsDownloadUpdate => "下载更新",
        UiTextKey::SettingsRestoreLastSession => "启动时恢复上次会话",
        UiTextKey::SettingsRestoreLastSessionDescription => {
            "恢复已保存的项目、终端与文件标签页和分屏布局。重连存活进程，重建 shell 并恢复 Agent 会话，不重放其他命令。"
        }
        UiTextKey::PerformanceProjects => "项目",
        UiTextKey::PerformanceTerminals => "终端",
        UiTextKey::PerformanceTabs => "标签页",
        UiTextKey::PerformanceEditors => "编辑器",
        UiTextKey::PerformanceCpu => "应用 CPU",
        UiTextKey::PerformanceMemory => "应用内存",
        UiTextKey::PerformanceSystemCpu => "系统 CPU",
        UiTextKey::PerformanceSystemMemory => "系统内存",
        UiTextKey::SettingsNewTabCommandPicker => "新标签页命令选择框",
        UiTextKey::SettingsNewTabCommandPickerDescription => "点击新建标签页按钮时显示命令选择框。",
        UiTextKey::SettingsNewTabCommands => "新标签页命令",
        UiTextKey::SettingsNewTabCommandsDescription => {
            "配置命令选择框中的命令；命令将在当前项目目录中运行。"
        }
        UiTextKey::SettingsNewTabCommandPlaceholder => "例如 lazygit、nvim .、codex",
        UiTextKey::SettingsAddCommand => "添加命令",
        UiTextKey::SettingsWindowEffect => "窗口背景效果",
        UiTextKey::SettingsWindowEffectDescription => "选择无效果、透明或磨砂玻璃主窗口。",
        UiTextKey::SettingsWindowEffectNone => "无",
        UiTextKey::SettingsWindowEffectTransparent => "透明",
        UiTextKey::SettingsWindowEffectBlurred => "磨砂玻璃",
        UiTextKey::SettingsWindowOpacity => "主窗口不透明度",
        UiTextKey::SettingsWindowOpacityDescription => {
            "统一控制标题栏、侧边栏、编辑器和终端等主页面表面。0.00 完全透明，1.00 完全不透明；仅用于透明和磨砂玻璃模式。"
        }
        UiTextKey::SettingsUiFontFamily => "界面字体",
        UiTextKey::SettingsUiFontFamilyDescription => "YTTT 外壳、面板和控件使用的字体。",
        UiTextKey::SettingsUiFontSize => "界面字号",
        UiTextKey::SettingsUiFontSizeDescription => "界面基础字号（像素）；控件将按比例缩放。",
        UiTextKey::SettingsUiLineHeight => "界面行高",
        UiTextKey::SettingsUiLineHeightDescription => "界面文字的行高倍数。",
        UiTextKey::SettingsUiTheme => "界面主题",
        UiTextKey::SettingsUiThemeDescription => "用于 YTTT 外壳、面板和控件的主题。",
        UiTextKey::SettingsUiStyle => "界面风格",
        UiTextKey::SettingsUiStyleDescription => {
            "全局切换控件的形状、密度、边框、悬浮效果和层级阴影。"
        }
        UiTextKey::SettingsIconTheme => "图标主题",
        UiTextKey::SettingsIconThemeDescription => {
            "使用已安装的兼容 Zed 图标主题显示文件、文件夹和编辑器图标。"
        }
        UiTextKey::SettingsTerminalTheme => "终端主题",
        UiTextKey::SettingsTerminalThemeDescription => "可选的终端颜色覆盖。",
        UiTextKey::SettingsSearchTheme => "搜索主题...",
        UiTextKey::SettingsBarLeft => "左侧",
        UiTextKey::SettingsBarCenter => "中间",
        UiTextKey::SettingsBarRight => "右侧",
        UiTextKey::SettingsEditSettingsToml => "编辑设置 TOML",
        UiTextKey::SettingsEditSettingsTomlDescription => "打开应用设置文件进行高级编辑。",
        UiTextKey::SettingsEditBarsToml => "栏位",
        UiTextKey::SettingsEditBarsTomlDescription => "在专用编辑器中编辑窗口栏与状态栏模板。",
        UiTextKey::SettingsShowPath => "显示路径",
        UiTextKey::SettingsThemesDirectory => "主题目录",
        UiTextKey::SettingsThemesDirectoryDescription => "打开用户主题 TOML 文件所在目录。",
        UiTextKey::SettingsImportZedThemes => "导入 Zed 主题",
        UiTextKey::SettingsImportZedThemesDescription => {
            "从已安装的 Zed 扩展中探测并导入兼容的界面主题和图标主题。"
        }
        UiTextKey::SettingsImportZedThemesAction => "导入",
        UiTextKey::SettingsImportZedThemesComplete => "Zed 主题导入完成",
        UiTextKey::SettingsImportZedThemesNone => "未探测到兼容的 Zed 主题。",
        UiTextKey::SettingsImportZedThemesImported => "已导入",
        UiTextKey::SettingsImportZedThemesConflictHint => "对于已导入的主题：",
        UiTextKey::SettingsImportZedThemesSkipExisting => "跳过导入",
        UiTextKey::SettingsImportZedThemesOverwriteExisting => "覆盖导入",
        UiTextKey::SettingsLanguageDetection => "语言检测",
        UiTextKey::SettingsLanguageDetectionDescription => {
            "根据文件名、扩展名和首行检测代码编辑器语言。"
        }
        UiTextKey::SettingsDefaultCodeLanguage => "默认代码语言",
        UiTextKey::SettingsDefaultCodeLanguageDescription => "自动检测关闭或未知时使用的兜底语言。",
        UiTextKey::SettingsSupportedLanguages => "支持的语言",
        UiTextKey::SettingsSupportedLanguagesDescription => "代码编辑器内置可用语言。",
        UiTextKey::SettingsLanguageServer => "语言服务器",
        UiTextKey::SettingsLanguageServerDescription => "为后续诊断和补全预留 LSP 启动入口。",
        UiTextKey::SettingsLanguageServerCommand => "语言服务器命令",
        UiTextKey::SettingsLanguageServerCommandDescription => "为默认语言服务器集成预留的命令。",
        UiTextKey::SettingsSearchCodeLanguage => "搜索语言...",
        UiTextKey::SettingsEditorFontFamily => "字体",
        UiTextKey::SettingsEditorFontFamilyDescription => "项目文件编辑器使用的字体。",
        UiTextKey::SettingsEditorFontSize => "字体大小",
        UiTextKey::SettingsEditorFontSizeDescription => "编辑器字体的像素大小。",
        UiTextKey::SettingsEditorLineHeight => "行高",
        UiTextKey::SettingsEditorLineHeightDescription => "编辑器行高倍数。",
        UiTextKey::SettingsEditorTabSize => "Tab 宽度",
        UiTextKey::SettingsEditorTabSizeDescription => {
            "每个 Tab 对应的空格数；已打开的文件需要重新打开后生效。"
        }
        UiTextKey::SettingsEditorSoftWrap => "自动换行",
        UiTextKey::SettingsEditorSoftWrapDescription => "按编辑器可用宽度换行显示长行。",
        UiTextKey::SettingsEditorLineNumbers => "行号",
        UiTextKey::SettingsEditorLineNumbersDescription => "在项目文件中显示行号。",
        UiTextKey::SettingsVimMode => "Vim 模式",
        UiTextKey::SettingsVimModeDescription => {
            "选择唯一的 Vim 模式：全局模式统一编辑器、终端、项目树、设置页、面板、标签和命令面板；仅编辑器模式只在项目编辑器中启用 Vim；禁用则完全关闭。"
        }
        UiTextKey::SettingsVimQuickStart => "Vim 快速上手",
        UiTextKey::SettingsVimQuickStartDescription => {
            "全局模式：Ctrl-[ 或 Escape 进入普通模式；i 返回插入模式或终端输入；Ctrl-W H/J/K/L 在窗格、工作区与项目树之间切换焦点；项目树中 j/k 移动条目，h/l 折叠、展开或打开；gt/gT 切换标签；Leader 命令用于打开文件、命令面板和标签。"
        }
        UiTextKey::SettingsVimModeScope => "模式与界面",
        UiTextKey::SettingsVimModeScopeDescription => {
            "普通 · 插入 · 可视 · 操作符 · 终端 · 设置 · 命令面板 · 项目树"
        }
        UiTextKey::SettingsVimLeader => "Vim Leader 键",
        UiTextKey::SettingsVimLeaderDescription => {
            "默认为 Space。在 keybindings.toml 顶层设置 leader = \"...\"，并在多键序列中使用 <leader>；修改后会自动重载。"
        }
        UiTextKey::SettingsEditorAutosave => "自动保存",
        UiTextKey::SettingsEditorAutosaveDescription => "选择编辑文件的自动保存时机。",
        UiTextKey::SettingsEditorAutosaveOff => "关闭",
        UiTextKey::SettingsEditorAutosaveOnFocusChange => "焦点切换时",
        UiTextKey::SettingsEditorAutosaveAfterDelay => "延迟后",
        UiTextKey::SettingsEditorAutosaveDelay => "自动保存延迟",
        UiTextKey::SettingsEditorAutosaveDelayDescription => "延迟自动保存使用的毫秒数。",
        UiTextKey::SettingsProjectPanelDefaultOpen => "默认打开文件树",
        UiTextKey::SettingsProjectPanelDefaultOpenDescription => "首次打开项目时显示文件树。",
        UiTextKey::SettingsProjectPanelShowHidden => "显示隐藏文件",
        UiTextKey::SettingsProjectPanelShowHiddenDescription => {
            "在项目文件树中包含隐藏文件和目录。"
        }
        UiTextKey::SettingsProjectPanelWidth => "文件树宽度",
        UiTextKey::SettingsProjectPanelWidthDescription => "右侧项目文件树的默认宽度。",
        UiTextKey::SettingsProjectSidebarWidth => "项目侧栏宽度",
        UiTextKey::SettingsProjectSidebarWidthDescription => "左侧已打开项目侧栏的宽度。",
        UiTextKey::SettingsDefaultShell => "默认 Shell",
        UiTextKey::SettingsDefaultShellDescription => {
            "供 Shell 执行模式的布局面板和新终端标签页使用。"
        }
        UiTextKey::SettingsSelectShell => "选择 Shell",
        UiTextKey::SettingsCustomShell => "自定义 Shell",
        UiTextKey::SettingsCustomShellDescription => {
            "将可执行文件路径或命令名称添加到已保存的 Shell 列表。"
        }
        UiTextKey::SettingsCustomShellPlaceholder => "Shell 路径或命令",
        UiTextKey::SettingsAddShell => "添加",
        UiTextKey::SettingsEnvironmentVariables => "全局环境变量",
        UiTextKey::SettingsEnvironmentVariablesDescription => {
            "自动注入每个新启动的本地或 SSH Shell 与命令；设置同名变量会覆盖原值。"
        }
        UiTextKey::SettingsEnvironmentNamePlaceholder => "变量名",
        UiTextKey::SettingsEnvironmentValuePlaceholder => "值",
        UiTextKey::SettingsSetEnvironmentVariable => "添加或更新",
        UiTextKey::SettingsFontFamily => "字体族",
        UiTextKey::SettingsFontFamilyDescription => {
            "终端字体族。System default 使用内置的 Hack Nerd Font Mono。"
        }
        UiTextKey::SettingsSearchFont => "搜索字体...",
        UiTextKey::SettingsFontSize => "字体大小",
        UiTextKey::SettingsFontSizeDescription => "终端字体像素大小。",
        UiTextKey::SettingsLineHeight => "行高",
        UiTextKey::SettingsLineHeightDescription => "终端行高倍数。",
        UiTextKey::SettingsPadding => "内边距",
        UiTextKey::SettingsPaddingDescription => "终端面板内部留白。",
        UiTextKey::SettingsScrollback => "回滚行数",
        UiTextKey::SettingsScrollbackDescription => "保留在内存中的终端行数。",
        UiTextKey::SettingsScrollbar => "滚动条",
        UiTextKey::SettingsScrollbarDescription => "在终端面板中显示细滚动指示器。",
        UiTextKey::SettingsTerminalCursorShape => "光标形状",
        UiTextKey::SettingsTerminalCursorShapeDescription => "设置终端光标的形状。",
        UiTextKey::SettingsTerminalCursorBlinking => "光标闪烁",
        UiTextKey::SettingsTerminalCursorBlinkingDescription => "终端获得焦点时闪烁光标。",
        UiTextKey::SettingsTerminalHideMouseWhenTyping => "输入时隐藏鼠标",
        UiTextKey::SettingsTerminalHideMouseWhenTypingDescription => {
            "向终端发送键盘输入时隐藏指针。"
        }
        UiTextKey::SettingsTerminalCopyOnSelect => "选中即复制",
        UiTextKey::SettingsTerminalCopyOnSelectDescription => {
            "完成终端文本选择后自动复制，无需执行复制操作。"
        }
        UiTextKey::SettingsTerminalOsc52Policy => "OSC 52 剪贴板访问",
        UiTextKey::SettingsTerminalOsc52PolicyDescription => "控制终端应用对系统剪贴板的访问权限。",
        UiTextKey::SettingsTerminalKittyKeyboard => "Kitty 键盘协议",
        UiTextKey::SettingsTerminalKittyKeyboardDescription => "允许终端应用协商 Kitty 键盘协议。",
        UiTextKey::SettingsLayoutSource => "布局来源",
        UiTextKey::SettingsLayoutSourceDescription => "当前项目布局来源。",
        UiTextKey::SettingsOpenProjectFirst => "请先打开项目",
        UiTextKey::SettingsSaveCurrentLayout => "保存当前布局",
        UiTextKey::SettingsSaveCurrentLayoutDescription => "将当前布局保存为应用本地覆盖。",
        UiTextKey::SettingsExportProjectLayout => "导出项目布局",
        UiTextKey::SettingsExportProjectLayoutDescription => "将当前布局写入项目配置。",
        UiTextKey::SettingsEditLayoutToml => "编辑布局 TOML",
        UiTextKey::SettingsEditLayoutTomlDescription => "编辑所选项目的布局文件。",
        UiTextKey::SettingsDefaultLayoutPath => "默认布局文件",
        UiTextKey::SettingsDefaultLayoutPathDescription => {
            "没有项目配置时使用的全局默认布局 TOML。"
        }
        UiTextKey::SettingsEditDefaultLayout => "编辑默认布局 TOML",
        UiTextKey::SettingsEditDefaultLayoutDescription => "编辑全局默认布局文件。",
        UiTextKey::SettingsReloadDefaultLayout => "重新加载默认布局",
        UiTextKey::SettingsReloadDefaultLayoutDescription => "从磁盘重新加载全局默认布局。",
        UiTextKey::SettingsResetDefaultLayout => "重置默认布局",
        UiTextKey::SettingsResetDefaultLayoutDescription => "使用内置模板替换全局默认布局。",
        UiTextKey::SettingsEditKeybindingsToml => "编辑快捷键 TOML",
        UiTextKey::SettingsEditKeybindingsTomlDescription => "打开用户快捷键文件。",
        UiTextKey::KeybindingSettingsVimPreviousTitle => "设置 Vim：上一组",
        UiTextKey::KeybindingSettingsVimPreviousDescription => "选择上一个可见的设置组。",
        UiTextKey::KeybindingSettingsVimNextTitle => "设置 Vim：下一组",
        UiTextKey::KeybindingSettingsVimNextDescription => "选择下一个可见的设置组。",
        UiTextKey::KeybindingSettingsVimFirstTitle => "设置 Vim：第一组",
        UiTextKey::KeybindingSettingsVimFirstDescription => "选择第一个可见的设置组。",
        UiTextKey::KeybindingSettingsVimLastTitle => "设置 Vim：最后一组",
        UiTextKey::KeybindingSettingsVimLastDescription => "选择最后一个可见的设置组。",
        UiTextKey::KeybindingVimNormalTitle => "进入 Vim 普通模式",
        UiTextKey::KeybindingVimNormalDescription => "将当前 yttt 界面切换回 Vim 普通模式。",
        UiTextKey::KeybindingVimInsertTitle => "进入 Vim 插入模式",
        UiTextKey::KeybindingVimInsertDescription => "允许在当前 yttt 界面直接输入文本。",
        UiTextKey::KeybindingVimTerminalTitle => "进入 Vim 终端模式",
        UiTextKey::KeybindingVimTerminalDescription => "将当前终端切换回直接进程输入模式。",
        UiTextKey::KeybindingProjectsVimPreviousTitle => "项目 Vim：上一个项目",
        UiTextKey::KeybindingProjectsVimPreviousDescription => "选择上一个已打开的项目。",
        UiTextKey::KeybindingProjectsVimNextTitle => "项目 Vim：下一个项目",
        UiTextKey::KeybindingProjectsVimNextDescription => "选择下一个已打开的项目。",
        UiTextKey::KeybindingProjectsVimFirstTitle => "项目 Vim：第一个项目",
        UiTextKey::KeybindingProjectsVimFirstDescription => "选择第一个已打开的项目。",
        UiTextKey::KeybindingProjectsVimLastTitle => "项目 Vim：最后一个项目",
        UiTextKey::KeybindingProjectsVimLastDescription => "选择最后一个已打开的项目。",
        UiTextKey::KeybindingProjectTreeVimUpTitle => "项目树 Vim：上一个条目",
        UiTextKey::KeybindingProjectTreeVimUpDescription => "移动到上一个可见的项目树条目。",
        UiTextKey::KeybindingProjectTreeVimDownTitle => "项目树 Vim：下一个条目",
        UiTextKey::KeybindingProjectTreeVimDownDescription => "移动到下一个可见的项目树条目。",
        UiTextKey::KeybindingProjectTreeVimLeftTitle => "项目树 Vim：折叠或父级",
        UiTextKey::KeybindingProjectTreeVimLeftDescription => "折叠当前目录或选择其父级。",
        UiTextKey::KeybindingProjectTreeVimRightTitle => "项目树 Vim：展开或打开",
        UiTextKey::KeybindingProjectTreeVimRightDescription => {
            "展开目录、选择其第一个子项，或打开文件。"
        }
        UiTextKey::KeybindingProjectTreeVimOpenTitle => "项目树 Vim：打开",
        UiTextKey::KeybindingProjectTreeVimOpenDescription => "打开文件或切换所选目录。",
        UiTextKey::KeybindingProjectTreeVimToggleTitle => "项目树 Vim：切换目录展开状态",
        UiTextKey::KeybindingProjectTreeVimToggleDescription => "展开或折叠所选项目树目录。",
        UiTextKey::KeybindingProjectTreeVimFirstTitle => "项目树 Vim：第一个条目",
        UiTextKey::KeybindingProjectTreeVimFirstDescription => "选择第一个可见的项目树条目。",
        UiTextKey::KeybindingProjectTreeVimLastTitle => "项目树 Vim：最后一个条目",
        UiTextKey::KeybindingProjectTreeVimLastDescription => "选择最后一个可见的项目树条目。",
        UiTextKey::KeybindingTerminalViToggleTitle => "切换终端 Vi 模式",
        UiTextKey::KeybindingTerminalViToggleDescription => "进入或离开终端 Vi 模式。",
        UiTextKey::KeybindingTerminalViLeaveTitle => "退出终端 Vi 模式",
        UiTextKey::KeybindingTerminalViLeaveDescription => "将终端切换回普通输入模式。",
        UiTextKey::KeybindingTerminalViSelectionTitle => "切换终端 Vi 选区",
        UiTextKey::KeybindingTerminalViSelectionDescription => "开始或清除终端 Vi 选区。",
        UiTextKey::KeybindingTerminalViYankTitle => "复制终端 Vi 选区",
        UiTextKey::KeybindingTerminalViYankDescription => "复制并清除终端 Vi 选区。",
        UiTextKey::KeybindingTerminalViMoveLeftTitle => "终端 Vi：向左移动",
        UiTextKey::KeybindingTerminalViMoveLeftDescription => "将终端 Vi 光标向左移动。",
        UiTextKey::KeybindingTerminalViMoveDownTitle => "终端 Vi：向下移动",
        UiTextKey::KeybindingTerminalViMoveDownDescription => "将终端 Vi 光标向下移动。",
        UiTextKey::KeybindingTerminalViMoveUpTitle => "终端 Vi：向上移动",
        UiTextKey::KeybindingTerminalViMoveUpDescription => "将终端 Vi 光标向上移动。",
        UiTextKey::KeybindingTerminalViMoveRightTitle => "终端 Vi：向右移动",
        UiTextKey::KeybindingTerminalViMoveRightDescription => "将终端 Vi 光标向右移动。",
        UiTextKey::KeybindingTerminalViMoveFirstTitle => "终端 Vi：第一列",
        UiTextKey::KeybindingTerminalViMoveFirstDescription => "移动到终端第一列。",
        UiTextKey::KeybindingTerminalViMoveLastTitle => "终端 Vi：最后一列",
        UiTextKey::KeybindingTerminalViMoveLastDescription => "移动到终端最后一列。",
        UiTextKey::KeybindingTerminalViMoveFirstOccupiedTitle => "终端 Vi：第一个非空列",
        UiTextKey::KeybindingTerminalViMoveFirstOccupiedDescription => "移动到终端第一个非空列。",
        UiTextKey::KeybindingTerminalViMoveHighTitle => "终端 Vi：视口顶部",
        UiTextKey::KeybindingTerminalViMoveHighDescription => "移动到终端视口顶部。",
        UiTextKey::KeybindingTerminalViMoveMiddleTitle => "终端 Vi：视口中部",
        UiTextKey::KeybindingTerminalViMoveMiddleDescription => "移动到终端视口中部。",
        UiTextKey::KeybindingTerminalViMoveLowTitle => "终端 Vi：视口底部",
        UiTextKey::KeybindingTerminalViMoveLowDescription => "移动到终端视口底部。",
        UiTextKey::KeybindingTerminalViMoveWordLeftTitle => "终端 Vi：上一个单词",
        UiTextKey::KeybindingTerminalViMoveWordLeftDescription => "移动到终端中的上一个单词。",
        UiTextKey::KeybindingTerminalViMoveWordRightTitle => "终端 Vi：下一个单词",
        UiTextKey::KeybindingTerminalViMoveWordRightDescription => "移动到终端中的下一个单词。",
        UiTextKey::KeybindingTerminalViMoveWordEndTitle => "终端 Vi：下一个单词的末尾",
        UiTextKey::KeybindingTerminalViMoveWordEndDescription => "移动到终端中下一个单词的末尾。",
        UiTextKey::KeybindingTerminalViMoveBracketTitle => "终端 Vi：匹配括号",
        UiTextKey::KeybindingTerminalViMoveBracketDescription => "移动到匹配的终端括号。",
        UiTextKey::KeybindingTerminalViMoveParagraphUpTitle => "终端 Vi：上一个段落",
        UiTextKey::KeybindingTerminalViMoveParagraphUpDescription => "移动到终端中的上一个段落。",
        UiTextKey::KeybindingTerminalViMoveParagraphDownTitle => "终端 Vi：下一个段落",
        UiTextKey::KeybindingTerminalViMoveParagraphDownDescription => "移动到终端中的下一个段落。",
        UiTextKey::SettingsKeybindingDiagnostics => "快捷键诊断",
        UiTextKey::SettingsKeybindingDiagnosticsDescription => "显示无效命令和快捷键冲突。",
        UiTextKey::SettingsNoKeybindingConflicts => "没有快捷键冲突",
        UiTextKey::SettingsUnbound => "未绑定",
        UiTextKey::SettingsConflict => "冲突",
        UiTextKey::SettingsKeybindingDialogTitle => "编辑快捷键",
        UiTextKey::SettingsKeybindingRecorderPrompt => "请按下按键序列",
        UiTextKey::SettingsKeybindingRecorderHint => "继续按键可组成序列；单独按修饰键不会被录制。",
        UiTextKey::SettingsKeybindingCurrentBindings => "当前快捷键",
        UiTextKey::SettingsKeybindingNoBindings => "尚未设置快捷键",
        UiTextKey::SettingsKeybindingReplace => "替换快捷键",
        UiTextKey::SettingsKeybindingRecording => "正在录制",
        UiTextKey::SettingsKeybindingFinishRecording => "完成录制",
        UiTextKey::SettingsKeybindingRemove => "移除",
        UiTextKey::SettingsAddKeybindingAlternative => "添加备用键",
        UiTextKey::SettingsClearKeybindings => "清除",
        UiTextKey::SettingsConflictingKeybinding => "快捷键冲突",
        UiTextKey::SettingsInvalidCommandId => "无效的命令 ID",
        UiTextKey::SettingsSave => "保存",
        UiTextKey::SettingsExport => "导出",
        UiTextKey::SettingsOpen => "打开",
        UiTextKey::SettingsEdit => "编辑",
        UiTextKey::SettingsReset => "重置",
        UiTextKey::SettingsDelete => "删除",
        UiTextKey::StatusSystemNotificationsEnabled => "系统通知：已启用",
        UiTextKey::StatusSystemNotificationsDisabled => "系统通知：已禁用",
        UiTextKey::StatusErrorContext => "错误",
        UiTextKey::StatusWarningContext => "警告",
        UiTextKey::StatusKeybindingsFile => "快捷键文件",
        UiTextKey::StatusLayoutFile => "布局文件",
        UiTextKey::StatusSettingsFile => "设置文件",
        UiTextKey::StatusBarsFile => "栏位配置文件",
        UiTextKey::StatusThemesDirectory => "主题目录",
        UiTextKey::SshConnections => "SSH 连接",
        UiTextKey::RemoteServices => "远程服务管理",
        UiTextKey::RemoteServicesDescription => "管理远程连接和此电脑的远程访问。",
        UiTextKey::SshConnectionsDescription => "管理 SSH 端点、凭据和远程项目根目录。",
        UiTextKey::SshNoConnections => "尚未配置 SSH 连接。",
        UiTextKey::SshNewConnection => "新建连接",
        UiTextKey::SshEditConnection => "编辑连接",
        UiTextKey::SshConnectionName => "名称",
        UiTextKey::SshCommand => "SSH 命令",
        UiTextKey::SshCommandPlaceholder => "ssh -p 22 -i ~/.ssh/id_ed25519 user@example.com",
        UiTextKey::SshCommandParse => "应用命令",
        UiTextKey::SshHost => "主机",
        UiTextKey::SshPort => "端口",
        UiTextKey::SshUser => "用户",
        UiTextKey::SshRemoteRoot => "远程根目录",
        UiTextKey::SshAuthentication => "认证方式",
        UiTextKey::SshIdentityFile => "私钥文件",
        UiTextKey::SshKeyPassphrase => "私钥口令",
        UiTextKey::SshPassword => "密码",
        UiTextKey::SshPasswordPromptTitle => "需要 SSH 密码",
        UiTextKey::SshPasswordPromptDescription => "请输入此连接的登录密码，并选择是否保存。",
        UiTextKey::SshPasswordRequired => "请输入密码后继续。",
        UiTextKey::SshPasswordRejected => "服务器拒绝了该密码，请检查后重试。",
        UiTextKey::SshPasswordUnavailable => "已保存的密码不可用，请重新输入。",
        UiTextKey::SshRememberPassword => "将密码保存到系统凭据库",
        UiTextKey::SshAuthAuto => "自动",
        UiTextKey::SshAuthAgent => "SSH Agent",
        UiTextKey::SshAuthPassword => "密码",
        UiTextKey::SshAuthPrivateKey => "私钥",
        UiTextKey::SshConnect => "连接",
        UiTextKey::SshDisconnect => "断开",
        UiTextKey::SshDeleteConnection => "删除连接",
        UiTextKey::SshHostKeyTitle => "验证 SSH 主机密钥",
        UiTextKey::SshHostKeyDescription => "发送凭据前，请确认服务器指纹。",
        UiTextKey::SshHostKeyChangedTitle => "SSH 主机密钥已更改",
        UiTextKey::SshHostKeyChangedDescription => {
            "已保存的主机密钥与服务器不匹配。请核对新指纹后再覆盖。"
        }
        UiTextKey::SshHostKeySavedFingerprint => "已保存的指纹",
        UiTextKey::SshHostKeyReceivedFingerprint => "收到的指纹",
        UiTextKey::SshHostKeyReject => "拒绝",
        UiTextKey::SshHostKeyTrustOnce => "仅本次信任",
        UiTextKey::SshHostKeyTrustAndSave => "信任并保存",
        UiTextKey::SshHostKeyReplace => "覆盖已保存密钥",
        UiTextKey::SshConnectionSaved => "SSH 连接已保存",
        UiTextKey::SshConnectionDeleteInUse => "删除连接前，请先关闭使用该连接的远程项目。",
        UiTextKey::SshOpenRemoteProject => "打开远程项目",
        UiTextKey::SshProjectRecent => "最近远程项目",
        UiTextKey::SshProjectPath => "远程路径",
        UiTextKey::SshProjectOpenCurrentFolder => "打开当前文件夹",
        UiTextKey::SshProjectLoadingDirectory => "正在加载远程目录…",
        UiTextKey::SshProjectEmptyDirectory => "当前目录没有子目录。",
        UiTextKey::SshProjectNoMatchingDirectories => "没有匹配的目录。",
        UiTextKey::SshProjectConnectionMissing => "保存的 SSH 连接已不存在。",
        UiTextKey::SshProjectConnectionFailed => "无法连接 SSH 主机。",
        UiTextKey::SshRuntimeUnavailable => "SSH 运行时不可用。",
        UiTextKey::SshDisconnected => "未连接",
        UiTextKey::SshConnecting => "正在连接…",
        UiTextKey::SshVerifyingHostKey => "正在验证主机密钥…",
        UiTextKey::SshAuthenticating => "正在认证…",
        UiTextKey::SshConnected => "已连接",
        UiTextKey::SshReconnecting => "正在重连…",
        UiTextKey::SshFailed => "连接失败",
        UiTextKey::EditorVimMoveLeftTitle => "编辑器 Vim：向左移动",
        UiTextKey::EditorVimMoveLeftDescription => "将 Vim 光标向左移动。",
        UiTextKey::EditorVimMoveRightTitle => "编辑器 Vim：向右移动",
        UiTextKey::EditorVimMoveRightDescription => "将 Vim 光标向右移动。",
        UiTextKey::EditorVimMoveDownTitle => "编辑器 Vim：向下移动",
        UiTextKey::EditorVimMoveDownDescription => "将 Vim 光标向下移动一个逻辑行。",
        UiTextKey::EditorVimMoveUpTitle => "编辑器 Vim：向上移动",
        UiTextKey::EditorVimMoveUpDescription => "将 Vim 光标向上移动一个逻辑行。",
        UiTextKey::EditorVimMoveDisplayDownTitle => "编辑器 Vim：向下移动一个显示行",
        UiTextKey::EditorVimMoveDisplayDownDescription => "将 Vim 光标向下移动一个显示行。",
        UiTextKey::EditorVimMoveDisplayUpTitle => "编辑器 Vim：向上移动一个显示行",
        UiTextKey::EditorVimMoveDisplayUpDescription => "将 Vim 光标向上移动一个显示行。",
        UiTextKey::EditorVimMoveNextWordStartTitle => "编辑器 Vim：下一个单词",
        UiTextKey::EditorVimMoveNextWordStartDescription => "移动到下一个单词的开头。",
        UiTextKey::EditorVimMoveNextWordEndTitle => "编辑器 Vim：下一个词尾",
        UiTextKey::EditorVimMoveNextWordEndDescription => "移动到下一个单词的末尾。",
        UiTextKey::EditorVimMovePreviousWordStartTitle => "编辑器 Vim：上一个单词",
        UiTextKey::EditorVimMovePreviousWordStartDescription => "移动到上一个单词的开头。",
        UiTextKey::EditorVimMoveLineStartTitle => "编辑器 Vim：行首",
        UiTextKey::EditorVimMoveLineStartDescription => "移动到当前行的开头。",
        UiTextKey::EditorVimMoveFirstNonWhitespaceTitle => "编辑器 Vim：首个非空白字符",
        UiTextKey::EditorVimMoveFirstNonWhitespaceDescription => "移动到首个非空白字符。",
        UiTextKey::EditorVimMoveLineEndTitle => "编辑器 Vim：行尾",
        UiTextKey::EditorVimMoveLineEndDescription => "移动到当前行的末尾。",
        UiTextKey::EditorVimMoveDocumentStartTitle => "编辑器 Vim：文档开头",
        UiTextKey::EditorVimMoveDocumentStartDescription => "移动到文档开头。",
        UiTextKey::EditorVimMoveDocumentEndTitle => "编辑器 Vim：文档末尾",
        UiTextKey::EditorVimMoveDocumentEndDescription => "移动到文档末尾。",
        UiTextKey::EditorVimCountZeroTitle => "编辑器 Vim：数字 0",
        UiTextKey::EditorVimCountZeroDescription => "输入计数 0，或移动到行首。",
        UiTextKey::EditorVimCountOneTitle => "编辑器 Vim：计数 1",
        UiTextKey::EditorVimCountOneDescription => "将 1 追加到 Vim 计数。",
        UiTextKey::EditorVimCountTwoTitle => "编辑器 Vim：计数 2",
        UiTextKey::EditorVimCountTwoDescription => "将 2 追加到 Vim 计数。",
        UiTextKey::EditorVimCountThreeTitle => "编辑器 Vim：计数 3",
        UiTextKey::EditorVimCountThreeDescription => "将 3 追加到 Vim 计数。",
        UiTextKey::EditorVimCountFourTitle => "编辑器 Vim：计数 4",
        UiTextKey::EditorVimCountFourDescription => "将 4 追加到 Vim 计数。",
        UiTextKey::EditorVimCountFiveTitle => "编辑器 Vim：计数 5",
        UiTextKey::EditorVimCountFiveDescription => "将 5 追加到 Vim 计数。",
        UiTextKey::EditorVimCountSixTitle => "编辑器 Vim：计数 6",
        UiTextKey::EditorVimCountSixDescription => "将 6 追加到 Vim 计数。",
        UiTextKey::EditorVimCountSevenTitle => "编辑器 Vim：计数 7",
        UiTextKey::EditorVimCountSevenDescription => "将 7 追加到 Vim 计数。",
        UiTextKey::EditorVimCountEightTitle => "编辑器 Vim：计数 8",
        UiTextKey::EditorVimCountEightDescription => "将 8 追加到 Vim 计数。",
        UiTextKey::EditorVimCountNineTitle => "编辑器 Vim：计数 9",
        UiTextKey::EditorVimCountNineDescription => "将 9 追加到 Vim 计数。",
        UiTextKey::EditorVimDeleteOperatorTitle => "编辑器 Vim：删除操作符",
        UiTextKey::EditorVimDeleteOperatorDescription => "开始 Vim 删除操作。",
        UiTextKey::EditorVimChangeOperatorTitle => "编辑器 Vim：修改操作符",
        UiTextKey::EditorVimChangeOperatorDescription => "开始 Vim 修改操作。",
        UiTextKey::EditorVimYankOperatorTitle => "编辑器 Vim：复制操作符",
        UiTextKey::EditorVimYankOperatorDescription => "开始 Vim 复制操作。",
        UiTextKey::EditorVimInsertCurrentTitle => "编辑器 Vim：插入",
        UiTextKey::EditorVimInsertCurrentDescription => "在光标处进入插入模式。",
        UiTextKey::EditorVimInsertAfterTitle => "编辑器 Vim：追加",
        UiTextKey::EditorVimInsertAfterDescription => "在光标后进入插入模式。",
        UiTextKey::EditorVimInsertFirstNonWhitespaceTitle => "编辑器 Vim：在缩进处插入",
        UiTextKey::EditorVimInsertFirstNonWhitespaceDescription => {
            "在首个非空白字符处进入插入模式。"
        }
        UiTextKey::EditorVimInsertLineEndTitle => "编辑器 Vim：在行尾追加",
        UiTextKey::EditorVimInsertLineEndDescription => "在当前行末尾进入插入模式。",
        UiTextKey::EditorVimInsertLineBelowTitle => "编辑器 Vim：在下方新建一行",
        UiTextKey::EditorVimInsertLineBelowDescription => "在下方新建一行并进入插入模式。",
        UiTextKey::EditorVimInsertLineAboveTitle => "编辑器 Vim：在上方新建一行",
        UiTextKey::EditorVimInsertLineAboveDescription => "在上方新建一行并进入插入模式。",
        UiTextKey::EditorVimToggleVisualTitle => "编辑器 Vim：切换 Visual 模式",
        UiTextKey::EditorVimToggleVisualDescription => "进入或退出字符选择 Visual 模式。",
        UiTextKey::EditorVimToggleVisualLineTitle => "编辑器 Vim：切换行 Visual 模式",
        UiTextKey::EditorVimToggleVisualLineDescription => "进入或退出整行选择 Visual 模式。",
        UiTextKey::EditorVimDeleteCharactersTitle => "编辑器 Vim：删除字符",
        UiTextKey::EditorVimDeleteCharactersDescription => "删除光标下的字符。",
        UiTextKey::EditorVimSubstituteCharactersTitle => "编辑器 Vim：替换并插入",
        UiTextKey::EditorVimSubstituteCharactersDescription => "删除字符并进入插入模式。",
        UiTextKey::EditorVimReplaceCharactersTitle => "编辑器 Vim：替换字符",
        UiTextKey::EditorVimReplaceCharactersDescription => "替换光标下的字符。",
        UiTextKey::EditorVimPasteAfterTitle => "编辑器 Vim：粘贴到光标后",
        UiTextKey::EditorVimPasteAfterDescription => "粘贴到光标后。",
        UiTextKey::EditorVimPasteBeforeTitle => "编辑器 Vim：粘贴到光标前",
        UiTextKey::EditorVimPasteBeforeDescription => "粘贴到光标前。",
        UiTextKey::EditorVimUndoTitle => "编辑器 Vim：撤销",
        UiTextKey::EditorVimUndoDescription => "撤销上一次编辑。",
        UiTextKey::EditorVimRedoTitle => "编辑器 Vim：重做",
        UiTextKey::EditorVimRedoDescription => "重做上一次编辑。",
        UiTextKey::EditorVimSearchForwardTitle => "编辑器 Vim：搜索",
        UiTextKey::EditorVimSearchForwardDescription => "打开向前搜索。",
        UiTextKey::EditorVimSearchNextTitle => "编辑器 Vim：下一个搜索匹配",
        UiTextKey::EditorVimSearchNextDescription => "移动到下一个搜索匹配。",
        UiTextKey::EditorVimSearchPreviousTitle => "编辑器 Vim：上一个搜索匹配",
        UiTextKey::EditorVimSearchPreviousDescription => "移动到上一个搜索匹配。",
        UiTextKey::EditorVimEscapeTitle => "编辑器 Vim：返回普通模式",
        UiTextKey::EditorVimEscapeDescription => "返回 Vim 普通模式。",
        UiTextKey::SettingsLanguageSystem => "跟随系统",
        UiTextKey::SettingsCursorBlock => "方块",
        UiTextKey::SettingsCursorUnderline => "下划线",
        UiTextKey::SettingsCursorBeam => "竖线",
        UiTextKey::SettingsOptionDisabled => "禁用",
        UiTextKey::SettingsOsc52CopyOnly => "仅允许复制",
        UiTextKey::SettingsOsc52ReadWrite => "允许读取和写入",
        UiTextKey::SettingsVimGlobal => "全局",
        UiTextKey::SettingsVimEditorOnly => "仅编辑器",
        UiTextKey::DialogOk => "确定",
        UiTextKey::DialogContinue => "继续",
        UiTextKey::DesktopRestoreClientsFailed => "无法恢复远程客户端",
        UiTextKey::DesktopQuitTitle => "退出 yttt？",
        UiTextKey::DesktopQuitOwnedHostBody => {
            "此 Host 由桌面端管理。退出将停止网络监听、断开远程客户端，并终止运行中的终端和 Agent 任务。已确认的工作区和草稿仍会保留。"
        }
        UiTextKey::DesktopQuitIndependentHostBody => {
            "此 Host 独立运行，其网络监听、远程客户端、终端和 Agent 任务将继续运行。只会断开当前桌面客户端。"
        }
        UiTextKey::DesktopStopHostAndQuit => "停止 Host 并退出",
        UiTextKey::DesktopDisconnect => "断开桌面端",
        UiTextKey::DesktopPublishTimeout => {
            "部分工作窗口未能完成发布。请保持窗口打开，解决保存错误后再退出。"
        }
        UiTextKey::DesktopUnpublishedTitle => "部分更改未能发布",
        UiTextKey::DesktopUnpublishedBody => {
            "{error}\n未发布的更改可能丢失。Host 上已确认的工作区仍会保留。"
        }
        UiTextKey::DesktopKeepOpen => "保持桌面端打开",
        UiTextKey::DesktopQuitUnpublished => "不发布并退出",
        UiTextKey::DesktopStopFailed => "停止 Host 失败",
        UiTextKey::DesktopStopFailedBody => "桌面端仍保持连接。请检查 Host 状态后重试。",
        UiTextKey::WorkspaceRestoreLoading => "正在恢复工作区…",
        UiTextKey::WorkspaceRestoreFailedBody => "无法恢复已保存的工作区状态。",
        UiTextKey::WorkspaceRestoreFailedTitle => "工作区恢复失败",
        UiTextKey::ProfileForceControlTitle => "强制接管此配置档案？",
        UiTextKey::ProfileContinueTitle => "在此继续使用整个配置档案？",
        UiTextKey::ProfileForceControlBody => {
            "只会恢复最后确认的状态。前一个客户端尚未发布的编辑可能丢失。"
        }
        UiTextKey::ProfileContinueBody => {
            "所有工作区将一起转移。前一个客户端将发布编辑并进入观察模式。运行中的终端进程不会重启。"
        }
        UiTextKey::ProfileUnexpectedControlResponse => "收到非预期的控制权响应",
        UiTextKey::WorkspaceSettingsUnconfirmed => "设置尚未得到 Host 确认",
        UiTextKey::WorkspaceUnsavedTitle => "更改尚未保存",
        UiTextKey::WorkspaceKeepOpen => "保持窗口打开",
        UiTextKey::WorkspaceCloseUnsaved => "不保存并关闭",
        UiTextKey::WorkspaceSettingsPending => {
            "设置仍在等待保存或保存失败；请保持此窗口打开，以便重试或复制草稿"
        }
        UiTextKey::WorkspaceFlushHostUnavailable => "Host 不可用，无法完成远程工作区保存",
        UiTextKey::WorkspaceFlushControlUnavailable => {
            "未获得 Host 的独占控制权，无法在关闭前保存远程工作区更改"
        }
        UiTextKey::WorkspaceFlushIdentityUnavailable => "缺少工作区标识，无法完成远程工作区保存",
        UiTextKey::PersistenceRecoveryRequested => "已请求恢复工作区",
        UiTextKey::PersistenceRecoveryWaitOperation => {
            "请等待当前工作区操作完成，再恢复已保存的恢复副本"
        }
        UiTextKey::PersistenceRecoveryReconcile => {
            "请先协调 Host 工作区状态，再恢复已保存的恢复副本"
        }
        UiTextKey::PersistenceRecoveryPublishFirst => {
            "请先发布当前更改或协调状态，再恢复已保存的完整工作区"
        }
        UiTextKey::PersistenceRecoveryControlUnavailable => "无法获得工作区控制权",
        UiTextKey::PersistenceRecoveryDifferentWorkspace => {
            "已保存的恢复副本属于另一个 Host 工作区"
        }
        UiTextKey::PersistenceRestoreRetryHostUnavailable => "Host 不可用，无法重试工作区恢复",
        UiTextKey::PersistenceRestoreRetryIdentityUnavailable => {
            "缺少 Host 工作区标识，无法重试恢复"
        }
        UiTextKey::PersistenceRestoreIdentityLost => "远程 Host 丢失了工作区标识",
        UiTextKey::PersistencePublishDisconnected => "本地编辑发布前，Host 已断开连接",
        UiTextKey::PersistenceRecoveryObserver => {
            "此客户端处于观察模式，恢复的设备私有草稿将保留在本地"
        }
        UiTextKey::PersistencePublishForcedControl => "Host 控制权已被另一个客户端强制接管",
        UiTextKey::PersistencePublishLostControl => "工作区发布失去了控制权",
        UiTextKey::PersistencePublishEpochChanged => {
            "本地编辑发布前，Host 或工作区的控制权代次已变化"
        }
        UiTextKey::PersistenceRestoreEpochChanged => "此客户端断开期间，远程 Host 的代次已变化",
        UiTextKey::PersistenceDraftReconcileFirst => "请先协调 Host 工作区状态，再恢复设备私有草稿",
        UiTextKey::PersistenceDraftHostUnavailable => "Host 不可用，无法恢复设备私有草稿",
        UiTextKey::PersistenceDraftIdentityUnavailable => "缺少工作区标识，无法恢复设备私有草稿",
        UiTextKey::PersistenceDraftDifferentWorkspace => {
            "设备私有草稿恢复副本属于另一个 Host 工作区"
        }
        UiTextKey::PersistenceDraftInvalidText => "设备私有草稿包含无效文本，无法恢复",
        UiTextKey::PersistenceDraftMissingBody => "缺少文档正文，无法恢复设备私有草稿",
        UiTextKey::PersistenceDraftManifestMismatch => "文档正文与清单不匹配，无法恢复设备私有草稿",
        UiTextKey::PersistenceDraftNoMatchingProjects => {
            "没有设备私有草稿与此 Host 工作区当前打开的项目匹配"
        }
        UiTextKey::PersistenceDraftScopeMismatch => "本地草稿恢复范围与此 Host 工作区不匹配",
        UiTextKey::PersistencePublishWorkspaceChanged => "本地编辑发布前，工作区控制权已变化",
        UiTextKey::PersistenceHostClosedEditedFiles => "另一客户端关闭了本窗口中有未保存编辑的文件",
        UiTextKey::PersistencePublishInterrupted => "发布时连接中断；正在等待协调状态",
        UiTextKey::PersistencePublishStaleResponse => "已忽略过期的工作区发布响应；未重放本地编辑",
        UiTextKey::PersistenceDraftDocumentLimit => "文档草稿超过 6 MiB 的恢复限制",
        UiTextKey::PersistenceDraftWorkspaceLimit => "工作区草稿超过 64 MiB 的恢复限制",
        UiTextKey::PersistenceRemoveHistory => "无法移除工作区历史记录：{detail}",
        UiTextKey::PersistenceOpenInitialProject => "无法打开请求的初始项目：{error}",
        UiTextKey::PersistencePublicationRejected => "远程 Host 拒绝发布工作区：{error}",
        UiTextKey::PersistenceControlUnavailable => "远程工作区控制权不可用：{error}",
        UiTextKey::PersistenceStateRestoreFailed => "无法恢复远程工作区状态：{error}",
        UiTextKey::PersistenceEditorRestoreFailed => "无法恢复远程编辑器布局：{error}",
        UiTextKey::PersistenceEditorFinalizeFailed => "无法完成远程编辑器布局恢复：{error}",
        UiTextKey::PersistenceRecoverDraftsFailed => "无法恢复设备私有草稿：{error}",
        UiTextKey::PersistenceLocalRecoveryUnavailable => "本地草稿恢复不可用：{error}",
        UiTextKey::PersistenceListRecoveryFailed => "无法列出本地草稿恢复副本：{error}",
        UiTextKey::PersistenceReadRecoveryFailed => "无法读取本地草稿恢复副本：{error}",
        UiTextKey::PersistencePrepareRecoveryFailed => {
            "{reason}；未发布的编辑仍保持打开，但无法准备本地恢复快照：{error}"
        }
        UiTextKey::PersistenceRecoveryStorageUnavailable => {
            "{reason}；未发布的编辑仍保持打开，但设备私有恢复存储不可用"
        }
        UiTextKey::PersistenceLocateRecoveryFailed => {
            "{reason}；未发布的编辑仍保持打开，但无法定位设备私有恢复存储：{error}"
        }
        UiTextKey::PersistenceScopeRecoveryFailed => {
            "{reason}；未发布的编辑仍保持打开，但无法确定设备私有恢复存储的范围：{error}"
        }
        UiTextKey::PersistenceRecoverySaved => {
            "{reason}；未发布的编辑已保存到设备私有恢复存储，不会自动替换 Host 工作区"
        }
        UiTextKey::PersistenceRecoveryStorageFailed => {
            "{reason}；未发布的编辑仍保持打开，但设备私有恢复存储失败：{error}"
        }
        UiTextKey::PersistenceCloseSavedWorkspace => "无法关闭已保存的工作区：{detail}",
        UiTextKey::PersistenceUpdateClientRestore => "无法更新远程客户端恢复状态：{error}",
        UiTextKey::PersistenceOperationIdInvalid => "远程工作区操作标识无效：{error}",
        UiTextKey::PersistenceSnapshotInvalid => "远程工作区快照无效；更改仍未保存：{error}",
        UiTextKey::PersistenceUnexpectedCommitResponse => {
            "远程 Host 返回了非预期的工作区提交响应：{detail}"
        }
        UiTextKey::PersistenceCommitControlLost => "远程工作区提交被拒绝；控制权已丢失：{error}",
        UiTextKey::PersistencePublishDisconnectedDetail => "发布时 Host 断开连接：{error}",
        UiTextKey::PersistenceCommitFailed => "远程工作区提交失败；此前的快照保持完整：{error}",
        UiTextKey::SettingsIconThemeBuiltin => "内置",
        UiTextKey::SettingsTerminalThemeFollowUi => "跟随界面主题",
        UiTextKey::SettingsShellAuto => "自动",
        UiTextKey::PersistenceRecoveringDrafts => {
            "正在恢复设备私有草稿：{count} 份。不会替换 Host 工作区。"
        }
        UiTextKey::PersistenceRecoverableProjects => " 仍可恢复的项目：{count} 个。",
        UiTextKey::PersistenceLostTerminals => {
            "Host 丢失终端进程：{count} 个。正在恢复 Shell 和 Agent 会话，不会重放其他命令。"
        }
        UiTextKey::SettingsDeviceUnbound => "此客户端配置档案尚未绑定设备偏好设置。",
        UiTextKey::SettingsProjectChanged => "保存此设置之前，所选项目已变化。",
        UiTextKey::SettingsTabSizeInvalid => "制表符宽度必须介于 1 和 16 之间。",
        UiTextKey::SettingsLanguageEmpty => "默认语言不能为空。",
        UiTextKey::SettingsHostUnavailable => "Host 不可用",
        UiTextKey::SettingsUnexpectedDeviceResponse => "Host 返回了非预期的设备设置响应",
        UiTextKey::SettingsDraftSelectOwner => "请先选择此草稿所属的项目，再重试。",
        UiTextKey::SettingsDraftProjectControl => "重试此项目设置草稿需要 Host 控制权。",
        UiTextKey::SettingsDraftStale => {
            "此草稿创建后设置已变化。草稿仍会保留；请先检查当前值，再丢弃草稿并重新应用更改。"
        }
        UiTextKey::SettingsDraftControl => "重试此设置草稿需要 Host 控制权。",
        UiTextKey::SettingsDraftProjectInvalid => "恢复的项目设置草稿无效。",
        UiTextKey::SettingsNoSharedControl => "此窗口当前没有共享环境的控制权。",
        UiTextKey::OnboardingConfirmImport => "请先确认是否导入 Zed 主题，再完成首次引导",
        UiTextKey::OnboardingNeedsControl => "初始化默认布局和 Agent 需要 Host 控制权。",
        UiTextKey::OnboardingWaitSettings => "请等待当前设置保存完成，再完成首次引导。",
        UiTextKey::OnboardingControlLost => "保存首次引导设置之前，Host 控制权已丢失。",
        UiTextKey::SharedEditingRequired => "需要共享编辑控制权",
        UiTextKey::ConfigurationTargetUnavailable => "所选配置目标不可用。",
        UiTextKey::ConnectionsLocalWindow => "请从本地 yttt 窗口打开连接。",
        UiTextKey::ProjectOpenRequiresControl => "打开项目需要共享编辑控制权",
        UiTextKey::PreviousSshProjects => {
            "此前的 SSH 项目保留在“最近项目”中；请在独立的远程工作区中打开。"
        }
        UiTextKey::PersistenceWorkspaceAlreadyOpen => "此工作区已在当前客户端中打开",
        UiTextKey::PersistenceMissingIndex => "Host 未返回工作区索引",
        UiTextKey::PersistenceMissingRevision => "工作区修订版本不可用",
        UiTextKey::SettingsDraftCopyFailed => "无法复制设置草稿：{error}",
        UiTextKey::SettingsDraftInvalid => "恢复的设置草稿无效：{error}",
        UiTextKey::SettingsSaveFailed => "设置未保存：{error}",
        UiTextKey::SettingsProjectSaveFailed => "项目设置未保存：{error}",
        UiTextKey::SettingsRefreshFailed => "无法刷新设置：{error}",
        UiTextKey::HostProjectRegistrationFailed => "Host 项目注册失败：{error}",
        UiTextKey::SettingsUnknownGroup => "未知设置分组：{group_id}",
        UiTextKey::PersistenceClearSettingsRecoveryFailed => {
            "无法清除设备私有设置恢复副本：{error}"
        }
        UiTextKey::LayoutSourceGlobal => "全局默认",
        UiTextKey::LayoutSourceGlobalPatched => "全局默认 + 个人补丁",
        UiTextKey::LayoutSourceProject => "项目配置",
        UiTextKey::LayoutSourceProjectPatched => "项目配置 + 个人补丁",
        UiTextKey::LayoutSourcePersonal => "个人替代配置",
        UiTextKey::LayoutSourceLabel => "布局来源：{source}",
        UiTextKey::SettingsWarningFile => "设置 {path}：{error}",
        UiTextKey::SettingsWarningBars => "栏位 {field} 包含无效模块 {value}；已忽略",
        UiTextKey::SettingsWarningInvalidValue => "设置 {group}.{field} 无效；已使用默认值",
        UiTextKey::ThemeWarningFile => "主题 {path}：{error}",
        UiTextKey::ThemeWarningColor => "主题 {theme} 的颜色 {field} 无效；已使用回退颜色",
        UiTextKey::DevicePreferencesUnbound => "尚未绑定设备偏好设置。",
        UiTextKey::PersistenceRevisionChanged => {
            "此客户端断开期间，远程工作区从修订版本 {previous} 变为 {current}"
        }
        UiTextKey::PersistenceQueueFile => "无法将恢复的编辑器文件加入队列：{path}",
        UiTextKey::PersistenceProjectService => "恢复文件 {path} 所需的远程项目服务不可用",
        UiTextKey::PersistenceDocumentMissing => "未能创建恢复的编辑器文档 {path}",
        UiTextKey::PersistenceDraftConflict => {
            "恢复的草稿 {path} 与当前远程文件冲突；请检查后再保存"
        }
        UiTextKey::PersistenceDraftProjectMissing => "草稿 {path} 所属项目不可用，无法恢复",
        UiTextKey::PersistenceDraftBaseMissing => {
            "恢复的草稿 {path} 没有当前远程基准文件；仍处于未保存状态"
        }
        UiTextKey::SettingsInspectKeybindings => "无法检查快捷键目录 {path}：{error}",
        UiTextKey::SettingsResolveKeybindings => "无法解析快捷键目录 {path}：{error}",
        UiTextKey::SettingsWatchKeybindings => "无法监视快捷键配置 {path}：{error}",
        UiTextKey::HostTerminalCloseRejected => "Host 拒绝关闭终端：{details}",
        UiTextKey::RevealPathFailed => "无法在文件管理器中显示 {path}：{error}",
        UiTextKey::PersistenceHostUnavailable => "Host 不可用",
        UiTextKey::PersistenceWorkspaceUnavailable => "工作区不可用",
        UiTextKey::PersistenceNoSavedRecovery => "没有已保存的工作区恢复副本",
        UiTextKey::WorkspaceRecoveryTitle => "恢复工作区",
        UiTextKey::WorkspaceRecoveryBody => {
            "重新对账不会覆盖冲突。使用 Host 快照会替换当前布局，本地恢复副本仍保留。保留本地并发布会替换 Host 上此工作区的布局和草稿；不会重放终端命令。"
        }
        UiTextKey::WorkspaceRecoveryReconcile => "重新对账",
        UiTextKey::WorkspaceRecoveryUseHost => "使用 Host 快照",
        UiTextKey::WorkspaceRecoveryPublishLocal => "保留本地并发布",
        UiTextKey::WorkspaceFullRecoveryTitle => "恢复完整工作区？",
        UiTextKey::WorkspaceFullRecoveryBody => {
            "使用本机保留的布局、文件标签和草稿替换此 Host 工作区。当前更改必须先完成发布；终端命令不会重放。"
        }
        UiTextKey::WorkspaceRestoreAndPublish => "恢复并发布",
        UiTextKey::KeybindingUiCommandCenterTitle => "切换调度中心",
        UiTextKey::KeybindingUiCommandCenterDescription => "显示或隐藏单屏智能体调度中心。",
        UiTextKey::KeybindingUiCommandCenterQuestionPreviousTitle => "上一个调度中心问题",
        UiTextKey::KeybindingUiCommandCenterQuestionPreviousDescription => {
            "选择调度中心中上一个待处理问题。"
        }
        UiTextKey::KeybindingUiCommandCenterQuestionNextTitle => "下一个调度中心问题",
        UiTextKey::KeybindingUiCommandCenterQuestionNextDescription => {
            "选择调度中心中下一个待处理问题。"
        }
        UiTextKey::KeybindingUiCommandCenterQuestionSubmitTitle => "提交调度中心问题",
        UiTextKey::KeybindingUiCommandCenterQuestionSubmitDescription => {
            "提交所选调度中心问题的回答。"
        }
        UiTextKey::KeybindingUiPaletteNextTitle => "选择下一个面板项",
        UiTextKey::KeybindingUiPaletteNextDescription => "在已打开的面板中将选择移至下一项。",
        UiTextKey::KeybindingUiPalettePreviousTitle => "选择上一个面板项",
        UiTextKey::KeybindingUiPalettePreviousDescription => "在已打开的面板中将选择移至上一项。",
        UiTextKey::KeybindingUiPaletteAcceptTitle => "确认面板选择",
        UiTextKey::KeybindingUiPaletteAcceptDescription => "打开当前面板中所选项目。",
        UiTextKey::KeybindingUiPaletteDismissTitle => "关闭面板",
        UiTextKey::KeybindingUiPaletteDismissDescription => "关闭当前面板而不选择项目。",
        UiTextKey::KeybindingUiProjectPanelPreviousPageTitle => "项目面板：上一页",
        UiTextKey::KeybindingUiProjectPanelPreviousPageDescription => "切换到项目面板中的上一页。",
        UiTextKey::KeybindingUiProjectPanelNextPageTitle => "项目面板：下一页",
        UiTextKey::KeybindingUiProjectPanelNextPageDescription => "切换到项目面板中的下一页。",
        UiTextKey::KeybindingUiProjectPanelFilesPageTitle => "项目面板：文件",
        UiTextKey::KeybindingUiProjectPanelFilesPageDescription => "显示并聚焦项目面板中的文件页。",
        UiTextKey::KeybindingUiProjectsFocusListTitle => "聚焦项目列表",
        UiTextKey::KeybindingUiProjectsFocusListDescription => "将键盘焦点移至已打开项目列表。",
        UiTextKey::KeybindingUiProjectTreeCollapseAllNodesTitle => "折叠所有项目目录",
        UiTextKey::KeybindingUiProjectTreeCollapseAllNodesDescription => {
            "折叠项目树中所有已展开的目录。"
        }
        UiTextKey::KeybindingUiProjectTreeToggleHiddenFilesTitle => "切换显示隐藏项目文件",
        UiTextKey::KeybindingUiProjectTreeToggleHiddenFilesDescription => {
            "显示或隐藏项目树中的点文件。"
        }
        UiTextKey::KeybindingUiProjectTreeCreateFileTitle => "新建项目文件",
        UiTextKey::KeybindingUiProjectTreeCreateFileDescription => "在所选项目目录中创建文件。",
        UiTextKey::KeybindingUiProjectTreeCreateDirectoryTitle => "新建项目目录",
        UiTextKey::KeybindingUiProjectTreeCreateDirectoryDescription => {
            "在所选项目目录中创建目录。"
        }
        UiTextKey::KeybindingUiProjectTreeRenameEntryTitle => "重命名项目条目",
        UiTextKey::KeybindingUiProjectTreeRenameEntryDescription => "重命名所选项目文件或目录。",
        UiTextKey::KeybindingUiProjectTreeDeleteEntryTitle => "删除项目条目",
        UiTextKey::KeybindingUiProjectTreeDeleteEntryDescription => "删除所选项目文件或目录。",
        UiTextKey::KeybindingUiProjectTreeCopyEntryTitle => "复制项目条目",
        UiTextKey::KeybindingUiProjectTreeCopyEntryDescription => "复制所选项目文件或目录。",
        UiTextKey::KeybindingUiProjectTreeCutEntryTitle => "剪切项目条目",
        UiTextKey::KeybindingUiProjectTreeCutEntryDescription => "剪切所选项目文件或目录。",
        UiTextKey::KeybindingUiProjectTreePasteEntryTitle => "粘贴项目条目",
        UiTextKey::KeybindingUiProjectTreePasteEntryDescription => {
            "将已复制的项目条目粘贴到所选目录中。"
        }
        UiTextKey::KeybindingUiGitDiffDismissTitle => "关闭 Git 差异",
        UiTextKey::KeybindingUiGitDiffDismissDescription => "关闭当前 Git 差异面板。",
        UiTextKey::KeybindingUiGitDiffStageModeTitle => "切换 Git 差异暂存状态",
        UiTextKey::KeybindingUiGitDiffStageModeDescription => "在已暂存和未暂存更改之间切换。",
        UiTextKey::KeybindingUiGitDiffViewModeTitle => "切换 Git 差异视图",
        UiTextKey::KeybindingUiGitDiffViewModeDescription => "在统一和拆分差异视图之间切换。",
        UiTextKey::KeybindingUiGitDiffWhitespaceTitle => "切换 Git 差异空白处理",
        UiTextKey::KeybindingUiGitDiffWhitespaceDescription => "包含或忽略空白更改。",
        UiTextKey::KeybindingUiGitDiffPreviousFileTitle => "上一个 Git 差异文件",
        UiTextKey::KeybindingUiGitDiffPreviousFileDescription => "选择上一个已更改的文件。",
        UiTextKey::KeybindingUiGitDiffNextFileTitle => "下一个 Git 差异文件",
        UiTextKey::KeybindingUiGitDiffNextFileDescription => "选择下一个已更改的文件。",
        UiTextKey::KeybindingUiGitDiffCopyTitle => "复制 Git 差异",
        UiTextKey::KeybindingUiGitDiffCopyDescription => "复制所选文件的差异。",
        UiTextKey::KeybindingUiTerminalTabTitle => "向终端发送 Tab",
        UiTextKey::KeybindingUiTerminalTabDescription => "向聚焦的终端发送 Tab 键。",
        UiTextKey::KeybindingUiTerminalBacktabTitle => "向终端发送反向 Tab",
        UiTextKey::KeybindingUiTerminalBacktabDescription => "向聚焦的终端发送反向 Tab 键。",
        UiTextKey::KeybindingUiTerminalClipboardCopyTitle => "从终端复制",
        UiTextKey::KeybindingUiTerminalClipboardCopyDescription => "复制当前终端选择内容。",
        UiTextKey::KeybindingUiTerminalClipboardPasteTitle => "粘贴到终端",
        UiTextKey::KeybindingUiTerminalClipboardPasteDescription => {
            "将剪贴板文本粘贴到聚焦的终端。"
        }
        UiTextKey::KeybindingUiTerminalSearchOpenTitle => "打开终端搜索",
        UiTextKey::KeybindingUiTerminalSearchOpenDescription => "在聚焦的终端中打开搜索。",
        UiTextKey::KeybindingUiTerminalSearchForwardTitle => "下一个终端搜索匹配项",
        UiTextKey::KeybindingUiTerminalSearchForwardDescription => "选择下一个终端搜索匹配项。",
        UiTextKey::KeybindingUiTerminalSearchBackwardTitle => "上一个终端搜索匹配项",
        UiTextKey::KeybindingUiTerminalSearchBackwardDescription => "选择上一个终端搜索匹配项。",
        UiTextKey::KeybindingUiTerminalSearchHistoryUpTitle => "上一个终端搜索",
        UiTextKey::KeybindingUiTerminalSearchHistoryUpDescription => "调出上一个终端搜索查询。",
        UiTextKey::KeybindingUiTerminalSearchHistoryDownTitle => "下一个终端搜索",
        UiTextKey::KeybindingUiTerminalSearchHistoryDownDescription => "调出下一个终端搜索查询。",
        UiTextKey::KeybindingUiTerminalSearchDismissTitle => "关闭终端搜索或提示",
        UiTextKey::KeybindingUiTerminalSearchDismissDescription => "关闭终端搜索或键盘提示模式。",
        UiTextKey::KeybindingUiTerminalHintOpenTitle => "打开终端键盘提示",
        UiTextKey::KeybindingUiTerminalHintOpenDescription => {
            "打开可通过键盘选择的链接和文本提示。"
        }
        UiTextKey::KeybindingUiCloseAllTabsTitle => "关闭所有标签页",
        UiTextKey::KeybindingUiCloseAllTabsDescription => "关闭当前面板中的所有标签页。",
        UiTextKey::KeybindingUiCloseTabsBeforeTitle => "关闭之前的标签页",
        UiTextKey::KeybindingUiCloseTabsBeforeDescription => "关闭当前标签页之前的标签页。",
        UiTextKey::KeybindingUiCloseTabsAfterTitle => "关闭之后的标签页",
        UiTextKey::KeybindingUiCloseTabsAfterDescription => "关闭当前标签页之后的标签页。",
        UiTextKey::KeybindingUiCloseAllFileTabsTitle => "关闭所有文件标签页",
        UiTextKey::KeybindingUiCloseAllFileTabsDescription => "关闭所有文件标签页。",
        UiTextKey::KeybindingUiCloseAllTerminalTabsTitle => "关闭所有终端标签页",
        UiTextKey::KeybindingUiCloseAllTerminalTabsDescription => "关闭所有终端标签页。",
        UiTextKey::TerminalI18nIoError => "终端{operation}错误：{message}",
        UiTextKey::TerminalI18nIoRead => "读取",
        UiTextKey::TerminalI18nIoWrite => "写入",
        UiTextKey::TerminalI18nIoResize => "调整大小",
        UiTextKey::TerminalI18nIoMailbox => "事件邮箱",
        UiTextKey::TerminalI18nIoInputQueue => "输入队列",
        UiTextKey::TerminalI18nIoClipboard => "剪贴板",
        UiTextKey::TerminalI18nControlTransferInProgress => "此客户端正在观察或发布控制权转移。",
        UiTextKey::TerminalI18nHostConnectionNotReady => "Host 终端连接尚未就绪。",
        UiTextKey::TerminalI18nInputTerminalNotRunning => {
            "终端未运行；请先查看窗格列表，再发送输入。"
        }
        UiTextKey::TerminalI18nInputNoHostChannel => "终端没有 Host 输入通道。",
        UiTextKey::TerminalI18nInputControlUnavailable => "终端输入控制不可用。",
        UiTextKey::TerminalI18nInputViewNotReady => "终端视图尚未就绪。",
        UiTextKey::TerminalI18nInputEmpty => "输入为空。",
        UiTextKey::TerminalI18nHostRuntimeUnavailable => "Host 运行时不可用。",
        UiTextKey::TerminalI18nHostRuntimeNotConnected => "Host 运行时未连接。",
        UiTextKey::TerminalI18nHostRestartedBeforeReconcile => {
            "Host 在终端启动结果可协调前已重启。"
        }
        UiTextKey::TerminalI18nAddressConflict => {
            "终端地址与 Host 会话 {session_id} 冲突：预期指纹为 {expected}，收到 {actual}"
        }
        UiTextKey::TerminalI18nSessionNoLongerAvailable => {
            "终端 {session_id} 已无法在 Host 上使用。"
        }
        UiTextKey::TerminalI18nControlRequired => {
            "启动终端 {session_id} 需要工作区控制权，且不能有正在进行的控制权转移。"
        }
        UiTextKey::TerminalI18nHostEpochChanged => {
            "协调终端启动时 Host 世代已变化：预期 {expected}，观察到 {actual}。"
        }
        UiTextKey::TerminalI18nHostCatalogRequestClosed => "Host 资源目录请求通道已关闭。",
        UiTextKey::TerminalI18nHostCatalogResponseUnexpected => "收到意外的 Host 资源目录响应。",
        UiTextKey::TerminalI18nHostTerminalRequestClosed => "Host 终端请求通道已关闭。",
        UiTextKey::TerminalI18nHostTerminalResponseUnexpected => {
            "收到意外的 Host 终端响应：{response}"
        }
        UiTextKey::TerminalI18nLaunchOutcomeUnknown => {
            "终端启动结果仍未知；重试会使用同一次启动尝试。"
        }
        UiTextKey::TerminalI18nSessionMissingFromCatalog => "终端会话已不在 Host 资源目录中。",
        UiTextKey::TerminalI18nLeaseRevoked => "终端输入租约已被另一客户端撤销。",
        UiTextKey::TerminalI18nLeaseReleased => "终端输入租约已释放。",
        UiTextKey::TerminalI18nLeaseExpired => "终端输入租约在空闲接管后已过期。",
        UiTextKey::TerminalI18nControlRequested => "另一客户端（{client}）请求了终端控制权。",
        UiTextKey::TerminalI18nSessionUnavailable => "终端会话不可用",
        UiTextKey::TerminalI18nCommand => "命令：{command}",
        UiTextKey::TerminalI18nWorkingDirectory => "工作目录：{cwd}",
        UiTextKey::TerminalI18nPreviousProcessUnavailable => {
            "之前的进程已不可用。在您选择继续之前，不会重新启动任何进程。"
        }
        UiTextKey::TerminalI18nTakeWorkspaceControl => {
            "之前的进程已不可用。请取得工作区控制权，并等待所有转移完成后再启动它。"
        }
        UiTextKey::TerminalI18nStatus => "状态：{status}",
        UiTextKey::TerminalI18nLaunchReconciling => "正在协调终端启动结果",
        UiTextKey::TerminalI18nProcessLifecycle => "进程{state}",
        UiTextKey::TerminalI18nResumeSavedSession => "恢复已保存的会话",
        UiTextKey::TerminalI18nStartNewProcess => "启动新进程",
        UiTextKey::TerminalI18nReconnect => "重新连接",
        UiTextKey::TerminalI18nRetry => "重试",
        UiTextKey::TerminalI18nRestart => "重新启动",
        UiTextKey::TerminalI18nLifecycleIdle => "空闲",
        UiTextKey::TerminalI18nLifecycleStarting => "正在启动",
        UiTextKey::TerminalI18nLifecycleReconciling => "正在协调",
        UiTextKey::TerminalI18nLifecycleRunning => "运行中",
        UiTextKey::TerminalI18nLifecycleStopping => "正在停止",
        UiTextKey::TerminalI18nLifecycleExitedCode => "已退出（{code}）",
        UiTextKey::TerminalI18nLifecycleKilled => "已终止",
        UiTextKey::TerminalI18nLifecycleExited => "已退出",
        UiTextKey::TerminalI18nLifecycleSpawnFailed => "启动失败",
        UiTextKey::TerminalI18nLifecycleLost => "已丢失",
        UiTextKey::TerminalI18nStartFailed => "启动终端失败",
        UiTextKey::TerminalI18nError => "错误：{error}",
        UiTextKey::TerminalI18nDidNotStart => "终端未启动",
        UiTextKey::TerminalI18nAgentStarting => "Agent 正在启动",
        UiTextKey::TerminalI18nAgentIdle => "Agent 空闲",
        UiTextKey::TerminalI18nAgentWorking => "Agent 工作中",
        UiTextKey::TerminalI18nAgentWaiting => "Agent 等待中",
        UiTextKey::TerminalI18nAgentCompleted => "Agent 已完成",
        UiTextKey::TerminalI18nAgentFailed => "Agent 失败",
        UiTextKey::TerminalI18nAgentInterrupted => "Agent 已中断",
        UiTextKey::TerminalI18nAgentStatusUnknown => "Agent 状态未知",
        UiTextKey::BarI18nEditorPosition => "第 {line} 行，第 {column} 列",
        UiTextKey::BarI18nEditorModified => "● 已修改",
        UiTextKey::BarI18nEditorSaved => "已保存",
        UiTextKey::BarI18nEditorNoDiagnostics => "无诊断信息",
        UiTextKey::BarI18nEditorSelectionSingle => "{characters} 个字符 · {lines} 行",
        UiTextKey::BarI18nEditorSelectionMultiple => "{characters} 个字符 · {lines} 行",
        UiTextKey::BarI18nEditorSelectionTooltipSingle => {
            "已选择 {lines} 行，共 {characters} 个 Unicode 标量字符"
        }
        UiTextKey::BarI18nEditorSelectionTooltipMultiple => {
            "已选择 {lines} 行，共 {characters} 个 Unicode 标量字符"
        }
        UiTextKey::BarI18nEditorNoSelection => "未选择内容",
        UiTextKey::BarI18nEditorTabSize => "制表符 {size}",
        UiTextKey::BarI18nEditorTabSizeTooltip => "制表符宽度：{size} 个空格",
        UiTextKey::BarI18nEditorWrap => "自动换行",
        UiTextKey::BarI18nEditorNoWrap => "不自动换行",
        UiTextKey::BarI18nEditorWrapEnabled => "已启用自动换行",
        UiTextKey::BarI18nEditorWrapDisabled => "已禁用自动换行",
        UiTextKey::BarI18nTerminalRunning => "运行中",
        UiTextKey::BarI18nTerminalStopped => "已停止",
        UiTextKey::BarI18nTerminalExitCode => "退出 {code} · {reason}",
        UiTextKey::BarI18nTerminalExit => "退出 · {reason}",
        UiTextKey::BarI18nTerminalExitCompleted => "已完成",
        UiTextKey::BarI18nTerminalExitFailed => "失败",
        UiTextKey::BarI18nTerminalExitKilled => "已终止",
        UiTextKey::BarI18nTerminalSizeTooltip => "{columns} 列 × {rows} 行",
        UiTextKey::BarI18nAgentWaiting => "等待中",
        UiTextKey::BarI18nAgentWaitingReason => "等待 · {reason}",
        UiTextKey::BarI18nAgentWaitApproval => "批准",
        UiTextKey::BarI18nAgentWaitInput => "输入",
        UiTextKey::BarI18nAgentWaitExternal => "外部操作",
        UiTextKey::BarI18nAgentModel => "模型 {model}",
        UiTextKey::BarI18nAgentModelTooltip => "Agent 模型：{model}",
        UiTextKey::BarI18nAgentChildren => "子 Agent {count}",
        UiTextKey::BarI18nAgentChildrenTooltip => "{count} 个活跃子 Agent",
        UiTextKey::BarI18nAgentStateDurationTooltip => "当前选定 Agent 状态的持续时间",
        UiTextKey::BarI18nDurationSeconds => "{seconds}秒",
        UiTextKey::BarI18nDurationMinutesSeconds => "{minutes}分钟 {seconds}秒",
        UiTextKey::BarI18nDurationHoursMinutes => "{hours}小时 {minutes}分钟",
        UiTextKey::BarI18nDurationDaysHours => "{days}天 {hours}小时",
        UiTextKey::BarI18nUpdateNotChecked => "尚未检查更新状态",
        UiTextKey::BarI18nUpdateAvailable => "发现 v{version} 新版本",
        UiTextKey::AgentInitializationFailed => "Agent 初始化失败：{error}",
        UiTextKey::CliLocalizationCtlHelp => {
            "用法：yttt ctl <资源> <操作> [选项]\n\n  windows list\n  projects list\n  tabs list | create | focus | rename | close\n  panes list | split | focus | rename | close | resize | send | read\n  agents list | create | send\n  board show                   打开原生多项目 调度中心\n\n目标：\n  --window ID                  windows list 返回的窗口 ID（有歧义时必填）\n  --project ID                 projects list 返回的项目 ID\n  --tab ID                     tabs list 返回的终端标签页 ID\n  --pane ID                    标签页内的窗格 ID\n\n创建 / 管理：\n  tabs create --project ID [--command 'cargo test'] [--title NAME]\n  agents create --project ID --provider codex [--title NAME] [-- <provider arguments>]\n  panes split --project ID --tab ID --pane ID --direction horizontal|vertical [--command CMD]\n  panes resize --project ID --tab ID --pane ID --direction left|right|up|down [--percent 5]\n  tabs|panes rename <targets> --title NAME\n  tabs|panes close <targets>   终止目标中的进程\n\n输入：\n  panes send <targets> --text TEXT [--enter] [--raw]\n  agents send <targets> --text TEXT\n  agents send <targets> --prompt-file PATH\n  panes|agents send <targets> --stdin\n  panes read <targets>        读取当前终端视口\n  board answer <targets> --request ID --answers-json JSON\n                              排入结构化答案；请查看 Agent 确认\n\n  --json                      机器可读响应（错误输出到 stderr）\n  --help                      显示此帮助\n\n桌面应用必须已在运行。使用 YTTT_PROFILE_ROOT 指定自定义配置。\n列表涵盖已打开的桌面窗口和终端标签页。变更操作会选择其目标。\nCreated 表示布局已存在且已请求启动；请列出窗格以检查就绪状态。\nInputAccepted 表示 Host 已接收字节，不表示命令或 Agent 回合已完成。\nAgent send 会提交文本并附加 Enter，并拒绝 working/waiting/stale 的 Agent 状态。\n使用 panes send 在繁忙的 Agent 或批准提示下进行有意的原始交互。\n请求绝不会自动重试。超时后，请先检查状态再重试。\n"
        }
        UiTextKey::CliLocalizationArgumentsUtf8 => "命令行参数必须是有效的 UTF-8",
        UiTextKey::CliLocalizationExpectedResource => "需要资源；请查看 yttt ctl --help",
        UiTextKey::CliLocalizationExpectedAction => "需要操作；请查看 yttt ctl --help",
        UiTextKey::CliLocalizationUnexpectedArgument => "意外的参数：{argument}",
        UiTextKey::CliLocalizationMissingOptionValue => "{option} 缺少值",
        UiTextKey::CliLocalizationDuplicateOption => "重复的选项：{option}",
        UiTextKey::CliLocalizationRequestRequired => "必须提供 --request",
        UiTextKey::CliLocalizationAnswersJsonRequired => "必须提供 --answers-json",
        UiTextKey::CliLocalizationInvalidStructuredAnswers => "结构化答案无效：{error}",
        UiTextKey::CliLocalizationProviderRequired => "必须提供 --provider",
        UiTextKey::CliLocalizationSplitDirection => "--direction 必须为 horizontal 或 vertical",
        UiTextKey::CliLocalizationTitleRequired => "必须提供 --title",
        UiTextKey::CliLocalizationResizeDirection => "--direction 必须为 left、right、up 或 down",
        UiTextKey::CliLocalizationPercentRange => "--percent 必须是 1 到 90 之间的数字",
        UiTextKey::CliLocalizationInputSourceExactlyOne => {
            "必须且只能提供 --text、--prompt-file 或 --stdin 之一"
        }
        UiTextKey::CliLocalizationAgentRawInput => "请使用 panes send --raw 发送原始终端输入",
        UiTextKey::CliLocalizationUnknownCommand => {
            "未知命令：{resource} {action}；请查看 yttt ctl --help"
        }
        UiTextKey::CliLocalizationUnknownOption => "此命令不支持的选项：{option}",
        UiTextKey::CliLocalizationProviderArgumentsOnly => "仅 agents create 接受 provider 参数",
        UiTextKey::CliLocalizationTabsRejectPane => "标签页命令不接受 --pane",
        UiTextKey::CliLocalizationPaneRequired => "必须提供 --pane",
        UiTextKey::CliLocalizationRequestValidationError => "无效的控制请求：{error}",
        UiTextKey::CliLocalizationControlFailure => {
            "{error}。请确认对应的桌面应用正在运行；在重试变更操作前先检查状态"
        }
        UiTextKey::CliLocalizationTargetPane => " 窗格={pane}",
        UiTextKey::CliLocalizationTarget => "窗口={window} 项目={project} 标签页={tab}{pane}",
        UiTextKey::CliLocalizationWindowItem => {
            "{id}\t可写={writable}\t加载中={loading}\t项目={project}"
        }
        UiTextKey::CliLocalizationBooleanTrue => "是",
        UiTextKey::CliLocalizationBooleanFalse => "否",
        UiTextKey::CliLocalizationTabItem => "{target}\t{title}\t窗格数={panes}",
        UiTextKey::CliLocalizationStateRestoring => "正在恢复",
        UiTextKey::CliLocalizationCreated => "已创建 {target}（{state}）",
        UiTextKey::CliLocalizationUpdated => "已更新 {target}",
        UiTextKey::CliLocalizationClosed => "已关闭 {target}",
        UiTextKey::CliLocalizationInputAccepted => "Host 已接收发送到 {target} 的 {bytes} 个字节",
        UiTextKey::CliLocalizationCommandCenterShown => "调度中心已显示在窗口={window}",
        UiTextKey::CliLocalizationAnswerQueued => {
            "已为 {target} 的请求={request} 排入答案；正在等待 Agent 确认"
        }
        UiTextKey::CliLocalizationLifecycleCommandExclusive => "一次只能使用一个 Host 生命周期命令",
        UiTextKey::CliLocalizationLoginStartupConfirmationRequired => {
            "查看后台 Host 访问提示后，--enable-login-startup 需要 --confirm-remote-access"
        }
        UiTextKey::CliLocalizationProfileIdDuplicate => "--profile-id 只能指定一次",
        UiTextKey::CliLocalizationProfileMismatch => {
            "登录启动请求的配置是 {requested_profile}，但此可执行文件所属的配置是 {profile}"
        }
        UiTextKey::CliLocalizationLoginStartupDisabled => "已禁用",
        UiTextKey::CliLocalizationLoginStartupEnabled => "已启用",
        UiTextKey::CliLocalizationLoginStartupRequiresApproval => "需要在系统设置中批准",
        UiTextKey::CliLocalizationLoginStartupUnavailable => "不可用",
        UiTextKey::CliLocalizationLoginStartupStatus => "登录启动{status}（{method}）",
        UiTextKey::CliLocalizationHostForceStopping => "正在强制停止",
        UiTextKey::CliLocalizationOpenLogsFailure => "无法打开 yttt 日志：{error}",
        UiTextKey::CliLocalizationLoginStartupFailure => "登录启动命令失败：{error}",
        UiTextKey::CliLocalizationDetachedHostStdioFailure => {
            "无法将分离的 Host 与桌面命令行的标准输入输出隔离：{error}"
        }
        UiTextKey::CliLocalizationLifecycleRuntimeFailure => {
            "无法初始化 Host 生命周期运行时：{error}"
        }
        UiTextKey::CliLocalizationHostLauncherFailure => "无法初始化 Host 启动器：{error}",
        UiTextKey::CliLocalizationHostStopped => "Host 已停止",
        UiTextKey::CliLocalizationHostStatus => {
            "Host {state}：{terminals} 个终端、{clients} 个客户端、{projects} 个项目、{ssh_connections} 个 SSH 连接、{jobs} 个任务"
        }
        UiTextKey::CliLocalizationUnexpectedHostStatusResponse => {
            "意外的 Host 状态响应：{response}"
        }
        UiTextKey::CliLocalizationHostStarted => "Host 已启动",
        UiTextKey::CliLocalizationHostAlreadyRunning => "Host 已在运行",
        UiTextKey::CliLocalizationHostBusy => "Host 正被 {count} 个阻塞项占用：{blockers}",
        UiTextKey::CliLocalizationUnexpectedHostStopResponse => "意外的 Host 停止响应：{response}",
        UiTextKey::CliLocalizationHostRestarted => "Host 已重启",
        UiTextKey::CliLocalizationHostForceStopped => "Host 已强制停止",
        UiTextKey::CliLocalizationUnexpectedHostForceStopResponse => {
            "意外的 Host 强制停止响应：{response}"
        }
        UiTextKey::CliLocalizationHostLifecycleFailure => "Host 生命周期命令失败：{error}",
        UiTextKey::CliLocalizationHostExitTimeout => "等待 Host 退出超时",
        UiTextKey::CliLocalizationRemoteClientLaunchInvalid => "远程 Client 启动无效：{error}",
        UiTextKey::CliLocalizationInstallerPreflightArgumentError => {
            "安装程序 Host 预检参数错误：{error}"
        }
        UiTextKey::CliLocalizationDesktopActivationFailure => "无法激活 yttt 桌面 shell：{error}",
        UiTextKey::CliLocalizationHostRuntimeFailure => "无法初始化 Host 运行时：{error}",
        UiTextKey::CliLocalizationHostProcessFailure => "yttt Host 失败：{error}",
        UiTextKey::CliLocalizationRegistrationPermissionPrompt => {
            "yttt 需要权限才能在 /usr/local/bin/yttt 安装其终端命令。"
        }
        UiTextKey::CliLocalizationRegistrationFailure => {
            "无法安装 yttt 终端命令。仍可继续使用应用程序。\n\n{error}\n\n请解决冲突或允许安装，然后重新启动 yttt 以重试。也可以直接运行应用程序的 Contents/MacOS/yttt 可执行文件。"
        }
        UiTextKey::CliLocalizationRegistrationAlertTitle => "yttt 命令行安装",
        UiTextKey::CliLocalizationPromptInputReadFailure => "无法读取提示输入：{error}",
        UiTextKey::CliLocalizationControlRuntimeFailure => "无法初始化桌面控制运行时：{error}",
        UiTextKey::CliLocalizationStatePending => "等待中",
        UiTextKey::CliLocalizationStateInitializing => "正在初始化",
        UiTextKey::CliLocalizationErrorInvalidRequest => "请求无效",
        UiTextKey::CliLocalizationErrorNotFound => "未找到",
        UiTextKey::CliLocalizationErrorAmbiguousTarget => "目标不明确",
        UiTextKey::CliLocalizationErrorNotReady => "尚未就绪",
        UiTextKey::CliLocalizationErrorPermissionDenied => "权限被拒绝",
        UiTextKey::CliLocalizationErrorBusy => "忙碌",
        UiTextKey::CliLocalizationErrorOutcomeUnknown => "结果未知",
        UiTextKey::CliLocalizationErrorFailed => "失败",
        UiTextKey::WorkbenchI18nConnectionAddressPlaceholder => "主机:端口",
        UiTextKey::WorkbenchI18nExistingHostOperationFailed => "连接操作失败",
        UiTextKey::WorkbenchI18nSharedEditingControlRequired => "需要共享编辑控制权限。",
        UiTextKey::WorkbenchI18nDirectoryPickerFailed => "无法打开目录选择器",
        UiTextKey::WorkbenchI18nDirectoryPickerInterrupted => "目录选择被中断",
        UiTextKey::WorkbenchI18nAgentSnapshotProjectMismatch => {
            "Host Agent 快照的项目与终端 {terminal} 不匹配"
        }
        UiTextKey::WorkbenchI18nAgentSnapshotApplyFailed => "应用 Agent 快照失败",
        UiTextKey::WorkbenchI18nAgentSessionResumeFailedRetained => {
            "无法恢复 Agent 会话；原会话已保留。请重试或显式打开新会话。"
        }
        UiTextKey::WorkbenchI18nAgentSessionsLoadError => "无法加载 Agent 会话",
        UiTextKey::WorkbenchI18nAgentSessionUnavailable => "Agent 会话已不可用。",
        UiTextKey::WorkbenchI18nAgentSessionProviderNoResumeCommand => {
            "{provider} 未提供恢复此会话的命令。"
        }
        UiTextKey::WorkbenchI18nAgentSessionResumeFailed => "无法恢复 Agent 会话",
        UiTextKey::WorkbenchI18nAgentSessionsMinutesAgo => "{count} 分钟前",
        UiTextKey::WorkbenchI18nAgentSessionsHoursAgo => "{count} 小时前",
        UiTextKey::WorkbenchI18nAgentSessionsDaysAgo => "{count} 天前",
        UiTextKey::WorkbenchI18nAuxiliaryWindowOpenFailed => "无法打开窗口",
        UiTextKey::WorkbenchI18nCloseProjectKeyboardHint => "Enter 关闭，Esc 取消",
        UiTextKey::WorkbenchI18nGitOperationFailed => "Git 操作失败",
        UiTextKey::WorkbenchI18nProjectServicesUnavailable => "项目服务不可用",
        UiTextKey::WorkbenchI18nDevicePreferencesUnbound => "设备偏好设置尚未绑定。",
        UiTextKey::WorkbenchI18nBarsEditorSerializeFailed => "序列化窗口栏 TOML 失败",
        UiTextKey::WorkbenchI18nLayoutEditorPrepareFailed => "准备布局 TOML 失败",
        UiTextKey::WorkbenchI18nLayoutEditorSerializeFailed => "序列化布局 TOML 失败",
        UiTextKey::WorkbenchI18nLayoutEditorParsePosition => "第 {line} 行，第 {column} 列",
        UiTextKey::WorkbenchI18nCommandUnavailableDetail => "命令不可用：{reason}",
        UiTextKey::WorkbenchI18nProjectHostConfigurationUnavailable => {
            "所选项目没有 Host 配置路径。"
        }
        UiTextKey::WorkbenchI18nRemoteAccessInvalidAddress => "监听地址无效",
        UiTextKey::WorkbenchI18nRemoteAccessNotListening => "远程访问未在监听",
        UiTextKey::WorkbenchI18nRemoteAccessActionFailed => "远程访问操作失败",
        UiTextKey::WorkbenchI18nRemoteConnectionKindHost => "Host",
        UiTextKey::WorkbenchI18nSshConnectionOperationFailed => "SSH 连接操作失败",
        UiTextKey::WorkbenchI18nSshRequiredFields => "名称、主机和用户为必填项。",
        UiTextKey::WorkbenchI18nFieldRequired => "{field} 为必填项。",
        UiTextKey::WorkbenchI18nRemoteRestoreRemovalFailed => "无法移除远程恢复记录",
        UiTextKey::WorkbenchI18nSshConnectionNotConfigured => "未配置 SSH 连接 {connection}",
        UiTextKey::WorkbenchI18nSshProjectConnectionRequired => {
            "请先连接 SSH 端点，再打开其远程项目。"
        }
        UiTextKey::WorkbenchI18nSshProjectPickerOperationFailed => "SSH 项目操作失败",
        UiTextKey::WorkbenchI18nSshProjectHomeUnavailable => "远程主目录暂不可用。",
        UiTextKey::WorkbenchI18nWorkspaceHistoryLoadError => "无法加载工作区历史",
        UiTextKey::WorkbenchI18nWorkspaceHistoryOpenError => "无法打开工作区",
    }
}
