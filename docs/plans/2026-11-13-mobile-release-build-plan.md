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

## 执行记录（2026-11-13）

| 任务 | 提交 | 结果 |
|---|---|---|
| 1 Android 自动化参数 | cdfbc8b | debug 的 3 项测试在桩下失败 2 项，实现后通过；release 的 1 项测试读到的是空的。 |
| 2 iOS 自动化参数 | 071ce07 | 3 项测试，读变量的那项在桩下失败，实现后通过，全部 22 项通过。 |
| 3 Android 正式版 | 3c0c4a9 | R8 加 JNA、UniFFI 的保留规则，构建无警告。AGP 9 默认只跑 debug 的单元测试，为 release 另外打开。 |
| 4 检查脚本 | 3c0c4a9、5ec1217 | 见下。 |
| iOS Release 链接 | 071ce07 | Release 默认构建所有架构，x86_64 模拟器链接失败（核心只有 arm64）。模拟器构建排除 x86_64。 |

### 检查脚本

- Android：debug APK 在 `classes3.dex` 里找到 `qrImage`，失败；release APK 通过。
- iOS：Debug 的 `YtttSpike.debug.dylib` 里找到 `YTTT_SPIKE`，失败；Release 通过。
- 第一次 Release 链接失败时，留下的 `.app` 只有 92 KB、没有可执行文件，脚本却判为通过。改为先确认可执行文件存在，否则报错退出。

### 模拟器验证

体积：Android APK 从 40 MB（debug）到 22 MB（release）；iOS `.app` 从 46 MB（Debug）到 30 MB（Release）。两者用的都是 release 的 Rust 核心，差别来自压缩和 Swift 优化。

- **Android 正式版**（R8 压缩，用调试密钥临时签名后安装）：
  1. 带 `-e pair …` 启动，停在配对页，没有开始配对；
  2. 把二维码图片 `adb push` 进相册，点“从图片识别”选中，配对成功，首页显示项目和两个 Agent；
  3. 打开 `spike:shell:1`，接管，输入 `echo releaseOK`，Host 上执行了；
  4. 右抽屉打开 `README.md`，加一行后保存，Host 上的文件变了；
  5. 回到桌面后，Agent 等待时发出提醒通知、等待结束时撤回，持续了十几分钟。点开 Code review 的通知，进入那个 Tab。
- **iOS Release**（模拟器不能点击）：
  - 设了 `YTTT_SPIKE_PAIR_CODE` 启动，停在配对页，没有配对。对照：Debug 构建同样的启动直接配对，进入首页。
  - 把 Release 覆盖安装在已配对的 Debug 上，不重新配对就连上了，首页的 Agent 状态随 Host 变化。说明 Release 的 Swift 代码、Keychain 里的密钥和存下的 Host 都能用。
  - 打开终端、接管输入、编辑文件没有在 Release 上走过：要点击。

### 两个误判

- 第一次等后台通知时等了两分钟没看到，以为后台不通知。其实是检查脚本写错了：在 `dumpsys` 里通知标题离 channel 行不止 12 行。系统事件日志里这段时间的通知照常发出和撤回。
- 点 Build 的通知却进了 Code review、或回到首页。点击日志表明，点到的分别是 Code review 的通知和分组摘要：Build 的等待只有约 16 秒，从找到它的位置到点击之间它已撤回，下面的通知移了上来。用调试版点 Build 进入 Build，正式版点 Code review 进入 Code review，行为一致。

### 未验证

- iOS Release 的点击操作（终端、接管、编辑）。
- 真机上的正式版：两端都还没有签名。
