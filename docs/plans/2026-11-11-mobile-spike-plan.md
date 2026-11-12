# 移动端 spike 计划

> **For agentic workers:** REQUIRED: 遵循仓库 `AGENTS.md`。在当前会话内按顺序执行本计划，不要把计划执行委托给 subagent。所有改动都在 `.worktrees/mobile-spike`（分支 `spike/mobile-client`）里进行。每项先写能复现缺失行为的测试，再做最小实现；只有验收条件实际通过后才能勾选。

**状态：** 完成（2026-11-12），结论见设计文档 §8。未验证项列在 §8 末尾。计划已按 2026-11-11 审查意见修订。**偏离：** 1–4 的 Rust 改动按“iroh feature / mobile-core crate”两次提交，没有逐项拆分（各项共用同一组文件）。

**执行记录：**
- 第 1 项：iOS 的 cdylib 链接要求 `IPHONEOS_DEPLOYMENT_TARGET` 与 C 依赖一致，否则缺 `___chkstk_darwin`；`scripts/mobile/build-ios.sh` 固定为 17.0。未 strip 的 iOS staticlib 86 MB。依赖树里 ring 与 aws-lc、reqwest 0.12 与 0.13 并存；`yttt-terminal-core` 经 alacritty_terminal 带进 signal-hook、rustix-openpty 等 PTY 依赖，手机用不到。
- 全量测试：CI 那组包 + 两个新 crate，83 个测试目标通过。`yttt-host` 的 `control_connection::a_slow_agent_history_scan_does_not_hold_up_the_sessions_other_requests` 在整组运行时卡死过一次（31 分钟），单独跑与整文件跑都通过；是已有的时序问题（扫描线程晚于测试打开并关闭 FIFO 时会永久阻塞），与本分支无关。
- 第 3 项手测（spike Host）：按审查建议持久化 `client_instance_id` 后，第二个进程握手成功随即被 Host 关闭、无限重试：Host 把 ID 绑定到首个认证会话并保留 5 分钟。改为每次连接新生成 ID，集成测试加了“关闭后立即重连”。
- 第 1 项 Android：vendor 的 `alacritty_terminal` 用了 Android libc 没有的 `shm_open`，编不过。mobile-core 只用它和 terminal-core 的样式位、模式位常量，改为本地常量加一致性测试，两者降为 dev-dependency（同时去掉了 signal-hook、rustix-openpty 等 PTY 依赖）。之后 `cargo ndk -t arm64-v8a --platform 26` 通过：`.so` 28 MB，strip 后 18.4 MB，gzip 约 7.1 MB；iOS staticlib 降到 80 MB（未 strip）。
- 手测改用 `examples/spike_host.rs`：临时 profile 上的真实 `yttt-host`，P2P 不走 relay，自动确认配对，退出即删除，不留受信设备；不碰开发或正式 profile。**偏离**于“只连开发 Host”的约束，隔离更彻底。
- 第 3 项手测（`mobile_tty`，release）：真实 P2P 配对与连接；观察演示终端，颜色、中文、emoji 正确，每秒的 tick 只重绘一行；接管 shell 后输入的命令执行；`seq 1 200000` 在 Host 上 0.75 s 跑完，客户端收到 49 帧共 244 KB，drain p50 56 µs、p95 102 µs、max 323 µs。vim / htop 与“电脑点接管后回到只读”未测。
- 第 5 项手测（iOS 模拟器，借环境变量 `YTTT_SPIKE_*` 自动配对、打开、接管、输入）：配对、观察（颜色、中文、emoji、实时刷新，60 fps，drain p95 29 µs）、接管后输入、PTY 改为手机宽度（`$COLUMNS` = 58）都成功，SwiftTerm 自带按键栏可用。方向键、Ctrl-C、粘贴、Agent CLI 与应用内 `seq` 吞吐未测；没有真机。
- 第 6 项（Android 模拟器 API 35 arm64，debug）：AGP 9.4.1 / Kotlin 2.4.20 / Gradle 9.8.0 / compileSdk 37 照 termlib 的组合；AGP 按需装了 platform 37 与 build-tools 36。AGP 9 内置 Kotlin，生成的绑定要加进 `kotlin` 源集。`cargo ndk -o` 会把依赖自己的 cdylib（`libiroh-*.so`）也拷进 jniLibs，脚本改为只拷核心库。经真实 P2P 配对；观察（`forcedSize` 120×36，字号很小）颜色、中文、emoji、刷新正确，60 fps，drain p95 83 µs；接管后自动输入的命令执行，经系统输入法输入的命令也执行，PTY 为 63×40。
- 环境副作用：`brew install gradle` 把 Homebrew 的 `openjdk` 从 23.0.2 升到 27（23.0.2 仍在 Cellar）；Gradle 用 `JAVA_HOME=/opt/homebrew/Cellar/openjdk/23.0.2/...` 运行。
- 第 5 项组件测试：SwiftTerm 的 `translateToString` 默认把宽字符的占位格输出为空格，需传 `skipNullCellsFollowingWide: true`；重绘写入的行尾空白是真实空格，`trimRight` 不去掉。两者都是取文本的方式，不是渲染错误。

**设计：** [`2026-11-11-mobile-client-design.md`](./2026-11-11-mobile-client-design.md)

**目标：** 用最少的代码回答三个问题，结论写回设计文档 §8：

1. `yttt-client-core` + iroh 能否编译到 iOS / Android，并在手机上连上 Mac 上的 Host。
2. 把 Host 语义增量编码为 VT 重绘序列、按屏幕刷新拉取，交给 SwiftTerm / Termux 显示，是否正确、是否流畅。
3. Termux 终端组件能否接远程数据源。

**不在 spike 内：** 首页、抽屉、文件、通知、Keychain、扫码；Host 与 wire 协议改动（设计 §7 问题 1、2 的协议改动另立计划）。spike 代码可以粗糙，但 `yttt-mobile-core` 的 VT 编码器按正式代码写（有测试），之后直接沿用。

---

## 执行约束

- [ ] 0 → 7 顺序执行，每项单独提交。
- [x] 不改 Host 与 wire 协议；每次提交前，CI `validation.yml` “Test workspace” 那组包（`--all-targets --locked --no-fail-fast`）加上 `yttt-mobile-core`、`yttt-transport-iroh` 保持通过。不用 `--workspace`：它会编译 vendor 里 `gpui-markdown-editor` 的测试，那些测试在 master 上就编不过，CI 也不跑。
- [x] 新依赖按仓库多数 crate 的写法用 caret 版本；UniFFI 固定 `=0.31.1`，与本机 `uniffi-bindgen 0.31.1` 一致。
- [x] 设备私钥在 spike 中存 App 沙盒文件，并在代码里标注“spike：正式版改 Keychain/Keystore”。
- [ ] 手动测试只连开发 Host（`scripts/run-dev-app.sh`），不连正式 profile：配对会留下受信设备，接管会抢走电脑上的终端。每轮手测结束后在开发 Host 上撤销测试设备，并在本计划里记一笔。
- [x] 终端一律以 Observer 附加；只有用户点“接管”时才发 `TakeTerminalController`。

---

# 0：工具链

- [x] `rustup target add aarch64-apple-ios aarch64-apple-ios-sim aarch64-linux-android`（不改仓库文件）。
- [x] Android command-line tools（Homebrew cask `android-commandlinetools`，`ANDROID_HOME=/opt/homebrew/share/android-commandlinetools`）、`cargo-ndk`。
- [x] `sdkmanager`：`platform-tools`、`platforms;android-35`、`build-tools;35.0.0`、`ndk;28.2.13676358`、`emulator`、`system-images;android-35;google_apis;arm64-v8a`；用 `avdmanager` 建一个 arm64 模拟器。

# 1：`yttt-mobile-core` 骨架与交叉编译

- [x] 新 crate `crates/yttt-mobile-core`，`crate-type = ["lib", "staticlib", "cdylib"]`，依赖 `yttt-client-core`、`yttt-protocol`、`yttt-transport`、`yttt-transport-iroh`、`yttt-terminal-core`（`STYLE_*` 常量）、`yttt-core`、`tokio`、`uniffi`。加入 workspace members。
- [x] 一个导出函数 `core_version() -> String` 作为 FFI 冒烟。
- [x] `yttt-transport-iroh` 的 iroh 依赖：`fast-apple-datapath`（私有 API，有 App Store 拒审风险）与 `portmapper`（UPnP 在 iOS 需要 multicast entitlement）都只在 `cfg(not(any(target_os = "ios", target_os = "android")))` 下启用，桌面三平台的 feature 集合保持不变。

**验收：**
- [x] `cargo build -p yttt-mobile-core --release` 分别在 `--target aarch64-apple-ios-sim`、`aarch64-apple-ios` 与 `cargo ndk -t arm64-v8a` 下成功；记录 staticlib / `.so` 大小，以及依赖树里 ring 与 aws-lc、两个 reqwest 版本的占比。
- [x] `cargo tree -p yttt --target aarch64-apple-darwin -e features -i iroh` 与改动前相同（桌面不受影响）。
- [ ] 失败时记录具体依赖和修法；aws-lc-sys 编不过就改用 iroh 的 `tls-ring`。

# 2：VT 重绘编码器

`crates/yttt-mobile-core/src/vt.rs`：输入 `SemanticViewport`，输出字节。规则见设计 §4，要点：

- 每次输出以 `CSI ?7l`（关自动换行）开头。首次、尺寸、`session_epoch`、`geometry_epoch` 变化时 `ESC[H ESC[2J` 后整屏重绘；之后只重绘内容 hash 变化的行。
- 每行先 `CUP` 到行首并 `CSI 2K`；每个 span 再按 `start_column` 单独 `CUP`，SGR 全量重设后输出文本；写完文本后不输出 `EL`。
- 颜色：`Named` 按 alacritty `NamedColor` 数值（0–15 → 30–37 / 90–97，`Foreground`/`Background` → 39/49，暗色和亮色变体按基色），`Indexed` → `38;5;n`，`Rgb` → `38;2;r;g;b`；背景同理；下划线颜色只在有下划线样式且不等于前景时输出 58，默认 59。
- 样式位（`STYLE_*`）→ SGR 1、2、3、4、4:2–4:5、7、8、9。
- 调色板覆盖 → OSC 4 / 10 / 11。
- 模式：`APP_CURSOR` → `?1`，`BRACKETED_PASTE` → `?2004`，`APP_KEYPAD` → `DECKPAM`/`DECKPNM`；`set_interactive(true)` 后才同步鼠标（1000/1002/1003/1006）与焦点（1004）。其余模式不同步。
- 光标：末尾 `CUP` 到光标位置 + `DECSCUSR` + `DECTCEM`。超链接包 OSC 8。

**测试（先写）：**

- 往返：编码结果喂给一个新的 alacritty `Term`（`alacritty_terminal` 作 dev-dep），再用 `SemanticTerminalState::capture` 抓回 viewport，与原 viewport 比较行内容、光标、模式位。覆盖：多行文本；Named / Indexed / Rgb 前景背景与默认色；各样式位；CJK 宽字符与组合字符；光标位置、形状、隐藏；光标键与 bracketed paste 模式；超链接。
- 字节级断言（alacritty 掩盖不了的部分）：输出以 `CSI ?7l` 开头；不含 `CSI K` / `CSI 0K`（只有行首的 `CSI 2K`）；增量只改一行时，除末尾的光标 `CUP` 外，`CUP` 的行号只有这一行；下划线颜色缺省时不出现 58；未 `set_interactive` 时不出现 1000/1002/1003/1006/1004。
- 尺寸与 epoch 变化时输出含 `ESC[2J`。

**验收：** 以上测试通过；`cargo clippy -p yttt-mobile-core --all-targets` 无警告。组件侧的正确性在第 5、6 项另测。

# 3：会话 API（Rust 侧，先不经 FFI）

`crates/yttt-mobile-core/src/session.rs`，在自己的 tokio 运行时上运行：

- `pair(code_text, device_label, device_secret, on_short_code) -> PairedHostRecord`：`PairingCode::decode` → `IrohEndpoint::bind` → `yttt_client_core::p2p::pair`。记录序列化为字节，由调用方保存。
- `connect(record, device_secret) -> MobileSession`：身份与凭据按 `src/remote_host.rs` 的 `connect_paired` 构造，但不含 relay 账号、ticket 续期、`HostStorage` 与配置绑定；**偏离：** `client_instance_id` 每次连接新生成（前缀 `mobile-client-`），不放进记录；原因见设计 §5。`ClientCore::connect`。session 构造接受一个注入的 connector，供测试使用。
- `workspace()`：`WorkspaceRequest::List` → `Open` 标为 active 的那个；不发 `SetActive` 和任何视图 intent。
- `terminals() -> Vec<TerminalInfo>`：来自 resource catalog。
- `attach(id)`：以 **Observer** 发 `AttachTerminal`（不照抄 `pane.rs` 的 Interactive 附加）；`set_visible_terminals` 只含当前终端。
- `drain(id) -> Option<Vec<u8>>`：先比较 `terminal_metadata().sequence`，有变化才取 `terminal_snapshot` 交给 `VtEncoder`；无变化返回 `None`。
- `take_control()`：发 `TakeTerminalController`，从 `TerminalControlGranted` 事件里取 lease epoch；之后 `resize(id, cols, rows)` 与 `send_input(id, bytes)` 按现有 `TerminalMutationContext` 规则发送，编码器 `set_interactive(true)`。没有交还请求（设计 §7 问题 1），spike 中由电脑点接管取回。
- 统计：累计字节、`drain` 耗时（p50 / p95 / max）。

**测试（先写）：** 参照 `crates/yttt-client-core/tests/p2p_pairing.rs`（真实 Host + `RelaySetup::Off`）与 `tests/host_roundtrip.rs` 的 `RunningHost`，把需要的夹具复制进 `crates/yttt-mobile-core/tests/`（`yttt-host`、`yttt-transport-local` 作 dev-dep）：

- [x] 配对 + 连接成功，记录可序列化往返。
- [x] 附加后第一次 `drain` 是整屏重绘；无变化时返回 `None`；终端有输出后再 `drain` 只含变化的行。
- [x] 附加后 Host 上该终端的 controller 不是手机，PTY 尺寸未变。
- [x] `take_control` 后能输入，输出出现在下一次 `drain` 里。

**桌面冒烟程序** `crates/yttt-mobile-core/examples/mobile_tty.rs`：在 Mac 终端里配对开发 Host、列出终端、附加一个，以 60 Hz 调 `drain` 写到 stdout；`--control` 时进入 raw mode，把 stdin 字节交给 `send_input`。观察模式下本地终端比 Host 窄时右侧被截断（自动换行已关），启动时打印 Host 尺寸。

**验收（手动，需要用户在开发 Host 上确认配对）：**
- [x] `mobile_tty` 显示的内容与桌面一致；`seq 1 200000` 时不卡死，记录 `drain` p95。
- [ ] `--control` 下能输入，vim / htop 可用；电脑上点接管后 `mobile_tty` 回到只读。
- [x] 结束后撤销测试设备。（spike Host 的临时 profile 退出即删除，没有留下受信设备。）

# 4：UniFFI 绑定与 iOS 打包

- [x] 用 UniFFI proc-macro 导出第 3 项的 API：`MobileCore` 对象、回调接口 `PairingListener { on_short_code(u32) }`、错误类型。异步函数用 UniFFI async。**偏离：** 不用 `async_runtime = "tokio"`（它经 async-compat 只给一个单线程运行时），core 自带多线程 tokio 运行时，每个 async 方法把工作 spawn 过去再 await `JoinHandle`，任何外部执行器都能 poll。
- [x] `scripts/mobile/build-ios.sh`：编译 `aarch64-apple-ios` 与 `aarch64-apple-ios-sim` 的 staticlib，`uniffi-bindgen generate --library` 生成 Swift 绑定，打包 `YtttMobileCore.xcframework`（产物不入库，`.gitignore` 只新增条目）。

**验收：** 脚本一次跑通；生成的 Swift 文件可被第 5 项工程编译。

# 5：iOS spike App

`mobile/ios/`，xcodegen `project.yml` 生成工程，SwiftUI，SPM 依赖 SwiftTerm（固定到一个发布 tag）。Info.plist 带 `NSLocalNetworkUsageDescription`。

- [x] 页面 1：粘贴 `YTTT-PAIR:1:` 文本 → 显示六位数字 → 配对成功后保存记录。
- [x] 页面 2：终端列表 → 点进页面 3。
- [x] 页面 3：SwiftTerm `TerminalView`，scrollback 0；`CADisplayLink` 每帧 `drain` 并 `feed`；“接管”按钮 → `take_control`，按视图尺寸 `resize`，键盘输入经 `TerminalViewDelegate.send` → `send_input`；角落显示字节/秒、`drain` p95、帧率。
- [x] 观察模式下固定为 Host 列数，放进横向 `ScrollView`。
- [x] 组件侧测试（XCTest）：把第 2 项测试用的几组 viewport 编码结果喂给 SwiftTerm 的 `Terminal`，逐格比较字符与属性，覆盖行尾满列、CJK、颜色与样式。

**验收（模拟器，连开发 Host）：**
- [x] XCTest 通过。
- [x] 配对、连接、观察成功；内容与桌面一致（中文、颜色、光标）。
- [ ] 接管后能在 shell 和一个 Agent CLI 里输入；方向键、Ctrl-C、粘贴可用。
- [ ] `seq 1 200000` 期间界面不卡，记录帧率与 `drain` p95。
- [ ] 若有真机：同一 Wi-Fi 下连一次（会弹本地网络权限），记录路径（直连 / Relay）。蜂窝网络只在 Host 用自建 relay 时能连；Host 用付费 relay 而手机没登录账号时连不上，属于预期。
- [x] 结束后撤销测试设备。（spike Host 的临时 profile 退出即删除，没有留下受信设备。）

# 6：Android spike App

> **偏离：** 第 6 项改用 ConnectBot termlib 0.3.13。Termux 的 `TerminalSession` 是 `final`，必定经 JNI 起本地进程，接远程数据源须 fork，且许可证不明（设计 §8）。下面 Termux 相关的条目作废；termlib 没有公开的屏幕读取 API，组件侧测试改为模拟器截图检查。

`mobile/android/`，Gradle（Kotlin DSL）+ Compose，`cargo-ndk` 编出 `arm64-v8a` 的 `.so`，UniFFI 生成 Kotlin 绑定。

- [ ] iroh 经 reqwest 0.13 依赖 `rustls-platform-verifier`，在 Android 上要在启动时做 JNI 初始化，并引入它的 Maven 组件。
- [ ] 引入 Termux `terminal-emulator` / `terminal-view`（JitPack 固定版本），先核实许可证。
- [ ] 实现远程数据源：不启动本地进程，把 `drain` 字节写进 `TerminalEmulator`，把 `TerminalView` 产生的按键字节交给 `send_input`。记录是否需要 fork。
- [x] 页面与 iOS 相同（配对文本、终端列表、终端），`Choreographer` 每帧 `drain`，`AndroidView` 包装。
- [ ] 组件侧测试（instrumented 或 JVM 单测）：与第 5 项相同的几组输入喂给 `TerminalEmulator`，逐格比较。

**验收（arm64 模拟器，连开发 Host）：** 与第 5 项相同；另记录 `.so` 大小。

# 7：结论

- [x] 在设计文档 §8 写下：三个问题的答案、测得的数据、遇到的坑和绕法、对正式实现的修改建议。

# 8：按终端接管（2026-11-12，master 合入按终端分配之后）

- [x] 分支 rebase 到 master 842d35a（无冲突）。
- [x] `yttt-mobile-core`：`take_control(session, cols, rows)` 发 `TakeTerminal` 并在 Host 应用尺寸后返回；`give_back(session_ids)` 发 `ReturnTerminals`；失去控制改跟 `TerminalLeaseRevoked`/`Released`；`Unsupported` → `HostTooOld`；`repaint(session)`。修复 drain 覆盖本地 geometry epoch 导致接管后输入被静默丢弃（设计 §8 第 9 条）。集成测试改为两个桌面终端：手机只拿走一个、交还、电脑取回，并在接管期间持续 drain。
- [x] iOS：接管前按屏幕算出行列（SwiftTerm 在 iOS 上不公开 `terminal`，用 `getOptimalFrameSize()` 得到字符格大小）；每帧查 `inControl`，失去时回到观察；“交还电脑”按钮；进后台在 `beginBackgroundTask` 内交还全部；Host 太旧时提示。模拟器：第一条命令 `cols=58 rows=53`，进后台再回来回到观察状态。
- [x] Android：接管时先放开 `forcedSize`，等 termlib 的 `onResize` 报出行列再接管；同样的 `inControl` 检查、交还按钮、`onStop` 交还全部、Host 太旧提示；控制状态变化后调用 `repaint`（设计 §8 第 10 条）。模拟器：第一条命令 `cols=63 rows=64`，Home 键再回来回到观察且画面完整。
- 未做：真机、Relay、软键盘尺寸合并（`resizeSuspended`）。
