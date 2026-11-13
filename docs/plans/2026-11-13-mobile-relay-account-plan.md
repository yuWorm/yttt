# 手机端 Relay 账号登录：计划

- 日期：2026-11-13
- 分支：`feat/mobile-ios-device`（worktree `.worktrees/mobile-ios-device`）
- 依据：[`2026-11-11-mobile-client-design.md`](2026-11-11-mobile-client-design.md) 未决问题 7；`yttt-control-client`；桌面 `src/relay_account.rs`、`src/remote_host.rs`、`src/ui/app/paired_host.rs`

## 问题

真机（iPhone 16 Pro，iOS 26.6.2）配对本机 Host 失败。手机日志（`YTTT_LOG`）显示两件事：

1. 付费 relay（`hk2.yctn.yai.best`）每次都拒绝手机：`The relay denied our authentication (a relay ticket is required)`。手机端没有 Relay 账号，拿不到 ticket，relay 对它完全不可用；在外网手机根本连不上。
2. 局域网直连握手做了一半：手机认为已连上 `192.168.1.105`，Host 端 30 s 后 `an incoming connection failed error=timed out`。手机和电脑都开着 TUN 代理（`198.18.0.1`）。另查，不在本计划内。

用户要求：先登录，再连接。

## 约束（来自控制面）

- 设备 key 以 `Client { host }` 注册后，角色和所跟的 Host 都不能改（`placement::register`：`the device's role and Host cannot change`）。手机现在所有 Host 共用一个 device secret，登录后第二台 Host 就注册不上。
- 浏览器登录（PKCE）只回调 loopback，手机用不了；设备码流程（RFC 8628，`/v1/oauth/device`）和邮箱密码（`/v1/sessions`）都可用。
- Ticket 有效期至少 1 h（`ticket_ttl` 下限 3600 s）；iroh 只在新 relay 连接上用新 token，长连的 endpoint 需要在首个 ticket 过期前重建。手机前后台切换、换网都会重建会话，`keep_ticket` 续期即可。

## 设计

### 核心（`yttt-mobile-core`）

1. **每台 Host 一把 key**：`keys::host_key(device_secret, host) = SHA-256("yttt-mobile host key v1" ‖ device_secret ‖ host)`。配对和连接都用它，两端存储不变。同一 Host 重新配对得到同一把 key（控制面上同角色同 Host，可重复注册）。已有配对要重新配对（应用未发布，不做迁移）。
2. **账号**（新模块 `account.rs`，经 FFI 导出）：
   - `sign_in_password(control_url, email, password) -> SignedIn`；
   - `begin_device_sign_in(control_url, label) -> DeviceSignIn`（对象：`user_code`、`verification_url`（带 code）、`finish()`、`cancel()`）；
   - `account_info(session) -> AccountInfo`（邮箱、套餐、设备数）；
   - `SignedIn { session: bytes, email, control_url }`；`session` 是 `ControlSession` 的 JSON，由应用存进 Keychain / Keystore 加密文件。
   - 401 映射为 `MobileCoreError::SignInExpired`，应用据此退出登录并提示重新登录。
3. **配对**：`pair(…, account: Option<bytes>)`。有账号时先 `register_device(host_key, label, Client{host})` + `ticket`，用 ticket 的 relay 与 token；配对失败则 `remove_device`，不占套餐的设备名额。拿不到 ticket 时退回配对码里的 relay 提示（只对自建 relay 有效），日志记录原因。
4. **连接**：`connect(…, account: Option<bytes>)`。最多等 5 s 拿 ticket（同桌面 `TICKET_WAIT`），拿到后 `keep_ticket` 续期，任务随会话关闭；拿不到退回 relay 提示。`MobileHost.relay_source()` 告诉界面现在用的是账号 ticket、配对提示还是无 relay。
5. **忘记 Host**：`release_host(account, device_secret, host_id)` 从账号移除这把 key。
6. **测试**：`host_key` 单测；会话 JSON 往返、401 映射单测；集成测试（`#[ignore]`，需 PostgreSQL，同 `yttt-client-core/tests/paid_relay.rs`）：只接受 ticket 的本地 relay 节点 + 控制面 + Host，手机用邮箱密码登录，`OnlyFromHost` 经 relay 配对、连接、拿到概要；未登录时同样的配对失败。

### iOS

- `AccountStore`：Keychain 条目 `relay-account`，存 `SignedIn`（不随 iCloud 同步）。
- `AppModel`：`account` 状态；配对、连接传入账号；登录或退出后重连所有 Host；`SignInExpired` 时退出登录并在设置与总控提示。
- 界面（原生 Form / NavigationStack）：
  - 设置页新增“Relay 账号”一节：未登录显示“登录”；已登录显示邮箱、控制台地址、设备数，“退出登录”（确认）。
  - 登录页：控制台地址（记住上次输入）；主按钮“在浏览器中登录”——显示设备码，用 `SFSafariViewController` 打开登录页，后台轮询，确认后自动关闭；次要“用邮箱和密码登录”。
  - 配对页顶部：未登录时一张卡片“先登录 Relay 账号，手机在外面也能连上电脑”［登录］，可跳过（只用局域网或自建 relay）。
  - 首次启动（没有 Host）：总控空状态分两步：① 登录 Relay 账号 ② 配对电脑。

### Android

同 iOS：`AccountStore`（复用 Keystore AES 密钥，`relay-account.enc`，`writeDurably`）；设置页一节、登录页（设备码用浏览器 `ACTION_VIEW` 打开，回到应用时轮询仍在）、配对页卡片、首次启动两步。

### 其他

- `YTTT_LOG`（已加）：核心日志写到 stderr，供真机调试；正式版不设置它。
- `scripts/mobile/run-ios-device.sh`（已加）：签名、安装、启动到连接的 iPhone；team 取自唯一的 Apple Development 证书或 `YTTT_IOS_TEAM`，不进仓库。

## 验证

1. 核心单测与集成测试（本机 Docker 起 PostgreSQL，测完删除容器）。
2. iOS 模拟器与 Android 模拟器：登录页、设置页、配对页卡片走一遍（对测试控制面）。
3. 真机：用户在 iPhone 上登录自己的 Relay 账号，配对本机 Host，经 relay 连接；关掉 Wi-Fi 走蜂窝再连。

## 不做

- 局域网直连在 TUN 代理下握手超时：另查。
- 浏览器 PKCE 登录（需控制面支持非 loopback 回调）。
