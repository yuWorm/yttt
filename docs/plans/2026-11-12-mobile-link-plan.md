# 移动端链路验证：Relay、Android TLS、网络切换

分支 `feat/mobile-link`，worktree `.worktrees/mobile-link`，基于 master 7628132。接 `docs/plans/2026-11-11-mobile-client-design.md` 的三处：§7 第 4 条，以及 §8 第 7、8 条。

- **relay**：手机在蜂窝网络上几乎总要经过 relay，但 spike 全程没有走过。
- **Android TLS**：设计 §8 第 7 条认为 Android 上要先做 `rustls-platform-verifier` 的 JNI 初始化。审查后核实这条不对，见任务 4。
- **网络变化**：`IrohEndpoint` 没有封装 iroh 的 `network_change`。

这些可能改动 Rust 核心的接口，要在写正式 App 之前弄清楚。每个任务先写失败的测试，再实现，测试通过后提交。

## 任务 1：只经 relay 的连接（核心 + 自动化测试）

**RelayChoice**

- 加 `OnlyFromHost`：使用配对时 Host 给的 relay，同时 `DirectPaths::Off`，手机不暴露也不使用直连地址。
- 它用于诊断（“只走 relay”开关）和测试，正式 App 不默认使用。
- `bind()` 目前写死了 `DirectPaths::Any`（`session.rs:98`），所以 `RelayChoice::setup` 改为同时返回 `RelaySetup` 和 `DirectPaths`。
- `OnlyFromHost` 遇到 Host 没有 relay 时立即返回 `MobileError::Transport("the Host has no relay")`。否则得到一个哪里都连不到的 endpoint，要等 30 s 连接超时才报错。

**集成测试 `a_phone_works_through_the_relay_alone`**

- `yttt-mobile-core` 增加 dev 依赖 `yttt-relay`。
- 在进程内起一个开放模式的 relay 节点：`NodeConfig::local(.., "127.0.0.1:0", &[])` 加上 `open`，地址 `http://{addr}`。
- Host 用 `SetP2p { relay_url }` 使用该 relay，手机用 `OnlyFromHost` 配对、连接、观察、接管、输入。
- 测试必须证明流量确实经过 relay（照 `crates/yttt-client-core/tests/paid_relay.rs` 的做法）。否则，如果 `OnlyFromHost` 误接到 `DirectPaths::Any`，测试会经配对码里的局域网地址照样通过。具体做法：
  - 配对前等节点上出现 Host 的连接；
  - 输入后断言节点记录的手机发送字节数大于 0。
- 开放模式的账号 ID 由 endpoint 公钥算出（`yttt-relay/src/node.rs` 里是 `pub(crate)`）。在 `yttt-relay` 导出 `pub fn open_account_id(endpoint: &[u8; 32]) -> u64` 供测试使用，不在测试里复制公式。
- 抽出测试里 Host 启动与配对的公共部分，两个测试共用。

## 任务 2：网络变化

- 加 `IrohEndpoint::network_change()`，转调 iroh 的 `Endpoint::network_change`。
  - iroh 会在 250 ms 去抖后重新读网卡；只有真的有变化时，才会重绑 socket、检查 relay 连接、重新 STUN。
- 加 `MobileSession::network_changed()`，做两件事：
  - 有 endpoint 时调用上面的 `network_change()`；
  - 调用 `ClientCore::probe_now()`：连接 5 s 内没有回音就换掉，正在退避的重连立即重试。桌面的 `src/link_watch.rs` 在网络变化时也是这么做的。
  - 没有这一步，坏掉的连接要等 20 s 的存活超时才会被发现。Android 上 netwatch 不监听路由变化，它看到的状态可能根本不变，这时只能靠 `probe_now()`。
  - 不发起新的 Session 连接，`client_instance_id` 每次连接新生成的规则不受影响。
- FFI `HostSession.network_changed()` 是同步调用，内部用 runtime spawn 执行，调用方不等结果。
- 集成测试断言可观察的结果：
  - 调用很快返回；
  - 之后输入与输出照常；
  - `close()` 之后再调用，既不 panic 也不卡住。
  - 真正的网络切换在任务 5 的模拟器上验证。

## 任务 3：spike 工具

- `spike_host --relay <url>`：Host 使用该 relay，地址会写进配对码（`examples/spike_host.rs:155` 的 `relay_url`）。
- 本机 relay：`cargo build --release -p yttt-relay && ./target/release/yttt-relay --config packaging/relay-selfhost/relay.toml`。开放模式，监听 `0.0.0.0:3341`，见 `docs/p2p-testing.md` §4。
- Host 的 relay 地址用 Mac 的局域网 IP（`http://192.168.x.y:3341`），不用下面两种：
  - `127.0.0.1`：对模拟器来说是它自己；
  - `10.0.2.2`：Host 那边不能用。
  - `adb reverse` 也不行：它走 adb 的回环，飞行模式切不断它，网络切换测试会白测。
- 两个 App 的自动化参数加只走 relay（iOS 用 `YTTT_SPIKE_RELAY_ONLY=1`，Android 用 `--ez relayOnly true`）。FFI `MobileCore.pair` 和 `connect` 增加 `relay_only: bool`。

## 任务 4：Android TLS（核实，不做 JNI）

- 现状：
  - `yttt-transport-iroh` 构建 iroh 时没有开 `platform-verifier`；
  - `IrohEndpoint::bind` 不设 CA 配置，iroh 用默认的 `CaTlsConfig`，也就是内置的 webpki-roots；
  - iroh 把同一份 TLS 配置交给 reqwest，所以 `rustls-platform-verifier` 虽然链接进来了，却从不被调用；
  - 控制面客户端用的 reqwest 0.12 加 `rustls-tls`，同样是 webpki-roots。
  - 结论：不需要 JNI 初始化，桌面与手机的信任根一致。
- 验证：Android 模拟器只经一个 HTTPS relay（iroh 官方公共 relay，只转发端到端加密的流量），完成配对、接管、输入，证明 TLS 可用。
- 记录取舍：内置根证书不认用户自装或企业的 CA，所以用私有 CA 的自建 relay 在桌面和手机上都连不上。要支持这种 relay，需要给 Android 开 iroh 的 `platform-verifier` 并设 `CaTlsConfig::system()`，那时才需要 JNI 初始化和 Kotlin 组件。具体坑见本文末尾的附注。

## 任务 5：App 接上网络变化，模拟器验证

- iOS：`NWPathMonitor`，路径变化时调用 `networkChanged()`。
- Android：`ConnectivityManager.registerDefaultNetworkCallback`，在 `onAvailable`、`onLost`、`onLinkPropertiesChanged` 时调用，并在原生侧去抖 500 ms。
  - 不接 `onCapabilitiesChanged`：信号强度、带宽变化都会触发它，而 Android 上 netwatch 每次都要起一个 `ip route` 进程。
  - `ACCESS_NETWORK_STATE` 权限已经在 manifest 里。
- 验证：Android 模拟器只经本机 relay 连接，执行 `adb shell cmd connectivity airplane-mode enable`，再 `disable`。
  - 记录恢复耗时；
  - 记录 iroh 有没有看到变化（日志 “link change detected”）、ClientCore 有没有重连（`diagnostics().reconnects`），以区分是原 QUIC 连接撑过来了，还是重建了会话；
  - 恢复后接管与输入正常。
- iOS 模拟器无法切换网络，只验证编译通过、运行不崩溃。

## 任务 6：结论

- 更新移动端设计 §7 第 4 条和 §8 第 7、8 条，写下测得的数据和对正式实现的影响。
- 正式 App 里 `network_changed` 应放在 `MobileCore` 上，转给所有活着的 endpoint，这样配对、连接进行中或者连着多台 Host 时也能收到。spike 先放在 `HostSession` 上。
- 在本文末尾写执行记录。

## 不做

- 真机与真实蜂窝网络（需要设备）。
- 付费 relay 账号与设备码登录（设计 §7 第 7 条，另立）。
- 后台保活与 Android 前台服务（通知功能的一部分，另立）。

## 附注：以后改用系统信任根时

- mobile-core 要直接依赖 Android 专用的 `rustls-platform-verifier = "0.7.1"`，与 iroh 的 reqwest 同版本，共用同一个全局状态。
- 用 jni 0.22 的写法：`#[jni::jni_mangle("<包名>.PlatformTls")]`、`EnvUnowned`、`with_env(..).resolve::<ThrowRuntimeExAndDefault>()`。0.22 里已经没有 `JNIEnv`。
- `init_with_env` 本身不需要 Kotlin 类存在；首次校验证书时才加载 Kotlin 组件。没装组件只会让 TLS 校验失败；没做初始化则会在校验时 panic。
- Gradle 方面：
  - `settings.gradle.kts` 用的是 `FAIL_ON_PROJECT_REPOS`，maven 仓库要写在 `dependencyResolutionManagement.repositories` 里；
  - 依赖坐标是 `rustls:rustls-platform-verifier`，group 是 `rustls`，不是 README 里写的 `org.rustls`；
  - `Cargo.lock` 相对 app 模块在 `../../../`。
- JNI 函数名必须与 Kotlin 类的完整类名一致，类要放在 App 的包里。

## 执行记录（2026-11-12）

审查子代理读过初版计划后发现两处事实错误，已改入上文：Android 不需要 TLS 的 JNI 初始化；`network_changed` 还要调 `probe_now`。

| 任务 | 提交 | 结果 |
|---|---|---|
| 1 只经 relay | 2b354b5 | `RelayChoice::OnlyFromHost`、`MobileSession::path()`（FFI `path()`，供界面显示“直连／经 relay”）。集成测试起进程内开放 relay，断言 2 s 后路径仍为 relay、手机经 relay 发送的字节数大于 0。只断言字节数不够：把 `OnlyFromHost` 改成允许直连，iroh 先经 relay 再打洞，字节数照样大于 0；加上路径断言后，这个变异 3 次全部失败。 |
| 2 网络变化 | c50a5b8 | `IrohEndpoint::network_change`、`MobileSession::network_changed`（iroh 重读网卡 + `probe_now`）、FFI 同步调用后台执行。测试：立即返回，健康会话不因探测重连，`close()` 后调用无害。 |
| 3 spike 工具 | 1869435 | `spike_host --relay`，FFI `pair`/`connect` 的 `relay_only`，两个 App 显示路径。 |
| 4 Android TLS | — | 不改代码。Android 模拟器只经公共 HTTPS relay 配对、接管、输入成功，没有 JNI 初始化。第一次配对失败（`InvitationClosed`）：spike_host 确认设备后立刻发新邀请，关掉了还没把确认结果送到设备的旧邀请；改为确认后等 5 s（在 5fdf6e5 里）。 |
| 5 App 网络回调 | 5fdf6e5 | 见下方数据。 |

### 断网恢复（Android 模拟器，只经 aps1-1 relay，飞行模式 10 s）

从关闭飞行模式到 demo 终端（每秒输出一次）恢复输出：

| 做法 | 各次耗时 | 说明 |
|---|---|---|
| 不接网络回调 | 16 s、11 s、11 s | 3 次中 2 次 QUIC 连接撑过了断网，没有重连 |
| 断网和恢复时都调用 | 10 s、14 s、8 s、20 s | 每次断网后 5 s 内会话都被换掉，恢复后要经 relay 重建 |
| 只在网络可用时调用（采用） | 10 s、14 s、9 s | 两次仍被换掉：第一个可用事件来得早，网络还没真正通 |

模拟器恢复 Wi-Fi 本身要 2–7 s，波动大，三种做法的差别在噪声以内。能确定的只有一点：断网时探测是负作用，所以不在断网时调用。网络回调真正有用的情形（Wi-Fi 与蜂窝之间换 IP、旧路径立即失效）只能在真机上测。

### 其他发现

- Android 日志里有 SELinux 拒绝：App 读不了 `/sys/class/net`，也读不了 netlink 路由（`nlmsg_readpriv`）。iroh 的 netwatch 在 Android 上看不到网卡变化，`network_change` 多半什么也不做，起作用的是 `probe_now`。
- 经远处 relay 配对约一分钟（设计 §8 第 11 条）。
- iOS 只验证了编译；模拟器无法切换网络。

### relay 复测（2026-11-12 下午，iOS 与 Android）

spike_host 分别接本机自建 relay（`yttt-relay --config packaging/relay-selfhost/relay.toml`，`http://192.168.1.36:3341`）和公共 HTTPS relay（`https://aps1-1.relay.n0.iroh.link`）；App 每次重新安装后只经 relay 配对、打开 shell、接管、输入。配对耗时从启动 App 算到 App 写下 `host.json`：

| relay | 平台 | Host 确认 | 配对完成 | 结果 |
|---|---|---|---|---|
| 本机 HTTP | iOS 模拟器 | +2.0 s | +2.0 s | 经 relay；第一条命令看到手机尺寸 |
| 本机 HTTP | Android 模拟器 | +3.0 s | +3.0 s | 经 relay；`cols=63`；从 iOS 手里接管同一终端 |
| 公共 HTTPS | iOS 模拟器 | +5.1 s | +5.6 s | 经 relay |
| 公共 HTTPS | Android 模拟器 | +5.6 s | +6.2 s | 经 relay；Android 无 TLS 初始化 |

再以默认模式（`FromHost`，不只走 relay）打开 demo 终端：两边都显示“直连”，tick 同步。之前“经远处 relay 配对约一分钟”没有复现（设计 §8 第 11 条）。
