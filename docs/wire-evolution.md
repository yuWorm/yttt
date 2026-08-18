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
- 资源/控制：`RESOURCE_PROTOCOL_VERSION = 3`（新增 ordered one-way `TerminalInput`）
- lifecycle：`LIFECYCLE_PROTOCOL_VERSION = 2`
- desktop-shell：`DESKTOP_SHELL_PROTOCOL_VERSION = 2`
