# 移动端：设备密钥放进 Keychain / Keystore，扫码配对

分支 `feat/mobile-secure-pairing`，worktree `.worktrees/mobile-secure-pairing`，基于 master 90eee22。

**现状**

- 设备的 32 字节密钥是配对后手机证明身份的凭据，现在两端都以明文文件存在 App 目录里。
- 配对要粘贴 `YTTT-PAIR:1:…` 文本。电脑的配对页和 `yttt-server p2p pair` 已经显示同一段文本的二维码。

**决定（2026-11-12）**

- Android 扫码用 ZXing core 3.5.4（Apache-2.0）加 CameraX 1.6.2，不依赖 Google 服务。
- iOS 用系统的 VisionKit 和 CoreImage。

每个任务先写失败的测试，再实现，测试通过后提交。合并后清理构建缓存。

## 任务 1：iOS 密钥进 Keychain

- 新建 `SecretStore`：Keychain 通用密码项，service `dev.yttt.spike`，account `device-secret`。
  - 可用性设为 `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`：开机解锁一次之后后台也能读到；不进 iCloud 钥匙串，不随备份迁移到别的设备。
- 首次启动时迁移：读旧的明文文件，写进 Keychain，确认写成功后删掉旧文件。
- 配对记录 `host.json` 本身不是密钥，留在 Application Support 里，但设为不备份（`isExcludedFromBackup`）。
- 测试：
  - XCTest 覆盖“没有时生成并保存”“再读拿到同一份”“从旧文件迁移后旧文件被删”。
  - 测试 target 没有宿主 App，Keychain 可能因为缺 entitlement 失败（`errSecMissingEntitlement`）。如果是这样，把测试改成以 App 为宿主运行。

## 任务 2：Android 密钥进 Keystore

- 新建 `SecretStore`：在 `AndroidKeyStore` 里生成一把不可导出的 AES-256-GCM 密钥 `yttt-device-secret`，用它加密设备密钥。
  - 文件 `device-secret.enc` 的内容是 12 字节 IV 加上密文。
  - 首次启动迁移旧的明文文件，成功后删除。
- `AndroidManifest` 设 `android:allowBackup="false"`，配对记录不进云备份。
- 测试：
  - 加解密和文件格式用可注入的 `Cipher` 工厂，在 JVM 单元测试里用软件 AES 密钥验证往返、篡改后解不开、迁移逻辑。
  - 真正的 Keystore 在模拟器上验证：启动后旧明文文件消失，只有 `.enc`，重启 App 仍能用同一密钥重连（Host 不要求重新配对）。

## 任务 3：iOS 扫码配对

- 配对页增加两个入口：
  - “扫描二维码”：`DataScannerViewController`，只认 QR，识别到 `YTTT-PAIR:` 开头的文本就关闭并开始配对。设备不支持（模拟器）时按钮不可用并说明原因。
  - “从照片识别”：`PhotosPicker` 选图，用 `CIDetector`（QR）取文本。
- `Info.plist` 增加 `NSCameraUsageDescription`。
- 纯函数 `pairingCode(in image:) -> String?`：从图片里找第一个 `YTTT-PAIR:` 开头的二维码。XCTest 用 `CIFilter.qrCodeGenerator` 生成二维码图片来测：
  - 能取到；
  - 不是配对码的二维码返回 nil；
  - 没有二维码的图片返回 nil。
- 自动化参数 `YTTT_SPIKE_QR_IMAGE=<路径>`：启动时用同一函数识别图片并配对。模拟器上用它验证识别、配对这条路。

## 任务 4：Android 扫码配对

- 配对页增加：
  - “扫描二维码”：先申请相机权限，再用 CameraX 的 `PreviewView` 加 `ImageAnalysis`。每帧取 Y 平面，交给 ZXing 的 `QRCodeReader`；识别到 `YTTT-PAIR:` 就停下并配对。
  - “从图片识别”：系统照片选择器选图，同一个解码函数处理。
- 纯函数 `decodePairingCode(luminance, width, height): String?`。JVM 单元测试用 ZXing 的 `QRCodeWriter` 生成二维码，转成亮度数组来测，情形与 iOS 相同。
- Manifest 加 `CAMERA` 权限，并声明 `uses-feature camera` 为非必需。
- 模拟器验证：
  - 用 `-camera-back virtualscene` 加 `-virtualscene-poster wall=<二维码图片>` 让模拟相机拍到二维码。如果画面里看不到海报，就用自动化参数 `qrImage` 走图片识别这条路；
  - 两条路都要真的完成配对。

## 任务 5：结论

更新移动端设计 §3（存储、扫码）。在本文末尾写执行记录。

## 不做

- 不删除 spike 的自动化参数：它们是验证手段，正式 App 拆分时再按构建类型去掉。
- 不做多台 Host（每台 Host 一条配对记录，下一步再做）。
- 密钥不要求生物识别：后台通知要在锁屏后也能重连。
