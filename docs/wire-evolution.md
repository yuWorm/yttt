# yttt wire 演进规则

控制、握手、lifecycle 与 desktop-shell 的结构化 payload 使用带字段名的 CBOR map（`ciborium` + serde）。帧头、CRC 与 `MAX_FRAME_BYTES` 不变。`Vec<u8>` 大块字段通过 `serde_bytes` 编成 CBOR 字节串，避免被展开成整数数组。

## 兼容原则

1. **枚举变体只追加，不重排、不复用名称。** 未知变体必须解码失败并映射为 typed 拒绝，不得错位成另一个变体。
2. **结构体字段只追加。** 新字段必须带 `#[serde(default)]` 或本身可缺省；旧对端忽略未知字段。
3. **字段名即标签。** 不得复用已废弃的字段名；废弃字段保留占位直到明确的大版本切割。
4. **不要在控制消息上使用 `skip_serializing_if` 来表达协议缺省。** 缺省应通过 `serde(default)` 在解码侧完成，避免两端对“字段是否存在”产生分歧。
5. **握手用 `protocol_range` 协商，而不是 `build_fingerprint`。** 区间有交集即可通信；`build_fingerprint` / `resource_compatibility` 只作诊断，不作为 fail-closed 条件。无交集时返回 `RejectReason::VersionMismatch`。
6. **`FRAME_FORMAT_VERSION` 只描述 16 字节帧头。** payload schema 演进走 `RESOURCE_PROTOCOL_VERSION` / `LIFECYCLE_PROTOCOL_VERSION` / `DESKTOP_SHELL_PROTOCOL_VERSION`。

## 当前版本

- 帧头：`FRAME_FORMAT_VERSION = 1`
- 资源/控制：`RESOURCE_PROTOCOL_VERSION = 13`。v13 追加工作区 `OmpSessionExists`
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
- desktop-shell：`DESKTOP_SHELL_PROTOCOL_VERSION = 2`
