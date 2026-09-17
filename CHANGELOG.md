# Changelog

## Unreleased

## 0.3.1 - 2026-09-16

### 中文

本次发布以实际公开版本 [v1.0.0](https://github.com/yuWorm/yttt/releases/tag/v1.0.0) 为基线，
汇总其后的 87 个提交（截至 `62fcc99`，包含合并提交）。开发分支在 `e8816c0` 中明确回到
pre-1.0 版本路线，因此本次版本为 **0.3.1**，不是旧版 1.0.0 的旧构建。

v0.3.0 标签保留，但 Windows 安装脚本编译失败，未发布 GitHub Release。0.3.1 修复 Inno Setup
将行首换行字符常量误识别为预处理指令的问题；以下为相对上一公开版的完整更新说明。

#### 新增与改进

- **独立 Host 与桌面生命周期**：终端、项目文件、Git、SSH 和 Agent 资源迁移到按配置档案隔离的
  无界面 Host；本地 IPC 带认证、资源目录、输入租约和重连恢复。每个档案由单一桌面托盘／菜单栏
  管理，可重新打开窗口、查看资源数量、启动／停止／重启 Host 和打开日志。新增对应 CLI 命令及
  macOS、Windows、Linux 的可选登录启动配置和权限说明。
- **SSH 与 TLS 远程工作区**：SSH 可部署独立的 `yttt-server`，支持 Linux/macOS 的 x86_64 和
  aarch64，通过私有 Unix socket 连接。也可通过默认关闭的 TLS 1.3 监听器连接现有桌面 Host，
  使用证书绑定的 Base64 连接码、TCP 转发和可选系统钥匙串凭据。远程配置、Git、Agent 和草稿
  归所属 Host 管理；控制权在整个档案内交接，过期控制端不能继续输入或写入。
- **完整会话恢复**：自动恢复和手动“恢复上次会话”使用同一路径，保留多个工作窗口、动态终端／
  文件标签、分屏和活动项，也保留确认过的空工作区。存活进程直接重新连接；冷启动重建干净 shell，
  并恢复已保存的 Agent 会话，包括曾启动过的延迟标签。恢复失败保留原会话，不重放任意命令或旧提示词。
- **统一远程服务入口**：“远程连接”统一管理 SSH 和网络 Host；提供独立的新增／编辑弹窗、
  保存与保存并连接操作、缺失凭据提示、连接码导入，以及中英双语的连接、接管和重试流程。
  远程目录选择器支持路径前缀过滤、键盘补全、隐藏目录和打开当前目录；SSH 项目按服务器分组。
- **按功能组织设置**：独立设置窗口支持跨分类搜索中文／英文名称和配置键。控件明确标注本地偏好、
  当前环境或当前项目的归属、生效时机和只读原因，不再要求先理解 Device/Host/Project 标签页。
  外观、字体、快捷键、Vim 和通知保留在本机；环境执行配置由 Host 持有。项目可单独覆盖编辑器
  缩进与语言设置，并恢复环境默认值。
- **可靠的配置与草稿保存**：读取配置不再隐式写文件；本机设置采用文件锁、重读和基线比较，避免
  多客户端覆盖新值。控制权丢失、断线或 Host 重启时保留失败的设置候选和未发布编辑草稿，支持显式
  重试／复制／丢弃。项目配置读写保留边界检查和版本冲突保护。
- **全局 Vim 与统一快捷键**：新增 Global / Editor only / Disabled 三种模式，统一终端、编辑器、
  项目树、面板、设置与命令面板的上下文快捷键。支持 leader、多键序列录制、备选绑定、单动作解绑、
  热重载和快速指南；状态栏显示模式与待完成按键。项目树提供 neo-tree 风格操作，`Ctrl-W h/j/k/l`
  可穿越左右侧栏和工作区面板；终端模式下 `Escape` 和 `Ctrl-[` 交给进程，`Ctrl-\ Ctrl-N` 返回 Normal。
- **Agent 集成**：统一 Codex、Claude Code、Grok Build／Groky、OpenCode、Pi 和 Oh My Pi 的适配器，
  检测在普通 shell 中手动启动的 Agent，提供分组／搜索的历史会话列表、原生命令恢复、稳定会话标题、
  任务／工具／子 Agent 状态，以及等待输入、完成或失败时的应用内和可选系统通知。
- **可配置标题栏和状态栏**：使用 bracket 模板自由排列项目、Git、Vim、编辑器、终端、Agent 与性能
  信息；独立 TOML 编辑窗口提供 41 项可搜索组件、实时草稿预览、校验和恢复默认值。提供 Recommended、
  Minimal、Development、Agent 预设；默认布局更安静，不展示性能指标。多个窗口共用始终运行的后台
  性能采样器，模板只控制是否显示数据。
- **统一 Zed 风格**：引入统一 UI 原语与实时外观配置，细化语义颜色、字体回退、菜单、选择器、Git diff、
  标题／状态栏、侧栏、分屏与通知。设置和远程服务使用可复用的独立原生窗口；布局 TOML 使用独立编辑窗口。
  更新应用图标为棱角 Y、终端提示符和独立下划线光标，并保留可编辑 SVG 源文件。
- **更新与发布工具**：新增非阻塞应用更新检查、每日缓存、手动检查、按平台下载，以及基于 changelog 的
  发布说明、更新清单和校验和生成。发布流程在三平台验证通过后打包，并拒绝覆盖已经发布的资产。
  修复版本准备脚本跳过锁文件解析的问题，确保升级工作区版本后可使用 `--locked` 构建。
  发布前清理 Clippy 阻塞，修正跨平台测试路径、快捷键和编辑器初始化，隔离 Agent 安装测试继承的环境；
  macOS 打包测试改为验证实际生成的应用包与 fixture shell，不再依赖旧图标源文件。
  修复 Windows Host 持久化时的目录同步错误，以及同名档案使用不同运行目录时的命名管道冲突。
  新增持久化全局终端环境变量，自动注入新启动的本地／SSH shell 和 CLI 命令。

#### 主要修复

- 拆分 Host 的控制、终端交互、终端数据和状态事件通道，移除逐键响应和渲染期间的同步 PATH 扫描；
  合并终端帧、使用损坏行更新与有界 UI 批处理，降低输入延迟、卡顿和长时间运行的线程开销。
  Host 终端的拖选、词／行选择和复制正确处理软换行与宽字符。
- Agent Hook 使用有序投递、确认、有限重试和去重；修复完成后仍显示运行、退出后残留状态、嵌套 Agent
  状态串扰、关闭后重建标签继承旧身份、Host 替换后无法重连，以及已完成 Agent 标签无法关闭。
- 修复首次远程窗口丢失工作区、连接交接提前关闭最后一个窗口、默认布局写入失败，以及后台／遮挡窗口
  中的辅助设置操作失效。
- 本地和 SSH 项目树支持项目内部的目录符号链接，保留循环和根边界检查；删除操作只删除链接本身。
  修复远程目录名过度省略、异步结果与输入不同步、远程 `~` 展开错误。
- 修复 Global Vim 的 `Ctrl-W` 前缀迁移、大小写按键提示和 Normal 模式下 IME／文本泄漏；修复项目树
  hover 崩溃、常见文件类型图标缺失、长 Agent 标题撑开侧栏，以及透明背景下选中态和标签底边异常。
- 被外部删除的已打开文件仍可编辑，标签以删除线提示，保存直接重建文件。通知在透明窗口中保持不透明，
  操作按钮、上下文与状态信息保持可读。

#### 升级与兼容性

- **已知性能验证结果**：macOS CI 的 debug 构建在 Host 终端重连性能 smoke 中未达到约 60 FPS
  的现有阈值；两次测量的绘制 p50/p95 分别为 21.28/38.96 ms 和 20.82/41.25 ms。
  三平台 Required validation 已通过。本版保留原性能标准并披露此结果继续发布；
  该测量不等同于优化后的 Release 安装包帧率保证。
- **版本路线回退**：0.3.1 延续开发分支的 pre-1.0 决策。旧公开版 1.0.0 的用户请从本次 Release
  手动下载安装；不要依赖 SemVer 更新检查将较小的 0.3.1 识别为升级。历史 Git 标签和已发布资产不变。
- **Host 协议版本为 8**：Client 和 Host 应使用兼容构建。不兼容且仍有任务的 Host 会阻止自动替换；
  请先保存工作并安排重启，不要强制终止正在运行的任务。
- **关闭窗口不等于退出**：关闭最后一个本地窗口保留托盘和桌面所属 Host；退出桌面会终止其所属 Host
  与资源。显式 CLI／登录启动的独立 Host，以及 SSH 部署的远程 Host，具有独立生命周期。
- **恢复不是命令重放**：新偏好默认启用“恢复上次会话”；冷恢复重建 shell、恢复可恢复的 Agent，
  其他命令进程保持停止，观察者不会启动它们。先备份重要配置和未保存内容。
- **配置迁移**：旧本地外观、主题、图标、快捷键和 bars 偏好迁入本机 profile 的 `device` 目录。
  设置现按功能分类；远程访问入口为“远程服务 → 远程访问此计算机”，不再位于“权限”。
- **Bars 模板**：旧模块数组在保存时迁为模板；早期 `[Space]` / `[Space*N]` 必须改为
  `[Space: 1]` / `[Space: N]`（1–256）。已有显式布局不会自动替换；选择预设并保存才能应用新布局。
  旧性能采样开关不再生效，并在下次保存本机设置时移除。
- **安全与包格式**：连接码包含访问密钥，只应私下分享。桌面包提供 macOS arm64 DMG、Windows x86_64
  安装器和 Linux x86_64 tarball；另附四种平台／架构的无界面 Server、`update.json` 和 `SHA256SUMS`。
  macOS 包仍为 ad-hoc 签名，未做 Developer ID 签名或公证。

### English

This release compares against the last publicly shipped version,
[v1.0.0](https://github.com/yuWorm/yttt/releases/tag/v1.0.0), covering 87 subsequent commits
through `62fcc99`, including merges. Commit `e8816c0` explicitly returned development to
pre-1.0 versioning: **0.3.1 is the new release, not an older build of 1.0.0**.

The v0.3.0 tag is retained, but no GitHub Release was published because Windows installer compilation
failed. Version 0.3.1 fixes Inno Setup interpreting a line-leading newline character constant as a
preprocessor directive. The complete changes since the last public release follow.

#### Added and changed

- **Independent Host and desktop lifecycle:** terminals, project files, Git, SSH and Agent resources
  now belong to a profile-isolated headless Host, with authenticated local IPC, resource catalogs,
  input leases and reconnect recovery. One tray/menu-bar owner per profile can reopen windows,
  inspect resource counts, manage Host lifecycle and open logs. Equivalent CLI controls and opt-in
  login startup are available on macOS, Windows and Linux, with desktop permission guidance.
- **SSH and TLS workspaces:** SSH deploys a standalone `yttt-server` for Linux/macOS x86_64 and
  aarch64 over a private Unix socket. An opt-in TLS 1.3 listener also exposes an existing desktop
  Host using certificate-bound Base64 connection codes, TCP forwarding and optional OS-keychain
  credentials. Remote configuration, Git, Agents and drafts stay on their owning Host. Profile-wide
  control handoff fences stale controllers from terminal input and shared writes.
- **Complete workspace restoration:** automatic and manual restoration share one path for multiple
  windows, dynamic terminal/file tabs, split layouts and active items, including confirmed empty
  workspaces. Surviving processes reattach; cold restoration recreates clean shells and resumes saved
  Agent sessions, including previously started lazy tabs. Failed resume preserves the original
  session rather than replaying arbitrary commands or old prompts.
- **Unified Remote services:** one saved-connections list manages SSH and network Hosts, with
  type-specific add/edit modals, separate Save and Save-and-connect actions, credential prompts,
  connection-code import and English/Chinese connection, takeover and retry flows. Remote directory
  selection adds path-prefix filtering, keyboard completion, hidden folders and open-current actions;
  SSH projects are grouped by server.
- **Feature-oriented settings:** a separate native settings window searches localized/English labels
  and configuration keys across categories. Controls identify local, environment and project ownership,
  application timing and read-only reasons instead of requiring Device/Host/Project navigation.
  Appearance, fonts, keybindings, Vim and notifications remain local; execution settings belong to the
  Host. Projects can override editor tab size and language settings and restore environment defaults.
- **Safer configuration and draft writes:** configuration reads no longer create files. Device saves
  lock, reload and compare their baseline before writing, preventing concurrent Clients from losing
  newer preferences. Control loss, disconnects and Host restarts retain failed settings candidates and
  unpublished editor drafts for explicit retry/copy/discard. Project IO remains bounded and revision-checked.
- **Global Vim and unified keybindings:** Global / Editor only / Disabled modes share contextual
  bindings across terminals, editors, project trees, panes, settings and palettes. Leader expansion,
  multi-keystroke recording, alternatives, per-action unbinding, live reload and a quick-start guide
  accompany mode/pending-key feedback. Neo-tree-style file actions and `Ctrl-W h/j/k/l` span both
  sidebars and work areas. Terminal mode passes `Escape` and `Ctrl-[` through; `Ctrl-\ Ctrl-N`
  returns to Normal mode.
- **Agent integration:** unified adapters cover Codex, Claude Code, Grok Build/Groky, OpenCode, Pi and
  Oh My Pi, including Agents launched manually in shell panes. Added grouped/searchable session history,
  native resume commands, persistent session titles, task/tool/subagent state and in-app plus optional
  desktop notifications for input requests, completion and failure.
- **Configurable window/status bars:** bracket templates arrange project, Git, Vim, editor, terminal,
  Agent and performance components. A dedicated TOML editor provides a searchable 41-entry catalog,
  live draft previews, validation and Restore Defaults. Recommended, Minimal, Development and Agent
  presets complement quieter defaults without performance metrics. Workbench windows share one always-on
  background performance sampler; templates control display only.
- **Consistent Zed styling:** shared UI primitives and live appearance settings refine semantic colors,
  font fallbacks, menus, pickers, Git diff, bars, sidebars, splits and notifications. Settings and Remote
  services use reusable native windows; layout TOML has an independent editor. The application icon now
  combines an angular Y, terminal prompt and separate underscore cursor, backed by editable SVG artwork.
- **Updates and release tooling:** non-blocking update checks, daily caching, manual checks and
  platform-specific downloads use changelog-backed release notes, update manifests and checksums.
  Packaging is gated on three-platform validation and refuses to overwrite published assets.
  Release preparation now resolves the lockfile so bumped workspace versions support `--locked` builds.
  Cleared release-blocking Clippy diagnostics, corrected cross-platform test paths, shortcuts and editor
  setup, and isolated inherited Agent installation-test environments. macOS packaging tests now check
  the generated bundle and fixture shell instead of obsolete icon-source files.
  Fixed Windows Host persistence failing on directory synchronization and named-pipe collisions
  between same-named profiles using separate runtime directories.
  Persistent global terminal environment variables are injected into newly launched local/SSH shells
  and CLI commands.

#### Key fixes

- Isolated Host control, terminal-interactive, terminal-data and state-event channels; removed
  per-keystroke responses and synchronous render-time PATH scans. Coalesced frames, damage-row updates
  and bounded UI batches reduce input latency, stalls and long-running thread overhead. Host terminal
  drag, word and line selection/copy now handle soft-wrapped and wide-character text correctly.
- Ordered, acknowledged, bounded-retry and deduplicated Agent hooks fix stale running/completion state,
  missed process exits, nested Agent identity leakage, recreated tabs inheriting old identities,
  reconnecting after Host replacement and closing completed Agent tabs.
- Fixed first-remote-window workspace loss, closing the last window too early during connection handoff,
  Host-backed default-layout writes and auxiliary settings actions in occluded workbenches.
- Local and SSH trees now follow in-project directory symlinks with cycle/root-boundary checks;
  deletion removes only the link. Fixed over-truncated directory names, stale asynchronous path input
  and remote-user `~` expansion.
- Fixed Global Vim `Ctrl-W` prefix migration, case-sensitive key feedback and Normal-mode IME/text
  leakage; project-tree hover crashes, missing common file icons, oversized Agent sidebar titles,
  translucent selection contrast and active-tab bottom borders.
- Externally deleted open files remain editable, show struck-through tab titles and are recreated on
  save. Notifications remain opaque in translucent windows, with readable actions, context and status.

#### Upgrade and compatibility notes

- **Known performance validation result:** the macOS CI debug build missed the existing approximately
  60 FPS threshold in the Host terminal reattach smoke. Two runs measured paint p50/p95 of
  21.28/38.96 ms and 20.82/41.25 ms. Three-platform Required validation passed.
  This release proceeds with the result disclosed and the performance threshold unchanged;
  these debug measurements do not establish the optimized release package's frame rate.
- **Version reset:** 0.3.1 follows the development branch's explicit pre-1.0 decision. Users of the old
  public 1.0.0 release must download and install this release manually; SemVer update checks do not
  consider the numerically smaller 0.3.1 an upgrade. Historical tags and published assets remain unchanged.
- **Host resource protocol is version 8:** use compatible Client/Host builds. Busy incompatible Hosts
  refuse automatic replacement; save work and plan a restart instead of force-stopping live tasks.
- **Closing is not quitting:** closing the last local window retains the tray and desktop-owned Host.
  Quitting the desktop stops its Host and resources. Explicit CLI/login-started Hosts and SSH-deployed
  remote Hosts have independent lifetimes.
- **Restoration is not command replay:** Restore last session defaults on for new preferences. Cold
  restore recreates shells and resumes supported Agents; other commands remain stopped and observers
  never spawn them. Back up important configuration and unsaved work first.
- **Configuration migration:** legacy local appearance, themes, icons, keybindings and bars migrate into
  the local profile's `device` directory. Settings are organized by feature. Remote access now lives in
  **Remote services → Remote access to this computer**, not Permissions.
- **Bar templates:** legacy module arrays migrate on save. Replace early `[Space]` / `[Space*N]` syntax
  with `[Space: 1]` / `[Space: N]` (1–256). Explicit layouts remain unchanged until a preset is saved.
  Legacy performance-sampling switches are ignored and removed on the next Device-settings save.
- **Security and packages:** connection codes contain access keys; share them privately. Desktop assets
  are macOS arm64 DMG, Windows x86_64 installer and Linux x86_64 tarball, alongside four headless Server
  platform/architecture builds, `update.json` and `SHA256SUMS`. macOS remains ad-hoc signed, without
  Developer ID signing or notarization.

**完整提交对比 / Full comparison:** https://github.com/yuWorm/yttt/compare/v1.0.0...v0.3.1

**使用文档 / Usage:** https://github.com/yuWorm/yttt/blob/v0.3.1/docs/usage.md

## 0.3.0 - 2026-09-16

- 中文：仅创建标签；Windows 安装包编译失败，未发布 GitHub Release。完整更新内容随 0.3.1 发布。
- English: tagged only; Windows installer compilation failed and no GitHub Release was published.
  The complete release notes are included in 0.3.1.

## 1.0.0 - 2026-07-18

Historical public release. The development branch later relabeled this section as 0.2.0;
that label was not a published release. This heading follows the actual v1.0.0 tag.

### Added

- Added saved SSH connection management with SSH agent, private-key, and password authentication.
- Added explicit host-key verification backed by yttt's own `ssh-host-keys.toml` store; OpenSSH `known_hosts` files are never modified.
- Added an SFTP project picker, lazy remote file tree, conflict-checked remote editing, remote terminal panes, and remote Git status, branch, and diff operations.
- Added operating-system credential-store integration for remembered SSH passwords and endpoint-bound credential metadata.
- Added drag-to-edge work-area splitting for terminal and file tabs, with independent tab groups and resizable dividers.
- Added a project-wide file finder with Git-ignore-aware local and SSH indexing, fuzzy path ranking, file previews, and `cmd-p`/`ctrl-p` shortcuts.

### Changed

- Missing or rejected SSH passwords now open a focused retry prompt with an explicit save-password choice.
- Remote directory picker rows now use the active icon theme, fill the available width, and keep directory names left-aligned.
- Long remote directory lists now use a bounded scroll viewport while short lists remain content-sized.
- Reconnecting an SSH project refreshes expanded remote directories and Git status.
- Redesigned the empty workspace as a responsive centered dashboard with app branding, stacked actions, icons, and aligned shortcut hints.
- First-run terminal font selection now recommends the best installed fixed-width Nerd Font, or shows a Maple Mono NF installation link when none are available.

### Fixed

- Fixed Markdown IME composition updates to preserve and visibly highlight marked text across host focus requests while honoring GPUI document-space replacement ranges, preventing raw pinyin from accumulating beside committed Chinese candidates.
- Fixed file-finder previews to detect the selected file's language and apply syntax colors.
- Fixed active Markdown documents reclaiming focus from project-file create and rename inputs.
- Fixed notification popups to remain opaque when translucent window effects are enabled, matching the existing opaque dialog, panel, menu, and popover surfaces.
- Fixed Windows builds and SSH agent authentication by using the OpenSSH named pipe with a Pageant fallback.
