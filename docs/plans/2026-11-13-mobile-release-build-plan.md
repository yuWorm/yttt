# 移动端：自动化参数只留在调试版，正式版能构建、能用

分支 `feat/mobile-debug-hooks`，worktree `.worktrees/mobile-debug-hooks`，基于 master 4f5def8。

**现状**

- 两端都有验证用的自动化参数，任何构建里都生效：
  - iOS 读环境变量 `YTTT_SPIKE_*`：配对码、二维码图片、切换/忘记 Host、打开终端并输入、打开项目/Tab/窗格/抽屉/文件、只走中继。
  - Android 读启动 Intent 的 `pair`、`qrImage`、`type`、`relayOnly`。
  - 这些参数能让 App 不经确认就输入文字、配对别的 Host。正式版不该有。
- Android 的 `open` 不是自动化参数：等待通知点开时靠它打开对应终端，保留。
- 一直只构建、运行过调试版：
  - Android 没有配置 release，没压缩，JNA 和 UniFFI 生成的代码在 R8 下能不能用没试过；
  - iOS 没有用 Release 配置构建过。

**目标**

- 自动化参数只在调试版里存在：正式版的二进制里连这些名字都找不到，而不仅仅是不执行。
- 正式版在模拟器上能配对、看终端、接管输入、编辑文件、收通知。
- 签名仍不做：要用你的 Apple 团队和 Android 签名密钥。本地验证时用调试密钥临时签名，不写进构建配置。

每个任务先写失败的测试，再实现，测试通过后提交。合并后清理构建缓存。

## 任务 1：Android 自动化参数移到 debug 源码集

- `main` 里只留一个数据类 `LaunchHooks(pair, type, relayOnly)`，空值表示没有。
- `src/debug/.../Automation.kt`：`launchHooks(extra, flag, decodeImage)` 从启动参数读出 `LaunchHooks`。参数是函数而不是 `Intent`，好在 JVM 测试里调用。
- `src/release/.../Automation.kt`：同名函数，总是返回空的 `LaunchHooks`。
- `MainActivity` 只调用 `launchHooks`；“重建时不再配对”的判断留在 `MainActivity`。
- 测试：
  - `src/testDebug`：给出 `pair`、`type`、`relayOnly` 时读到它们；有 `qrImage` 没有 `pair` 时用解码结果。
  - `src/testRelease`：同样的参数，读到的是空的。

## 任务 2：iOS 自动化参数只在 Debug 配置编译

- 新文件 `Automation.swift`：`struct LaunchHooks`，`init(environment:)`。
  - `#if DEBUG` 时读 `YTTT_SPIKE_*`；否则所有字段为空，函数体里也不出现这些名字。
  - `static let current = LaunchHooks(environment: ProcessInfo.processInfo.environment)`。
- `AppModel`、`YtttSpikeApp`、`TerminalScreen` 改用 `LaunchHooks.current`，删掉 `AppModel` 里那一串 `static let auto…`。
- 二维码图片的识别（`pairingCode(in:)`）在 `AppModel` 里调用，`LaunchHooks` 只给出路径，测试里不用碰图片。
- 测试（XCTest 跑在 Debug 配置）：每个变量读得到；没设时为空；`YTTT_SPIKE_FORGET=0` 不算忘记。
- Release 配置靠任务 4 检查二进制。

## 任务 3：Android 正式版构建

- `release` 构建类型开启 `isMinifyEnabled` 和 `isShrinkResources`，加 `proguard-rules.pro`：
  - 保留 JNA（`com.sun.jna.**`，它用反射读 `Structure` 字段）；
  - 保留 UniFFI 生成的 `uniffi.yttt_mobile_core.**`（JNA 按名字找回调接口和结构体）。
- 不配置签名。验证时把 `app-release-unsigned.apk` 用 `~/.android/debug.keystore` 临时签名后安装。
- `versionName` 不动，App 名字和包名等你定。

## 任务 4：检查正式版里没有自动化参数

`scripts/mobile/check-release.sh`：

- Android：解开 release APK，在 `classes*.dex` 里找 `qrImage`、`relayOnly`、`YTTT_SPIKE`；找到就失败。
- iOS：在 Release 构建的可执行文件里用 `strings` 找 `YTTT_SPIKE`；找到就失败。
- 先对调试版运行，确认能找到（检查本身有效）；再对正式版运行，确认找不到。

## 任务 5：正式版在模拟器上验证

两端都装正式版，连一台 spike Host：

1. 配对：正式版没有自动化参数。Android 把二维码图片 `adb push` 进相册，在配对页点“从图片识别”选它（`adb input text` 输长配对码会丢字）；iOS 模拟器不能点击，见下；
2. 首页显示项目和 Agent；
3. 打开终端、接管、输入一行；
4. 打开一个文件，改一行，保存；
5. Android：切到后台，Agent 等待时收到通知，点开进入那个终端。

iOS 模拟器不能点击。正式版不能用自动化参数，所以 iOS 只验证：Release 构建能装能启动、显示配对页、不崩溃；用 Release 构建的 Rust 库和同一份 Swift 代码的调试版走一遍上面的流程，区别只在 Swift 的优化级别。如果有办法在模拟器里点击（例如 `xcrun simctl io` 以外的工具），再补完整流程。

## 任务 6：结论

更新移动端设计（新的一节：构建类型），在本文末尾写执行记录。

## 不做

- 签名、App 名字、图标、包名：要你提供或决定。
- 把 spike 的目录和类名改成正式名字：等名字定了一次改完。
- Rust 核心里的 `relay_only` 参数保留：它是连接选项，不是自动化入口；只是正式版的界面不会传 `true`。
