# Android：Agent 等待时的本地通知

分支 `feat/android-notifications`，worktree `.worktrees/android-notifications`，基于 master b56f5b8。

设计见 `docs/plans/2026-11-11-mobile-client-design.md` §6。要点：

- 前台服务，类型 `specialUse`，用来保持连接；
- Agent 进入等待时发本地通知，同一次等待去重；
- 正在查看该 Agent 时不发；
- 点通知打开对应终端；
- 不接 FCM。

数据来自已合入的首页概览（`HostSession.watch_overview`）。iOS 通知按既定决定暂缓。

每个任务先写失败的测试，再实现，测试通过后提交。编译用 `CARGO_PROFILE_DEV_DEBUG=line-tables-only`，合并后清理构建缓存。

## 任务 1：连接移到 Application 层

现在 `MobileCore`、`HostSession`、概览都放在 `MainActivity` 的 Compose 状态里，Activity 一停，前台服务就拿不到它们。改为：

- `YtttSpikeApp : Application` 持有 `MobileCore` 和 `Link`。
- `Link` 是进程内唯一的连接，负责以下几件事：
  - `connect(record)`：并发调用只连一次；
  - `session: StateFlow<HostSession?>`；
  - `overview: StateFlow<Overview>`：一个 `OverviewListener` 收到更新后写进这个 `StateFlow`，Activity 和服务都从这里读。Rust 侧每个会话只有一个监听者，所以在 Kotlin 侧分发；
  - `viewing: StateFlow<String?>`：用户当前在前台查看的终端；
  - 网络回调与 `networkChanged()`：从 Activity 移到这里。
- Activity 只负责界面：连接时调用 `Link.connect`，打开终端时设 `viewing`，离开或进后台时清掉。
- Activity 进后台时，终端页 detach，回到前台再 attach。服务在后台只需要 Agent 概要，所有终端都不显示，这样 Host 不再为后台推画面。现在的“进后台全部交还”保留。

## 任务 2：通知决策（纯 Kotlin + 单元测试）

`WaitNotifier.update(overview, viewing): List<NotificationChange>` 给出要发和要撤的通知：

- Agent 进入等待（`state == WAITING`）时发通知。同一次等待（`sessionId` + `stateStartedAtMs`）只发一次。
  - 设计写的是按 generation 与状态去重。`AgentSummary` 没有 generation，而重新进入等待时 `stateStartedAtMs` 一定会变，所以可以代替。
- 等待结束（状态变了，或者 Agent 不在了）时撤掉它的通知。
- 用户正在查看该终端（`viewing == sessionId`）时不发。之后离开那个终端，如果还在同一次等待里，也不补发，因为用户已经看到了。
- 通知内容：
  - 标题是“项目名 · tab 标题”；
  - 正文是“等批准：<等待说明或任务>”，或者“等回答：…”“等待：…”；
  - 通知 id 由 `sessionId` 算出，同一个 Agent 的新通知会替换旧的。
- 单元测试：
  - 进入等待发一次，重复的概览不重发；
  - 等待结束撤销；
  - 正在查看不发；
  - 再次进入等待（新的 `stateStartedAtMs`）再发；
  - Agent 消失时撤销；
  - 多个 Agent 互不影响。
- 只处理 `WAITING`，失败不发通知：失败通常不需要马上处理，以后需要再加。

## 任务 3：前台服务与通知

- Manifest：
  - 权限 `FOREGROUND_SERVICE`、`FOREGROUND_SERVICE_SPECIAL_USE`、`POST_NOTIFICATIONS`；
  - `<service android:name=".LinkService" android:foregroundServiceType="specialUse">`，加上 `PROPERTY_SPECIAL_USE_FGS_SUBTYPE` 属性，说明为什么要保持连接。
- 两个通知渠道：
  - `link`：低重要性，常驻通知“保持与 <Host> 的连接”，带“停止”按钮；
  - `agent-waiting`：高重要性，Agent 等待通知。
- `LinkService`：
  - 启动时 `startForeground`，确保已连接；
  - 订阅 `Link.overview` 与 `Link.viewing`，交给 `WaitNotifier`，再按结果发或撤通知；
  - 点“停止”时 `stopSelf`，连接交给 Activity 管理：Activity 不在前台时断开。
- Activity：
  - 连上后启动服务；
  - Android 13 以上在第一次启动服务前申请 `POST_NOTIFICATIONS`；拒绝也不影响连接，只是没有通知。
- 点等待通知：发一个打开 `MainActivity` 的 `PendingIntent`，带 `open = sessionId`。Activity 在 `onCreate` 和 `onNewIntent` 里打开该终端。
- 电池优化引导：首页加一行提示，可以跳到系统的“忽略电池优化”设置。spike 只放入口，不做检测。

## 任务 4：模拟器验证

用 spike_host 和它的假 Agent：

1. 配对、连上后按 Home 键，等假 Agent 进入等待，用 `adb shell dumpsys notification` 和下拉通知栏截图，确认收到通知，标题与正文正确。
2. 等待结束后通知被撤销。
3. 点通知（用 uiautomator 找到通知并点击）会打开该 Agent 的终端。
4. 在前台打开该 Agent 的终端，它进入等待时不发通知。
5. App 在后台 5 分钟以上，等待通知仍然能到，确认前台服务保持住了连接；记下 Host 上这段时间的连接情况。
6. 点常驻通知的“停止”后服务退出，不再有通知。

截图存本地 scratchpad，不上传。

## 任务 5：结论

更新移动端设计 §6（实现、偏差、测得的情况），在本文末尾写执行记录。

## 不做

- iOS 通知（APNs 需要官方推送网关，已暂缓）。
- 在通知上直接批准或回答（`AgentCommand`）。
- 开机自启、多 Host 同时保持。

## 执行记录（2026-11-12）

| 任务 | 提交 | 结果 |
|---|---|---|
| 2 通知决策 | 901e11d | 先写测试：桩实现下 6 项失败，实现后 6 项通过（另有首页的 2 项）。 |
| 1 + 3 连接上移、前台服务 | b783e63 | 两个任务改的是同一组文件，合成一个提交。 |
| 修复 | 82254fd | 服务停止时撤掉通知（见下）。 |
| 4 模拟器验证 | — | 见下。 |

### 模拟器验证（spike_host 的两个假 Agent）

1. 配对后服务以 `specialUse`（`types=0x40000000`）在前台运行，常驻通知“保持与 Host 的连接”。
2. 按 Home 键后，Agent 进入等待即收到高优先级通知，标题“Spike · Build”，正文“等批准：Fix the failing build”。`dumpsys` 记录的发出时间到撤销约 17 s，与假 Agent 16 s 的等待一致。
3. 用 uiautomator 点通知：App 回到前台并打开 Build 的终端。
4. 停在 Build 终端约 60 s：80 次检查里 Build 的通知 0 次，Code review 的通知 33 次，说明只压掉正在看的那个。
5. 后台 5.5 分钟后服务仍在前台，等待通知照常到达。模拟器接着电源、没有 Doze，真机熄屏的情况未测。
6. 点常驻通知的“停止”：服务退出。第一次发现最后一条等待通知留在通知栏（服务已走、没人撤），修复后再测，约 70 s 内 100 次检查都没有任何 yttt 通知。

### 偏差

- 去重键用 `sessionId` + `stateStartedAtMs`，不用 generation（`AgentSummary` 没有 generation）。
- 常驻通知与等待通知被系统自动归成一组，“停止”按钮要先展开组再展开常驻通知才看得到；正式版可以给常驻通知单独的 group 或用 `setGroup` 避免。
