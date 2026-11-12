# 移动端：配对多台 Host，并在它们之间切换

分支 `feat/mobile-multi-host`，worktree `.worktrees/mobile-multi-host`，基于 master cbeed46。

**现状**

- 手机只存一条配对记录 `host.json`。配对第二台 Host 会覆盖第一台，要换回来只能重新配对。
- Host 在配对时把自己的 `environment_id`（一个 UUID）当作名字发给手机，所以首页顶上显示的是一串 UUID。多台 Host 时无法分辨。

**目标**

- 手机记住多台 Host，同一时间只连当前那台。
- 首页顶部显示当前 Host 的名字，点开可以切换、添加、忘记。
- 设备密钥仍是一份，所有 Host 共用。Host 只认这台设备的公钥，共用不影响安全；每台 Host 各用一份密钥的收益不大，先不做。

每个任务先写失败的测试，再实现，测试通过后提交。编译用 `CARGO_PROFILE_DEV_DEBUG=line-tables-only`，合并后清理构建缓存。

## 任务 1：Host 用电脑名作为配对名字（Rust）

- `crates/yttt-host` 新增 `machine_name()`：取 `gethostname`，为空时退回 `environment_id`。`yttt-server` 里现有的 `host_name()` 改用它。
- `ConfirmPairing` 接受配对时，`host_label` 改为 `machine_name()`。
- 测试：在 `crates/yttt-client-core/tests/p2p_pairing.rs` 配对成功的用例里，断言记录里的 `host_label` 等于 `machine_name()`，并且不等于 `environment_id`。
- 桌面客户端也会因此显示电脑名而不是 UUID；它不依赖这个值等于 `environment_id`（已查过 `src/`）。

## 任务 2：核心给出 Host 的身份（Rust + FFI）

- `MobileCore.hostInfo(record) -> HostInfo { id, label }`，替换现在的 `hostLabel`。
  - `id` 是 Host 的 endpoint key 的字符串形式，用作手机上区分 Host 的键：同一台 Host 重新配对，`id` 不变。
- 单元测试：同一个 `PairedHost` 序列化后读出的 `id` 稳定，两台不同的 Host `id` 不同。

## 任务 3：Host 列表（原生纯逻辑，两端各一份）

`HostList`（Kotlin、Swift 各一份，和 `FileTree` 一样不依赖界面）：

- 内容：若干 `{ id, record }` 和当前 `id`。
- 操作：
  - `added(id, record)`：新 Host 追加到末尾；已有的 Host（重新配对）原位替换记录。两种情况都成为当前 Host。
  - `removed(id)`：删掉；删的是当前 Host 时，当前改为剩下的第一台，没有剩下的就为空。
  - `selected(id)`：切换当前 Host。
  - `names(label: (record) -> String)`：显示名。两台 Host 名字相同时，在后面加 ` · ` 和 `id` 的末 4 位，例如 `MacBook · 3f2a`。
- 存储：一个 `hosts.json`，内容是 `{ "current": id, "hosts": [{ "id", "record": base64 }] }`。
  - 和 `host.json` 一样放在 App 私有目录，不进备份。
  - 启动时如果没有 `hosts.json` 但有旧的 `host.json`，迁移成一台 Host，然后删掉 `host.json`。
- 单元测试覆盖上面每个操作、重名显示、编码往返、迁移。

## 任务 4：Android

- `Link`：
  - `record` 改为由 `HostList` 推导出的当前记录；`savePairing` 改为 `added`。
  - 新增 `switchTo(id)`：断开、切换、清空总控数据、再连接。
  - `forget()` 只忘记当前这台，然后连接剩下的当前 Host（如果有）。
  - 切换或忘记时，`WaitNotifier.cancelAll()`：旧 Host 的等待通知不再有意义。
- 首页顶栏：标题下方显示当前 Host 的名字，点开 `DropdownMenu`：
  - 每台 Host 一行，当前的打勾；
  - “添加 Host”：进入配对页，配对页顶部有返回，回到原来的 Host；
  - “忘记这台 Host”：确认后执行，从左抽屉移到这里，左抽屉里的那一项删掉。
- 前台服务的常驻通知文字带上当前 Host 的名字。

## 任务 5：iOS

- `AppModel` 同 Android：`hosts`、`switchTo`、`forget` 只忘记当前这台。
- 首页导航栏标题下方显示当前 Host 名字，点开 `Menu`，内容同 Android。
- “添加 Host”用 `sheet` 打开配对页，配对成功后关闭。
- 忘记 Host 的入口从左抽屉和设置菜单移到这个菜单。

## 任务 6：spike_host 与验证

- `spike_host --name <名字>`：
  - 状态目录改为 `target/mobile-spike-host-<名字>`，配对码写在这个目录下；
  - 工作区项目名改为 `Spike <名字>`，好在手机上分辨。
  - 不带参数时与现在一致。
  - 两台 spike Host 在同一台 Mac 上，电脑名相同，正好用来验证重名显示。
- 两个模拟器上：
  1. 配对 A，首页显示 `<电脑名>` 和项目 `Spike A`；
  2. 添加 B，首页切到 `Spike B`，菜单里两台 Host 都显示为 `<电脑名> · xxxx`；
  3. 切回 A，看到 `Spike A`；
  4. 重启 App，仍连 A，不要求重新配对；
  5. 忘记 A，自动连上 B；
  6. 再次配对 B（重新配对已有的 Host），列表里仍只有一台。
- Android 额外验证旧版本升级：先装 master 的 APK 配对一台，再覆盖安装本分支，`host.json` 迁移成一台 Host，不重新配对就连上。

## 任务 7：结论

更新移动端设计 §3，在本文末尾写执行记录。

## 不做

- 同时连多台 Host，以及收到非当前 Host 的 Agent 等待通知。要做的话，每台 Host 各要一个连接和一份总控数据，前台服务也要同时维护几条连接，耗电和流量都会增加，等真机上量过单台的耗电再定。
- 在手机上给 Host 改名。
- 每台 Host 各用一份设备密钥。

## 执行记录（2026-11-12）

| 任务 | 提交 | 结果 |
|---|---|---|
| 1 Host 用电脑名 | d1e0a4f | 配对测试加断言，桩下失败，实现后 `p2p_pairing` 全部通过。 |
| 2 `hostInfo` | d273bfc | 单元测试（桩下失败）：读出的 id 稳定，不同 Host 的 id 不同。 |
| 3 `HostList` | b1a68ef | Kotlin、Swift 各 6 项测试，桩下全部失败。Kotlin 测试加入真正的 `org.json`，因为 android.jar 里的只会抛异常。 |
| 4 Android | a6b8934 | 首页 Host 菜单、添加 Host、忘记前确认；左抽屉里的“忘记”移除。 |
| 5 iOS | 6c33da5 | 同上，“添加 Host”用 sheet；`connect` 加了防重入，首页和 Host 变更都会调用它。 |
| 6 spike_host 与修复 | 9574a66 | `--name`；Android 切换时崩溃的修复；常驻通知跟随当前 Host。 |

### 模拟器验证

两台 spike Host（`--name A`、`--name B`）同时运行。它们在同一台 Mac 上，电脑名都是 `WormdeMac-mini.local`，所以菜单里显示为 `… · be6b` 和 `… · a6a3`。

- **Android**：
  1. 先装 master 的 APK，配对 A，存下的是 `host.json`。覆盖安装本分支后，它迁移成 `hosts.json` 里的一台 Host，不重新配对就连上了，A 上没有新的配对请求。
  2. “添加 Host”页显示正常，有返回按钮。用 `adb shell input text` 粘贴 465 个字符的配对码，字符丢失：一次只到 12 个，分段输入也只到 264 个。所以改成由自动化参数 `pair` 在已有 Host 时添加一台，和“添加 Host”走同一个 `pair()`。
  3. 添加 B 时 App 崩溃：刚连上 A 时正在取终端列表，配对 B 导致 A 断开，这个调用抛出 “Host is not connected”，异常逃出已取消的 `LaunchedEffect`，结束了进程。改为捕获，和其他会话调用一样。修复后：
     - 添加 B，首页显示 `Spike B`；
     - 菜单列出两台，当前那台打勾（截图 `scratchpad/hostmenu-s.png`）；
     - 切回 A，显示 `Spike A`；
     - 重启后仍连 A；
     - 忘记 A：先确认，然后自动连上 B，通知里只剩 `Spike B` 的等待；
     - 再次配对 B，列表里仍只有一台；
     - 常驻通知标题跟着切换，在 `保持与 … · be6b 的连接` 和 `… · a6a3` 之间变化；
     - 之后没有再崩溃。
- **iOS**（模拟器不能点击，用自动化参数 `YTTT_SPIKE_PAIR_CODE`、`YTTT_SPIKE_SWITCH`、`YTTT_SPIKE_FORGET`，调用的是菜单用的同一组方法）：
  - 配对 A，再添加 B，显示 `Spike B` 和 `… · a6a3`；
  - 切换到 A，显示 `Spike A` 和 `… · be6b`；
  - 重启后仍连 A；
  - 忘记 A 后连上 B，名字恢复为不带后缀的 `WormdeMac-mini.local`；
  - 再次配对 B，列表里仍只有一台。
  - 菜单本身没有点开验证，只验证了首页上显示的那一行。

### 未验证

- iOS 的 Host 菜单交互和“添加 Host” sheet（模拟器不能点击）。
- 两台 Host 在不同电脑上（这次两台都在同一台 Mac 上）。
