# 移动端右抽屉（文件 / 会话）与文件编辑

分支 `feat/mobile-files`，worktree `.worktrees/mobile-files`，基于 master 51b75c5。设计见 `docs/plans/2026-11-11-mobile-client-design.md` §2、§5。

**界面**

- 右抽屉分“文件 / 会话”两段：
  - 文件：当前项目的文件树，目录点开时才读取；
  - 会话：该项目的 Agent 会话历史。
- 点文件全屏打开，可以编辑、保存，关闭后回到终端。

**Host 约定（设计 §5）**

- 读文件用 `Project::ReadFile`，存文件用 `Project::SaveFile { mode: Check(fingerprint) }`。
- 存文件冲突（`Conflict`）时，让用户选覆盖（`Force`）或重新加载。
- 不用文档 lease。
- 手机不发 `OpenFile` 等共享视图意图，电脑上的界面不受影响。

**编辑器（2026-11-12 定）**

- iOS：Runestone 0.5.2，MIT。
- Android：sora-editor 0.23.6，LGPL-2.1。作为独立的 Maven 依赖引入，不修改它，App 里附上它的许可证。

每个任务先写失败的测试，再实现，测试通过后提交。编译用 `CARGO_PROFILE_DEV_DEBUG=line-tables-only`，合并后清理构建缓存。

## 任务 1：核心的文件与会话接口（Rust）

在 `crates/yttt-mobile-core` 新增 `files.rs`，`MobileSession` 增加以下方法：

- `list_directory(project_id, relative_dir) -> Vec<FileEntry>`
  - 走 `ScanDirectory`，不显示隐藏文件。
  - `FileEntry` 包含 `name`、`path`（相对项目根）、`kind`（`Directory`、`File`，以及两种 symlink）。
  - 保持 Host 给的顺序，排序交给原生。
- `open_file(project_id, path) -> OpenedFile { path, text }`
  - 走 `ReadFile`。
  - fingerprint 留在核心里，按 `(project, path)` 记住，不交给原生。
  - 二进制或超过 Host 上限的文件，返回 Host 的错误说明。
- `save_file(project_id, path, text, force) -> SaveOutcome`
  - 不强制时用记住的 fingerprint 做 `Check`，强制时用 `Force`。
  - `Saved` 时更新记住的 fingerprint；`Conflict` 时返回冲突，原生据此询问用户。
  - 没打开过就保存，视为错误。
- `agent_sessions(project_id) -> Vec<SessionEntry>`
  - 项目根取自布局里该项目的 `location`。
  - providers 与桌面相同：codex、claude、grok、pi、omp。
  - `SessionEntry` 包含 provider、id、title、model、`updated_at_ms`。
  - SSH 项目暂不支持，返回错误。
- 项目要先在 Host 注册过才能读文件（`local_root`）。由工作区打开的项目是否已经注册，集成测试会回答。如果没有，手机发 `ProjectRequest::Register`。这个请求不属于共享视图意图。
- 集成测试 `a_phone_reads_edits_and_saves_a_project_file`，用真 Host：
  - 工作区打开项目，列出根目录，再列出子目录；
  - 打开文件、修改、保存，检查磁盘上的内容；
  - 在“电脑上”直接改文件后，手机保存得到冲突；强制保存后，磁盘上是手机的版本；
  - 重新打开后，再保存不冲突；
  - 会话列表对没有历史的项目返回空。

## 任务 2：FFI

`HostSession.listDirectory`、`openFile`、`saveFile`、`agentSessions`。都是 async，在核心的 runtime 上执行。新增的类型都是 uniffi Record/Enum。

## 任务 3：Android

- **右抽屉**：用第二个 `ModalNavigationDrawer`，外层设为 RTL 让它从右边出来，内容再改回 LTR。顶栏右侧加一个按钮打开它。
  - 分段用 Material 3 的 `SingleChoiceSegmentedButtonRow`。
  - 文件树：`LazyColumn`，按目录层级缩进，目录点开时才调用 `listDirectory`，目录排在文件前，按名字排序。
  - 会话：`ListItem` 显示标题、provider、model 和更新时间。spike 只显示，不做继续会话或删除。
- **编辑页**：全屏，用 `AndroidView` 承载 sora-editor 的 `CodeEditor`。
  - 等宽字体，不做语法高亮，高亮之后再加。
  - 顶栏有“关闭”“保存”。有未保存的修改时，关闭前确认。
  - 冲突时弹窗，选“覆盖”（强制保存）或“重新加载”。
- **许可证**：spike 在设置入口旁放“开源许可”，列出 sora-editor、termlib、JNA。

## 任务 4：iOS

- **右抽屉**：`SideDrawer` 增加从右边滑出的方向，顶栏右侧加按钮。
  - 分段用 `Picker(.segmented)`。
  - 文件树用 `List`，目录点开时才读取。
  - 会话列表与 Android 相同。
- **编辑页**：`fullScreenCover`，用 `UIViewRepresentable` 承载 Runestone 的 `TextView`。
  - 等宽字体，显示行号。
  - 保存、关闭确认、冲突处理与 Android 相同。

## 任务 5：模拟器验证

- spike_host 的项目里加几个文件和一个子目录，例如 `README.md`、`src/app.js`。
- 两个模拟器上：
  - 打开右抽屉，看到文件树，展开子目录；
  - 点文件打开、修改、保存，在 Mac 上检查文件内容；
  - 在 Mac 上改文件后，再从手机保存，出现冲突弹窗，选“覆盖”后检查磁盘内容。
- 会话一段在 spike 项目上应该是空的。如果开发机上有真实的 Codex 或 Claude 历史，用它的项目路径再看一次。
- Android 用坐标点击加截图（`uiautomator` 在终端页会卡）；iOS 用自动化参数加截图。

## 任务 6：结论

更新移动端设计 §2、§5，在本文末尾写执行记录。

## 不做

- SSH 项目的文件（要走 `RemoteFile`）。
- 新建、重命名、删除文件。
- 二进制文件与图片预览。
- 电脑上有未保存草稿时的提示（设计 §5 提到，下一步再做）。
