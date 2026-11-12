# 移动端链路验证：Relay、Android TLS、网络切换

分支 `feat/mobile-link`，worktree `.worktrees/mobile-link`，基于 master 7628132。接 `docs/plans/2026-11-11-mobile-client-design.md` §7 第 4 条、§8 第 7、8 条：手机在蜂窝网络上几乎总要经过 relay，spike 全程没有走过；Android 走 HTTPS relay 前要做 `rustls-platform-verifier` 的 JNI 初始化；`IrohEndpoint` 没有封装 iroh 的 `network_change`。这些可能改动 Rust 核心的接口，要在写正式 App 之前弄清楚。

每个任务先写失败的测试，再实现，测试通过后提交。

## 任务 1：只经 relay 的连接（核心 + 自动化测试）

- `RelayChoice` 加 `OnlyFromHost`：使用配对时 Host 给的 relay，且 `DirectPaths::Off`，手机不暴露也不使用直连地址。正式 App 不默认使用；它用于诊断（“只走 relay”开关）和测试。
- `yttt-mobile-core` 增加 dev 依赖 `yttt-relay`。集成测试 `a_phone_works_through_the_relay_alone`：在进程内起一个开放模式的 relay 节点（`127.0.0.1`，HTTP），Host 开启 P2P 并使用该 relay，手机用 `OnlyFromHost` 配对、连接、观察、接管、输入。
- 抽出测试里 Host 启动与配对的公共部分，供两个测试共用。

## 任务 2：网络变化

- `IrohEndpoint::network_change()`：转调 iroh 的 `Endpoint::network_change`，让它立即重新探测路径并重连 relay，而不是等超时。
- `MobileSession::network_changed()` 与 FFI `HostSession.network_changed()`。不发起新的 Session 连接：client-core 自身会在连接断开时重连，这里只缩短 iroh 发现网络变化的时间。
- 集成测试：在会话中调用 `network_changed()` 后，终端输入与输出照常（冒烟测试；真正的切换在任务 5 的模拟器上验证）。

## 任务 3：spike 工具

- `spike_host --relay <url>`：Host 使用该 relay（写入配对码）。
- 两个 App 的自动化参数加 `relay-only`（iOS `YTTT_SPIKE_RELAY_ONLY=1`，Android `--ez relayOnly true`），连接时用 `OnlyFromHost`。FFI `MobileCore.pair`/`connect` 增加 `relay_only: bool` 参数。

## 任务 4：Android TLS 初始化

- `yttt-mobile-core` 在 `cfg(target_os = "android")` 下依赖 `jni = "0.22"`，导出 JNI 函数 `Java_dev_yttt_mobile_PlatformTls_init(env, class, context)`，调用 `rustls_platform_verifier::android::init_with_env`。重复调用无害（内部 `OnceCell`）。
- App 加 `dev.yttt.mobile.PlatformTls`（`external fun init(context: Context)`，`System.loadLibrary("yttt_mobile_core")`），在 `MainActivity.onCreate` 中、任何网络调用之前调用。
- Gradle：按 crate README 加 `rustls-platform-verifier` 的 Maven 仓库（GitHub 上的 `maven-archive` 分支），版本从仓库根的 `Cargo.lock` 读取 `rustls-platform-verifier-android`（当前 0.2.0），保证 Kotlin 组件与 Rust 侧一致。
- 验证：Android 模拟器连一个 HTTPS relay（iroh 官方公共 relay，只转发端到端加密的流量），只经 relay 完成配对、接管、输入。先不初始化跑一次，记录失败的样子，再初始化跑通。

## 任务 5：App 接上网络变化，模拟器验证

- iOS：`NWPathMonitor`，路径变化时调用 `networkChanged()`。
- Android：`ConnectivityManager.registerDefaultNetworkCallback`，`onAvailable`/`onLost`/`onCapabilitiesChanged` 时调用。
- 验证：Android 模拟器只经本机 relay 连接，`adb shell cmd connectivity airplane-mode enable` 后再 `disable`，记录恢复耗时，恢复后接管与输入正常。iOS 模拟器无法切换网络，只验证编译与不崩溃。

## 任务 6：结论

更新移动端设计 §7 第 4 条、§8 第 7、8 条，写下测得的数据和对正式实现的影响；在本文末尾写执行记录。

## 不做

- 真机与真实蜂窝网络（需要设备）。
- 付费 relay 账号与设备码登录（设计 §7 第 7 条，另立）。
- 后台保活与 Android 前台服务（通知功能的一部分，另立）。
