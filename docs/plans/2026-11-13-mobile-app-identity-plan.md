# 移动端：正式的包名、名字和图标

分支 `feat/mobile-app-identity`，worktree `.worktrees/mobile-app-identity`，基于 master 249d8d1。

**决定（2026-11-13）**

- 包名 `com.yttt.yai`：Android 的 `applicationId` 和 Kotlin 包，iOS 的 Bundle ID。
- 名字 `yttt`：桌面图标下和系统设置里显示的名字。
- 图标用桌面的 `assets/app-icon/source/yttt-icon.svg`：深色圆角底，银色的 Y、提示符和光标。

**现状**

- 包名 `dev.yttt.spike`，名字 `yttt spike`，没有图标（Android 是系统默认的绿色机器人，iOS 是空白）。
- 目录和类名带 spike：`mobile/ios/YtttSpike`、`YtttSpikeApp`、Kotlin 包 `dev.yttt.spike`、Gradle 工程 `yttt-spike`。
- 配对时告诉 Host 的设备名是写死的 `Android spike`、`iOS spike`，电脑的设备列表里分不出是哪台手机。
- 调试版的自动化参数叫 `YTTT_SPIKE_*`。

包名变了，系统把它当成一个新 App：spike 版的配对记录和密钥不会带过来，要重新配对一次。spike 版装在手机上的话，两个并存，可以手动删掉旧的。

## 任务 1：图标

新脚本 `scripts/build-mobile-icons.py`，从同一个 SVG 生成两端的图标，生成的文件提交进仓库（和桌面图标一样）：

- **iOS**：`AppIcon.appiconset` 里一张 1024×1024 的 PNG。
  - iOS 自己切圆角，要求不透明。所以深色底铺满整张图、不带圆角，Y 和底的比例与桌面一致。
  - 用 `rsvg-convert` 渲染，去掉 alpha 通道。
- **Android**：自适应图标（minSdk 26 都支持，不需要各种尺寸的 PNG）：
  - 背景层：深色渐变，矢量；
  - 前景层：银色的 Y、提示符、光标，矢量。比例与桌面一致：深色底对应可见的 72dp，图形落在 66dp 的安全圆内；
  - 单色层：同一图形，Android 13 的主题图标用它。
  - 路径和颜色从 SVG 里读出来写成 VectorDrawable，不手抄，改了 SVG 重新运行脚本即可。
- **Android 通知小图标**：同一图形的白色剪影（`ic_stat_yttt`），替换现在借用的系统图标。
- 脚本自查：iOS 图是 1024×1024、没有 alpha；三个路径都读到了。
- 验证：两端模拟器的桌面截图，和桌面版图标并排对比。

## 任务 2：Android 改名

- `namespace`、`applicationId` 改为 `com.yttt.yai`；源码从 `dev/yttt/spike` 移到 `com/yttt/yai`（main、debug、release、三个测试源码集）。
- `YtttSpikeApp` 改为 `YtttApp`；`android:label` 改为 `yttt`；Gradle 工程名改为 `yttt`；`versionName` 改为 `0.1.0`。
- App 私有目录 `yttt-spike` 改为 `yttt`，日志标签改为 `yttt`，停止服务的 action 改为 `com.yttt.yai.STOP_LINK`。

## 任务 3：iOS 改名

- 目录 `YtttSpike` → `Yttt`，`YtttSpikeTests` → `YtttTests`，target 和 scheme 同名；`YtttSpikeApp` → `YtttApp`。
- Bundle ID `com.yttt.yai`，`CFBundleDisplayName` 改为 `yttt`，版本 `0.1.0`。
- Keychain 的 service 改为 `com.yttt.yai`；Application Support 下的目录改为 `yttt`。

## 任务 4：配对时的设备名

- Android：系统设置里的设备名（`Settings.Global.DEVICE_NAME`），没有就用型号（`Build.MODEL`）。
- iOS：`UIDevice.current.name`（iOS 16 起没有特殊授权时是“iPhone”，至少分得出是 iPhone 还是 iPad）。
- 验证：spike Host 打印的配对设备名。

## 任务 5：去掉 spike 时代的迁移

新包名下不会有 spike 留下的文件，这两段迁移不会再走到：

- 明文设备密钥迁进 Keychain / Keystore（两端 `SecretStore` 的 `legacyFile`）；
- 单台 `host.json` 迁成 `hosts.json`（两端 `HostStore`）。

删掉代码和对应的测试。

## 任务 6：自动化参数改名

- iOS 环境变量 `YTTT_SPIKE_*` 改为 `YTTT_DEBUG_*`；`check-release.sh` 的标记跟着改。
- Android 的 Intent 参数名不带 spike，不变。
- 测试 Host `spike_host` 是 Rust 的示例程序，不属于 App，名字不变。

## 任务 7：验证与结论

- 两端单元测试；两端调试版和正式版构建；`check-release.sh` 对两端正式版通过。
- 两端模拟器：装新包名的调试版，桌面上显示 `yttt` 和新图标；配对一台 spike Host，首页正常；Host 那边显示的设备名是手机的名字。
- Android 正式版：通知小图标是新的剪影。
- 更新移动端设计（构建类型那一节加上包名和图标），在本文末尾写执行记录。

## 不做

- 签名：仍要你的 Apple 团队和 Android 签名密钥。
- 启动画面：两端用系统默认（iOS 空白、Android 12 起用图标），以后再说。
- 已有的计划文档里的 spike 名字不改：它们记录的是当时的状态。

## 执行记录（2026-11-13）

| 任务 | 提交 | 结果 |
|---|---|---|
| 1 图标 | 7f66691 | 脚本从 SVG 读出底色渐变、三条路径、银色渐变和变换；iOS 图自查为 1024×1024、不透明。 |
| 2、4、5 Android | 6251c64 | 改包名、名字、版本；接上图标；配对用设备名；删掉两段迁移和对应测试。debug 31 项、release 29 项测试通过（各少了迁移那一项）。 |
| 3、4、5 iOS | 91dc380 | 目录、target、App 类型改名；Bundle ID、Keychain service、版本。21 项测试通过。 |
| 6 脚本和说明 | 5d077c3 | `YTTT_DEBUG_*`；`check-release.sh` 的标记跟着改。 |

**`.gitignore`**：加了 `Yttt.xcodeproj` 和 `Yttt/Info.plist`。旧的 `YtttSpike.xcodeproj`、`YtttSpike/Info.plist` 两条没删：仓库规则要求删 `.gitignore` 条目前先确认。

**一处修正**：XcodeGen 生成的 Info.plist 自带写死的 `CFBundleShortVersionString = 1.0`，设了 `MARKETING_VERSION` 也不生效；改为在 Info.plist 属性里引用 `$(MARKETING_VERSION)` 和 `$(CURRENT_PROJECT_VERSION)`，正式版显示 0.1.0。

### 模拟器验证

- 构建：两端调试版、正式版都能构建；`check-release.sh` 对两端调试版报出参数，对正式版通过。
- **iOS**：
  - 主屏幕上是新图标，名字 `yttt`；
  - Debug 版用改名后的 `YTTT_DEBUG_PAIR_CODE` 配对成功，首页正常；
  - Host 的设备列表（`devices.json`）里记的是 `iPhone 16`（模拟器的名字）。
- **Android 正式版**（本地用调试密钥签名）：
  - 应用列表里是新图标，名字 `yttt`，圆形遮罩下图形完整；
  - 从相册图片配对成功，Host 记的设备名是 `sdk_gphone64_arm64`（模拟器设置里的设备名）；
  - Agent 等待的提醒通知、常驻通知和弹出横幅都用新的剪影图标。

### 未验证

- Android 13 的主题图标（单色层）：要在壁纸设置里打开主题图标，没有试。
- 真机。
