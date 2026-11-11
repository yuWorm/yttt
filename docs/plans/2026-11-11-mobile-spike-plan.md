# 移动端 spike 计划

> **For agentic workers:** REQUIRED: 遵循仓库 `AGENTS.md`。在当前会话内按顺序执行本计划，不要把计划执行委托给 subagent。所有改动都在 `.worktrees/mobile-spike`（分支 `spike/mobile-client`）里进行。每项先写能复现缺失行为的测试，再做最小实现；只有验收条件实际通过后才能勾选。

**状态：** 未开始。

**设计：** [`2026-11-11-mobile-client-design.md`](./2026-11-11-mobile-client-design.md)

**目标：** 用最少的代码回答三个问题，结论写回设计文档 §8：

1. `yttt-client-core` + iroh 能否编译到 iOS / Android，并在手机上连上 Mac 上的 Host。
2. 把 Host 语义增量编码为 VT 重绘序列、按屏幕刷新拉取，交给 SwiftTerm / Termux 显示，是否正确、是否流畅。
3. Termux 终端组件能否接远程数据源。

**不在 spike 内：** 首页、抽屉、文件、通知、Keychain、扫码；Host 协议改动。spike 代码可以粗糙，但 `yttt-mobile-core` 的 VT 编码器按正式代码写（有测试），之后直接沿用。

---

## 执行约束

- [ ] 0 → 7 顺序执行，每项单独提交。
- [ ] 不改 Host 与 wire 协议；`cargo test --workspace` 在每次提交前保持通过（新 crate 自己的测试也在其中）。
- [ ] Rust 依赖固定精确版本（与仓库其他 crate 一致）；UniFFI 用 `=0.31.1`，与本机 `uniffi-bindgen 0.31.1` 一致。
- [ ] 设备私钥在 spike 中存 App 沙盒文件，并在代码里标注“spike：正式版改 Keychain/Keystore”。

---

# 0：工具链

- [ ] `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`（不改仓库文件）。
- [ ] Android：本机没有 SDK/NDK。需要用户同意后安装 command-line tools、platform-tools、一个 API 35 平台、NDK、arm64 模拟器镜像，以及 `cargo-ndk`、`rustup target add aarch64-linux-android`。未安装时第 6 项标为受阻，不影响 1–5。

# 1：`yttt-mobile-core` 骨架

- [ ] 新 crate `crates/yttt-mobile-core`，`crate-type = ["lib", "staticlib", "cdylib"]`，依赖 `yttt-client-core`、`yttt-protocol`、`yttt-transport`、`yttt-transport-iroh`、`yttt-core`、`tokio`、`uniffi`。加入 workspace members。
- [ ] 一个导出函数 `core_version() -> String` 作为 FFI 冒烟。

**验收：**
- [ ] `cargo build -p yttt-mobile-core --release --target aarch64-apple-ios-sim` 与 `--target aarch64-apple-ios` 成功；记录 staticlib 大小。
- [ ] 失败时记录具体是哪个依赖（预期风险点：aws-lc-sys、netwatch、portmapper）以及修法。

# 2：VT 重绘编码器

`crates/yttt-mobile-core/src/vt.rs`：输入 `SemanticViewport`（来自 `TerminalMirror::viewport()`），输出字节。

- 状态 `VtEncoder { rows: Vec<u64 /* 行内容 hash */>, cursor, modes, geometry_epoch, session_epoch }`。
- `encode(&mut self, viewport) -> Vec<u8>`：首次、尺寸或 epoch 变化时 `ESC[H ESC[2J` 后整屏重绘；之后只重绘 hash 变化的行。每行：`CUP` → 每个 span：SGR 全量重设 + 文本（span 之间的空列补空格）→ `ESC[0m ESC[K`。
- 颜色：`Named` 按 alacritty `NamedColor` 数值映射（0–15 → 30–37 / 90–97，`Foreground`/`Background` → 39/49，其余暗色/亮色变体按其基色），`Indexed` → `38;5;n`，`Rgb` → `38;2;r;g;b`；背景、下划线颜色（`58`）同理。
- 样式位（`STYLE_*`）→ SGR 1、2、3、4、4:2/4:3/4:4/4:5、7、8、9。
- 光标：`CUP` 到 `SemanticCursor` 位置、`DECSCUSR` 形状、`DECTCEM` 显隐，放在每次输出末尾。
- 模式（alacritty `TermMode` 位）变化时输出对应 `DECSET`/`DECRST`：光标键（1）、bracketed paste（2004）、鼠标上报（1000/1002/1003/1006）、焦点上报（1004）。不切换备用屏：重绘总是画在当前屏上。
- 超链接：span 带 `hyperlink` 时包 OSC 8。

**测试（先写）：** 用 `yttt-terminal-core` 里已有的 `alacritty_terminal` 做往返：把编码结果喂给一个新的 alacritty `Term`，逐格比较字符、前景、背景、样式位，以及光标位置与模式位。覆盖：

- [ ] 纯文本多行；行尾残留被 `EL` 清掉。
- [ ] Named / Indexed / Rgb 前景与背景，以及默认色。
- [ ] 各样式位。
- [ ] CJK 宽字符与组合字符。
- [ ] 增量：只改一行时输出只含这一行（检查字节里只有一个 `CUP` 行号）。
- [ ] 尺寸变化、`session_epoch` 变化触发整屏清除重绘。
- [ ] 光标位置、形状、隐藏；光标键模式与 bracketed paste 模式往返。
- [ ] 超链接 OSC 8 往返。

**验收：** 以上测试通过；`cargo clippy -p yttt-mobile-core` 无警告。

# 3：会话 API（Rust 侧，先不经 FFI）

`crates/yttt-mobile-core/src/session.rs`，在自己的 tokio 运行时上运行：

- `pair(code_text, device_label, device_secret, on_short_code) -> PairedHostRecord`：`PairingCode::decode` → `IrohEndpoint::bind` → `yttt_client_core::p2p::pair`。记录序列化为字节，由调用方保存。
- `connect(record, device_secret) -> MobileSession`：与 `src/remote_host.rs` 的 `connect_paired` 相同的身份与凭据构造（不含 relay 账号；`client_instance_id` 前缀 `mobile-client-`），`ClientCore::connect`。
- `terminals() -> Vec<TerminalInfo>`：来自 resource catalog。
- `attach(id)`：按 `src/ui/terminal/pane.rs` 的 `AttachTerminal` 流程以观察者附加；`set_visible_terminals` 只含当前终端。
- `drain(id) -> Option<Vec<u8>>`：取镜像当前 viewport，经 `VtEncoder` 输出；无变化返回 `None`。
- `take_control(id)` / `release_control(id)`：沿用 `TakeTerminalController` 与 `AcquireTerminalLease` 的现有流程；`send_input(id, bytes)` 构造 `TerminalMutationContext` 发送；`resize(id, cols, rows)`。
- 统计：累计字节、`drain` 耗时直方图（p50/p95/max）。

**桌面冒烟程序** `crates/yttt-mobile-core/examples/mobile_tty.rs`：在 Mac 终端里配对本机 Host、列出终端、附加一个，以 60 Hz 调 `drain` 把字节写到 stdout；`--control` 时进入 raw mode，把 stdin 字节交给 `send_input`。它在桌面上验证整条 Rust 链路与编码结果，不依赖手机。

**验收：**
- [ ] 测试：`drain` 在无变化时返回 `None`；附加后第一次 `drain` 是整屏重绘（用 `yttt-client-core` 现有的内存传输测试夹具，或者在 recovery_tests 的夹具上搭一个最小 Host）。
- [ ] 手动（需要用户在桌面上确认配对）：`mobile_tty` 显示的内容与桌面一致；运行 `seq 1 200000` 时不卡死，记录 `drain` p95；`--control` 下能输入、vim/htop 等全屏程序可用。

# 4：UniFFI 绑定与 iOS 打包

- [ ] 用 UniFFI proc-macro 导出第 3 项的 API：`MobileCore` 对象、回调接口 `PairingListener { on_short_code(u32) }`、错误类型。异步函数用 UniFFI async（Swift `async`）。
- [ ] `scripts/mobile/build-ios.sh`：编译 `aarch64-apple-ios` 与 `aarch64-apple-ios-sim` 的 staticlib，`uniffi-bindgen generate --library` 生成 Swift 绑定，打包 `YtttMobileCore.xcframework`（产物不入库，`.gitignore` 只新增条目）。

**验收：** 脚本一次跑通；生成的 Swift 文件可被第 5 项工程编译。

# 5：iOS spike App

`mobile/ios/`，xcodegen `project.yml` 生成工程，SwiftUI，SPM 依赖 SwiftTerm（固定到一个发布 tag）。

- [ ] 页面 1：粘贴 `YTTT-PAIR:1:` 文本 → 显示六位数字 → 配对成功后保存记录。
- [ ] 页面 2：终端列表 → 点进页面 3。
- [ ] 页面 3：SwiftTerm `TerminalView`，scrollback 0；`CADisplayLink` 每帧 `drain` 并 `feed`；“接管”按钮 → `take_control`，按视图尺寸 `resize`，键盘输入经 `TerminalViewDelegate.send` → `send_input`；角落显示字节/秒、`drain` p95、帧率。
- [ ] 观察模式下固定为 Host 列数，放进横向 `ScrollView`。

**验收（模拟器，连本机 Host）：**
- [ ] 配对、连接、观察成功；内容与桌面一致（中文、颜色、光标）。
- [ ] 接管后能在 shell 和一个 Agent CLI 里输入；方向键、Ctrl-C、粘贴可用。
- [ ] `seq 1 200000` 期间界面不卡，记录帧率与 `drain` p95。
- [ ] 若有真机：同一 Wi-Fi 与蜂窝网络（经 Relay）各连一次，记录路径（直连 / Relay）。

# 6：Android spike App（依赖第 0 项的 Android 工具链）

`mobile/android/`，Gradle（Kotlin DSL）+ Compose，`cargo-ndk` 编出 `arm64-v8a` 的 `.so`，UniFFI 生成 Kotlin 绑定。

- [ ] 引入 Termux `terminal-emulator` / `terminal-view`（JitPack 固定版本），先核实许可证。
- [ ] 实现远程数据源：不启动本地进程，把 `drain` 字节写进 `TerminalEmulator`，把 `TerminalView` 产生的按键字节交给 `send_input`。记录是否需要 fork。
- [ ] 页面与 iOS 相同（配对文本、终端列表、终端），`Choreographer` 每帧 `drain`，`AndroidView` 包装。

**验收（arm64 模拟器，连本机 Host）：** 与第 5 项相同的三条；另记录 `.so` 大小。

# 7：结论

- [ ] 在设计文档 §8 写下：三个问题的答案、测得的数据、遇到的坑和绕法、对正式实现的修改建议（含第 7 节未决问题 1 的观察结果）。
