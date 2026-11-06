# P2P 实测指南

本文写给在真机上验证 P2P 配对与连接的人。三种场景由易到难：同一局域网、跨网络经 relay、只走 relay。每一步都给出成功时应看到的现象，便于定位问题出在哪一环。

## 0. 准备

- 两台机器都用 `feat/p2p-host` 分支构建。桌面端：`cargo build --release`；无桌面的 Linux 服务器只需要 `yttt-server`：`cargo build --release -p yttt-server`。
- 每台 Client 对应每台 Host 有一把独立的设备密钥，存在系统凭据库（macOS 钥匙串、Windows 凭据管理器）。
- 路径类型（直连或 relay）显示在两处：
  - 远程窗口底部状态栏：`P2P · 连接中 / 直连 / 经 relay`；
  - Client 日志 `logs/client.log`：`P2P path: direct` / `P2P path: relay`。日志目录可用 `yttt --open-logs` 打开。

## 1. 让一台机器当 Host

### 1a. 有桌面

“远程服务”窗口 → “远程访问此电脑” → “P2P 配对” → 开启。relay 留空表示只允许直连，跨网络测试再填。

### 1b. 无桌面（SSH 登录的服务器）

先确保 Host 在跑：

```sh
yttt-server ensure            # 已在跑则直接返回
yttt-server p2p enable        # 只直连
yttt-server p2p enable --relay http://relay.example.com:3341   # 经 relay
yttt-server p2p status
```

用桌面端经 SSH 连过这台机器，`yttt-server` 就已经装在 `~/.local/share/yttt/server/<版本>/<平台>/yttt-server`。也可以自己构建后放在任意位置。Host 只在显式停止时退出；要在机器重启后自动运行，见 `packaging/linux/yttt-host.service`。P2P 开关会被保存，Host 重新启动后自动恢复。

## 2. 配对

1. 在 Host 上开邀请：
   - 有桌面：点“配对新设备”，会显示二维码；需要文本时点“显示配对码”或“复制配对码”。
   - 无桌面：`yttt-server p2p pair`，终端会打印二维码和配对码，然后等待设备。
2. 把配对码交给设备（单次有效，5 分钟内用）。
3. 在设备上：“远程连接” → “添加连接” → “配对 Host（P2P）” → 粘贴 → “配对”或“保存并连接”。
4. 设备上会显示 6 位数字。到 Host 上核对：
   - 有桌面：数字一致才点“只允许查看”或“允许完全控制”，否则点“拒绝”。
   - 无桌面：终端显示同样的数字，输入 `v`（只看）、`f`（完全控制）或其它（拒绝）。
5. 成功后设备的列表里出现这台 Host，标签为 “P2P”。

没有图形界面的设备也能测：在设备上运行

```sh
cargo run --release -p yttt-client-core --example p2p_probe -- '<配对码>' --seconds 60
```

它会完成配对，然后每秒打印一次路径和请求耗时。加 `--relay-only` 可强制只走 relay。测完在 Host 上撤销它（`yttt-server p2p devices` 查看密钥，`yttt-server p2p revoke <密钥>`）。

## 3. 场景与预期

| 场景 | 设置 | 预期 |
|---|---|---|
| 同一局域网 | Host 不设 relay | 连接后状态栏为“直连”，请求耗时在几毫秒级 |
| 跨网络 | Host 与设备使用同一个 relay（见 §4） | 先“经 relay”，打洞成功后几秒内变为“直连”；打不通（对称 NAT 等）则一直“经 relay”，但能正常工作 |
| 只走 relay | 用 `p2p_probe --relay-only` | 始终“经 relay”，Host 与设备都不暴露直连地址 |
| 断网恢复 | 连接后切换 Wi-Fi 或断网再恢复 | 窗口提示重连，网络恢复后自动回到可用状态，`client.log` 记录断开与恢复耗时 |
| 撤销 | 在 Host 上撤销设备 | 设备会话立即断开，之后无法再连；重新配对后恢复 |

## 4. 自建 relay（跨网络测试需要）

relay 只转发端到端加密的流量，看不到内容，也不能冒充任何一端。需要一台两端都能访问的机器（例如那台 SSH 服务器）：

```sh
cargo build --release -p yttt-relay
./target/release/yttt-relay --config packaging/relay-selfhost/relay.toml
```

- 默认监听 TCP 3341，地址形如 `http://<公网 IP>:3341`。防火墙或云安全组需放行该端口。
- 有证书时按配置文件里的注释改用 HTTPS，并开启 UDP 7842 的 QUIC 地址探测，打洞成功率更高。
- 自建模式不需要账号，任何拿到地址的人都能用它的带宽（每台设备限速），不要公开地址。
- 也可以用官方镜像 `n0computer/iroh-relay`（见 `packaging/relay-dev/docker-compose.yml`），但它的 `--dev` 模式只适合本机。

Host 开启 P2P 时填这个地址；设备不用单独设置，配对码里带着 Host 的 relay 地址。

## 5. 付费 relay（账号）

需要控制面和计费 relay 节点，本地可以用 docker 起一整套（见 `packaging/relay-dev/docker-compose.full.yml`，控制面在 `http://127.0.0.1:18080`，节点在 `http://127.0.0.1:3341`）。真机测试时把两者部署到双方都能访问的机器上，并把控制面配置里节点的 `url` 改成那台机器的地址。

1. 建账号（开发模式）：`curl -X POST http://<控制面>/v1/dev/accounts -H 'content-type: application/json' -d '{"email":"me@example.com","password":"至少八位密码","plan":"basic","node":"local-1"}'`。
2. Host 与设备都在“远程访问此电脑”页的“relay 账号”一节用同一账号登录。
3. Host 上点“本机作为 Host 时使用此账号”。Host 会登记自己的密钥并取得 ticket，P2P 一节显示正在使用的 relay。
4. 照常配对：设备已登录时，配对与之后的连接都经账号的 relay 节点，即使配对码里的 relay 地址不同。
5. 在 Host 页面的账号信息里应能看到本月用量增长；在控制面封停账号（`POST /v1/dev/accounts/<id>/suspension {"suspended":true}`）后，只走 relay 的会话约 30 秒内断开。

没有桌面时用命令行走同样的流程：

```sh
# Host（无桌面）：登录账号后 Host 自动登记密钥、取 ticket 并加入账号的 relay
yttt-server p2p enable
YTTT_RELAY_PASSWORD='…' yttt-server p2p login https://<控制面> me@example.com
yttt-server p2p pair

# 设备：--account 让 probe 也登录同一账号，带 ticket 加入 relay；结束时从账号移除它的密钥
YTTT_RELAY_PASSWORD='…' p2p_probe '<配对码>' --relay-only --seconds 30 \
    --account https://<控制面> me@example.com
```

不设 `YTTT_RELAY_PASSWORD` 时会在终端询问密码（输入可见）。`yttt-server p2p logout` 让 Host 退出账号。

每把设备密钥各占套餐的一个设备名额：Host 一个，每台设备对每台 Host 各一个。忘记一台配对的 Host 时会从账号移除对应的密钥。

### 直连不上时先排除环境

- relay 节点没有域名（`http://` 地址）时，设备永远“经 relay”，见 `docs/relay-operations.md`。
- 设备或 Host 开着全局代理 / TUN 模式（如 Clash 的 TUN、`198.18.x.x` 这类 fake-ip）时，UDP 走代理，QUIC 地址探测失败，也只能“经 relay”。给 relay 节点的 IP 设直连规则，或测试时关掉 TUN。
- 无桌面 Host 用 `RUST_LOG=iroh=debug yttt-server ensure` 启动后，`server.log` 里能看到 iroh 的连接与打洞过程；`yttt-server p2p status` 的 `clients` 列出当前连着的设备。

## 6. 出问题时收集什么

- Host：“远程访问此电脑”页的错误文字，或 `yttt-server p2p status` 的输出；服务器 Host 的日志在 `~/.local/state/yttt/remote/default/logs/server.log`（设置了 `XDG_STATE_HOME` 时以它代替 `~/.local/state`）。
- 设备：`logs/client.log` 的最后几十行，和连接窗口里显示的错误。
- 两台机器各自的网络环境：是否同一路由器、是否有公司防火墙或 VPN、Windows 防火墙是否弹窗。
- 配对失败时设备上显示的原因：`拒绝`、`邀请已关闭`（码过期或被第二台设备用过）、`认证失败`（码抄错）。
