# yttt wire 演进规则

控制、握手、lifecycle 与 desktop-shell 的结构化 payload 使用带字段名的 CBOR map（`ciborium` + serde）。帧头、CRC 与 `MAX_FRAME_BYTES` 不变。`Vec<u8>` 大块字段通过 `serde_bytes` 编成 CBOR 字节串，避免被展开成整数数组。

## 兼容原则

v22 起资源协议是稳定基线:**追加不升版本,删除、改名或改变已有字段语义才升版本。**

1. **枚举变体只追加，不重排、不复用名称。** 旧端收到不认识的变体时按外层信封降级,不断线:
   - 请求 → Host 回复该 `request_id` 的 `FailureCode::Unsupported`,连接继续服务后续请求。
   - 响应 → 接收端把它转成该 `request_id` 的 `Unsupported` 失败,等待者照常得到结果。
   - 事件(control 通道与 state-event 通道)→ 跳过,下一事件照常交付。
   - 外层信封本身不认识(不是 Request/Response/Event,或请求缺 `request_id`)→ 仍是解码错误并断线。
   新功能在旧端只失去这一个功能;调用方据 `Unsupported` 隐藏或降级该功能。
2. **结构体字段只追加。** 新字段必须带 `#[serde(default)]` 或本身可缺省；旧对端忽略未知字段。
3. **字段名即标签。** CBOR 按名称携带变体与字段;不得复用已废弃的名称。
   `crates/yttt-protocol/tests/wire_shapes.rs` 从 serde derive 的静态名称表遍历握手、lifecycle、
   control、terminal-interactive 与 state-event 的全部 wire 类型,与
   `tests/fixtures/wire-shapes-v22.json` 比较:删除/改名直接失败并要求升版本;追加也会失败,
   确认新字段带缺省后以 `YTTT_BLESS_WIRE_SHAPES=1 cargo test -p yttt-protocol --test wire_shapes`
   重写样本。`serde_json::Value`、内部标签(`tag = ...`)、untagged 与 flatten 类型只经过
   `deserialize_any`,其内部名称不在样本内,修改它们时需人工确认兼容。
4. **不要在控制消息上使用 `skip_serializing_if` 来表达协议缺省。** 缺省应通过 `serde(default)` 在解码侧完成，避免两端对“字段是否存在”产生分歧。
5. **握手用 `protocol_range` 协商，而不是 `build_fingerprint`。** 区间有交集即可通信；`BuildIdentity` 只用于诊断与握手 transcript。无交集时返回 `RejectReason::VersionMismatch`。launcher 只按握手结果决定:协商成功即 attach(不同 build 的 Independent Host 不被替换);`VersionMismatch` 才走“空闲则停止替换、忙碌报 blockers”。
6. **`FRAME_FORMAT_VERSION` 只描述 16 字节帧头。** payload schema 演进走 `RESOURCE_PROTOCOL_VERSION` / `LIFECYCLE_PROTOCOL_VERSION` / `DESKTOP_SHELL_PROTOCOL_VERSION`。
7. **必须升版本时**,Host 声明包含新旧版本的区间并同时服务两者,直到旧版本下线;不得让一次破坏性修改再次要求全部组件同步升级。
8. **lifecycle 协议保持封闭集合。** 它是新 launcher 停止旧 Host 的唯一通道,任何修改都要升版本,且 Host 必须继续支持旧版本。

## 当前版本

- 帧头：`FRAME_FORMAT_VERSION = 1`
- 资源/控制：`RESOURCE_PROTOCOL_VERSION = 22`。v22 是稳定基线:新增 `FailureCode::Unsupported`,
  未知请求/响应/事件按上文降级,wire 名称冻结在样本中。这是最后一次要求桌面、Host、远端
  Server 与 CLI 同步更新;此后的追加无需升版本。
  v22 内追加(未升版本):`SetTerminalVisibility { revision, visible }` 请求,Host 以 `Applied` 回复;
  旧 Host 回复 `Unsupported`,Client 停止上报,所有 attachment 按可见处理。Host 不再保留原始 PTY 回放:
  `after_sequence` 字段保留但被忽略,`raw_replay_tail` 恒为空;Host 不再发送 `ResyncRequired`,
  新 Client 收到旧 Host 发来的该事件时重建 terminal-data 连接。
  v21 追加 `MutateAgentSession`
  请求（provider、session_id、project_root、Rename/Delete 操作）与
  `AgentSessionMutated` 响应；按工作区写权限授权，由目标 Host 校验会话归属并执行，
  不接受客户端提供的历史文件删除路径。桌面、Host、远端 Server 与 CLI 需同步更新。
  v20 将 `AgentSessions` 响应改为
  `{ sessions, warnings }`，完整会话列表与 provider／文件级部分失败分别传递；
  不再因单个 provider 失败清空其他会话。桌面 Client、Host、远端 Server 与 CLI
  需同步更新。v19 追加工作区 `PutAgentState` /
  `GetAgentState` 请求与 `AgentStateStored` / `AgentState` 响应，正文使用 CBOR 字节串。
  `AgentStateRef` 包含 project/tab/pane、SHA-256 与字节数，保存在 snapshot 顶层 `agent_states`；
  `Commit` 在全部引用对象持久化且校验通过后才原子发布。正文每份最大 16 MiB、每工作区最大
  256 MiB；Client 与 Host 共用 16 MiB manifest 校验，不再对 Client 单独限制 1 MiB。
  Host 记录 schema 3 自动迁移旧版内联 Agent 状态，Client 快照 schema 2 使用对象引用。
  桌面 Client、Host、远端 Server 与 CLI 需同步更新。
  v18 追加
  `ProfileControlRequest::ReacquireIfUnowned { context }`：仅本机入口可用，Host 原子检查
  Host/control epoch、无 owner、无交接，拒绝时返回未改变的控制状态；不能强制接管。
  桌面 Client、Host、远端 Server 与 CLI 需同步更新。
  v17 为 `AgentSnapshots` 追加独立的
  `project_token_usage` 汇总，并追加 `ProjectTokenUsage` 服务端事件；即使没有 Agent 快照，
  Client 仍同步项目累计计量。`TokenUsage` 不改变 Agent 生命周期；Host 按稳定来源持久化
  高水位并排除主子重叠，覆盖标识只描述已知计量来源，不代表完整会话历史。
  桌面 Client、Host、远端 Server 与 CLI 需同步更新。
  v16 为执行记录追加缺省为空的 `telemetry`
  与 `current_action`；未上报不等于零。费用使用整数微美元，自估完成度仅接受 0–100，
  不作为人工验收或项目进度。保留记录与实时快照共用这一结构；迟到的已确认执行结果只补充
  遥测，不重启已结束执行。Client、Host、远端 Server 与 CLI 需同步更新。
  v15 追加 `AgentCommand`，答案提交与人工验收
  使用现有控制权校验，并校验 Host epoch、完整 pane scope 与 Agent instance。
  `AgentSnapshot.board` 为带缺省值的结构化任务、执行、消息与问题记录。
  `AgentSnapshots` 改为 `{ snapshots, has_more }`；恢复读取按确认游标分页（目标每页 8 MiB），
  单条事件仍携带完整有界快照。工作区 manifest 上限调整为 16 MiB，Host 状态文件为 24 MiB。
  本次资源协议为明确切换，桌面 Client、Host 与远端 Server 必须同步更新。
  v14 追加工作区 `SetRestoreOnStartup`、
  `Forget`、只读 `OpenExisting` 请求与 `Forgotten` 响应。`WorkspaceSummary.restore_on_startup`
  缺省为 false，旧记录保留为历史；`WorkspaceEnvironment.workspace_history_initialized` 缺省为 false。
  显式关闭改变启动集合，不再把所有历史窗口视为待恢复窗口。桌面端与 Host 必须同步更新。
  v13 追加工作区 `OmpSessionExists`
  请求／响应，由执行 Host 查询完整 OMP 会话存储；确认不存在时恢复流程自动新建会话，
  查询错误不当作不存在。v12 追加项目 `ReadFileChunk` / `FileChunk`
  和 SSH `ReadChunk` / `Chunk`，以最多 1 MiB 的 CBOR 字节串传输二进制文件；
  响应包含总字节数和修改时间，客户端流式下载时检查文件是否变化。
  文件预览、远程默认应用打开不再借用 UTF-8 文本响应，也不提高单帧上限。
  桌面端与 Host 需同步更新；版本无交集时仍拒绝连接。
  v11 增加 Kitty 语义布局：
  `SemanticViewport.placements` 保存像素位置、尺寸、源裁剪、独立裁剪区域及绘制顺序。
  `SemanticDelta.placements = None` 表示布局未变，`Some(...)` 完整替换，空集合清除。
  可见动画的全部帧资源随快照保留，动画 tick 只切换 placement 的资源引用，
  不重传已有像素；无 PTY 输出也会发布帧更新。退出后冻结最终帧。
  图片预算为每终端 16 MiB RGBA／128 个资源；帧载荷上限为 32 MiB，
  给完整图片资源集、终端文本和消息封装保留空间。编辑器的 6 MiB 文件上限不变。
  v10 引入 Sixel 语义图片资源：
  `SemanticRow.graphics` 保存单元格内的图片引用与源偏移，
  `SemanticViewport.images` 保存当前可见引用所需的不可变 RGBA 资源，像素使用 CBOR 字节串。
  `SemanticDelta.images = None` 表示资源集未变，`Some(...)` 表示完整替换，空集合表示清除。
  图片 ID 以终端 session/epoch 为作用域；checkpoint 和历史视口包含自身需要的资源。
  旧客户端无法绘制 Host 已声明支持的 Kitty 布局，因此客户端与 Host 必须同步更新；
  版本无交集时拒绝连接，不静默丢弃图片。
  v9 引入终端生命周期切换：`SpawnTerminal` 使用 `spec`、`start_id`、
  `expected_host_epoch`，终止请求携带 `host_epoch`、`session_epoch`，并新增 `OutcomeUnknown`。
  缺失 epoch 不提供宽松默认值。ordered one-way `TerminalInput` 于 v3 引入，
  独立终端通道于 v5 引入。
- lifecycle：`LIFECYCLE_PROTOCOL_VERSION = 3`
- desktop-shell：`DESKTOP_SHELL_PROTOCOL_VERSION = 5`，CLI Agent 查询同步包含执行遥测与当前动作。
  v4 新增原生看板显示与结构化问题提交命令；响应帧上限为 16 MiB。
  桌面与 CLI 必须使用同一版本。
