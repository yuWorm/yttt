# 运营付费 relay：控制面、控制台与节点

本文写给部署和运营 yttt relay 服务的人。relay 只转发用户设备之间端到端加密的流量；控制面和节点都拿不到任何能连接用户电脑的东西。

## 1. 控制面

### 一键安装

每个版本都附带交互式安装脚本（Linux，root）：

```sh
curl -fsSL https://github.com/yuWorm/yttt/releases/latest/download/install-control.sh -o install-control.sh
sudo sh install-control.sh
```

它依次询问：

| 问题 | 选项 |
|---|---|
| 安装方式 | Docker（compose，位于 `/opt/yttt-control`）/ 直接安装（systemd，静态二进制） |
| 数据库 | SQLite（一个文件，适合个人与小机器）/ PostgreSQL（Docker 下为容器；直接安装时用发行版的包，经本机 socket 以服务用户身份连接，没有数据库密码） |
| 对外方式 | Caddy 占用 80/443，自动申请与续期证书（需要域名）/ 只启动服务，自己用 nginx 等反向代理（可只绑定 127.0.0.1） |
| 注册 | 是否允许自助注册（之后在控制台改） |
| 管理员 | 邮箱与密码（输入两次） |

`ticket_secret`、`secrets_key` 与 PostgreSQL 容器密码随机生成，写进只有 root（和服务用户）能读的配置文件；脚本不打印它们。再次运行时脚本识别已有安装，提供“升级 / 重新配置 / 卸载”，**重新配置不会更换任何秘密**（换了会让所有 ticket 与节点失效）。也可以直接 `--upgrade`、`--reconfigure`、`--uninstall [--purge]`；所有问题都有对应参数，配合 `--yes` 可无人值守（管理员密码取 `YTTT_ADMIN_PASSWORD`），见 `sh install-control.sh --help`。

空闲时的内存：控制面 6–8 MB，PostgreSQL 约 40 MB，Caddy 约 15 MB。一台 512 MB 的小机器用 SQLite + Caddy 绰绰有余。

选 Caddy 时，脚本还会问“是否同时把本机部署为 relay 节点”（`--relay-domain relay.example.com` / `--no-relay`）。选是则：

- Caddy 同时为 relay 域名签证书并转发到本机 `127.0.0.1:3340`；relay 在 Caddy 之后明文监听（`yttt-relay enroll --behind-proxy`）。
- QUIC 地址探测（UDP 7842）需要 relay 域名的证书：`yttt-relay-cert.timer` 每 10 分钟把 Caddy 的证书复制到 `/etc/yttt-relay/quic/`，relay 每分钟检查一次并自动换上（配置里的 `[quic_tls]`）。
- 节点用安装时的管理员身份登记为 `node-1`（新账号的默认节点），与控制面同一种安装方式；需要放行 TCP 80、443 与 UDP 7842。
- 卸载控制面不会删除这个节点，用 `sudo yttt-relay-install --uninstall` 另行删除。

### 手动部署

每次发布（`v*` 标签）都会推送镜像 `ghcr.io/yuworm/yttt-relay:<版本>` 与 `:latest`（linux/amd64、linux/arm64），内含控制面、网页控制台和发给节点的两种架构静态 relay 二进制；同样的静态二进制也附在 GitHub Release 里。生产部署用 `packaging/relay/` 下的 compose 文件与配置模板：

```sh
cd packaging/relay
cp control.example.toml control.toml        # 填写 public_url、数据库密码、ticket_secret
POSTGRES_PASSWORD=… docker compose up -d     # YTTT_VERSION=0.4.0 可固定版本
```

控制面只监听本机 8080，前面放 HTTPS 反向代理（并设 `trusted_proxy = true`）。升级：改 `YTTT_VERSION` 后 `docker compose up -d`，数据库迁移自动执行；已部署的节点用 `sudo yttt-relay-install --upgrade` 换成新镜像里的 relay。

开发时也可以从源码构建整套镜像：`docker build -f packaging/relay-dev/Dockerfile -t yttt-relay-dev .`（构建机自身架构用 musl-gcc，另一个架构用 Zig 交叉编译；在 x86_64 上交叉编译 aarch64 时 Zig 0.13 可能不认 `--fix-cortex-a53-843419`，遇到时在 aarch64 机器上构建）。

控制面配置（TOML）的关键项：

| 项 | 说明 |
|---|---|
| `listen` | 监听地址 |
| `public_url` | 浏览器与新节点访问控制面的地址，生产必须是 `https://`（放在反向代理后面时同时设 `trusted_proxy = true`） |
| `database_url` | `postgres://…` 或 `sqlite:///路径/control.db`（不存在时创建） |
| `ticket_secret` | 签发 relay ticket 的 Ed25519 私钥（32 字节十六进制）；`yttt-control public-key <secret>` 打印公钥 |
| `secrets_key` | 加密控制台保存的秘密（GitHub / Google 的 client secret），32 字节十六进制；更换后需重新填写这些秘密 |
| `web_root` | 控制台静态文件，镜像里是 `/usr/share/yttt-control/web` |
| `relay_binaries_dir` | 发给节点的二进制，镜像里是 `/usr/share/yttt-control/bin` |
| `[signup]` | 可选，只在空库首次启动时写入：是否开放注册、新账号的 `plan` 与 `node`；之后在控制台“设置”里改 |
| `[[plans]]` | 可选，只在空库首次启动时写入；之后套餐在控制台“套餐”里新建、修改、归档 |
| `[[nodes]]` | 可选，只在空库首次启动时写入；之后节点在控制台管理 |
| `dev_mode` | 开发用：允许 http 的部署命令和 `/v1/dev/*` 接口，生产关闭 |

新部署还没有可用节点，注册是关闭的。第一个管理员用命令行创建（密码从标准输入读；安装脚本会替你执行）：

```sh
docker compose exec -T control yttt-control create-admin --config /etc/yttt/control.toml you@example.com
```

账号落在注册设置（首次启动时取自 `[signup]`）的套餐和节点上；该节点不存在时会以“待部署”状态建出来，登录控制台后在节点页“重新生成部署命令”把它部署起来，注册随即开放。已有账号要升为管理员用 `yttt-control promote-admin --config control.toml <email>`。

### GitHub / Google 登录（可选）

在控制台“设置 → 第三方登录”填写 client ID 与 secret 并启用即可（secret 以 `secrets_key` 加密保存，之后不再显示；`control.toml` 里的 `[oauth.*]` 会在启动时一次性导入控制台）。启用后，控制台的登录、注册页出现“使用 GitHub / Google 登录”，账号页可以关联、解除关联，用户可以完全不设密码。需要 `public_url`，Google 还要求它是 `https://`；不满足时控制面拒绝启动并说明原因。

- GitHub：Settings → Developer settings → **OAuth Apps** → New OAuth App（不是 GitHub App）。Homepage URL 填 `public_url`，Authorization callback URL 填 `{public_url}/v1/auth/github/callback`。
- Google：Google Cloud Console → APIs & Services → Credentials → Create credentials → OAuth client ID，类型 Web application，Authorized redirect URIs 填 `{public_url}/v1/auth/google/callback`；OAuth consent screen 只需 `openid`、`email`。

```toml
[oauth.github]
client_id = "…"
client_secret = "…"

[oauth.google]
client_id = "…"
client_secret = "…"
```

改完重启控制面（`docker compose up -d`）。规则：

- 同一个 GitHub / Google 账号只认它的不变 id，不认邮箱。新邮箱直接注册（遵守注册设置），**已被注册的邮箱不会自动合并**：提示用户用密码登录后在“账号”页关联；如果该邮箱是别人抢注的，由管理员处理。
- 没有密码的账号只能在刚登录后 5 分钟内设置密码，也不能解除最后一种登录方式。

### 应用与无屏 Host 的登录

- 应用“在浏览器中登录”打开控制台的授权页，用户点“允许”后浏览器把结果交回本机应用（只回到 `127.0.0.1`）。
- `yttt-server p2p login <控制面地址>` 显示短码和二维码，在任意设备上打开 `{public_url}/device` 输入或扫码确认；`--password <邮箱>` 仍可用密码（脚本化部署）。
- 这两种方式得到的是**设备会话**：只能登记设备、取 ticket、查看账号，不能修改账号与登录方式、确认别的登录或进入管理页。即使用户被骗点了“允许”，交出去的也只有 relay 用量。
- 升级到支持这些登录的版本后，已有的登录一律按设备会话处理：应用照常可用，控制台需要重新登录一次。

## 2. 部署节点

1. 控制台 → 节点 → 新建节点：填 ID（如 `tokyo-1`）、显示名，可选域名。
2. 复制对话框里的命令，在目标 Linux 服务器（x86_64 或 aarch64）上执行。命令只显示一次，1 小时内有效、单次可用。脚本询问直接安装（systemd）还是用 Docker（host 网络，镜像与控制面同版本），以及节点域名（部署命令已带域名时不再询问）；`--method native|docker`、`--yes` 可跳过询问。
3. 放行端口：
   - 无域名：TCP 3341。节点地址为 `http://<服务器公网 IP>:3341`。**无域名的节点只能中转，设备之间不会升级为直连**：直连靠设备经 relay 的 QUIC 地址探测（UDP 7842，需要 TLS）得知自己的公网地址，无域名节点不提供这项服务。正式节点请配域名；没有域名时可以用 `<IP 以连字符分隔>.sslip.io`（如 `103-68-192-229.sslip.io`），它解析到对应 IP，Let's Encrypt 能为它签证书。
   - 有域名：域名先解析到该服务器；放行 TCP 443、80 与 UDP 7842。证书由 Let's Encrypt 自动签发，缓存在 `/var/lib/yttt-relay`。443 必须直达服务器，前面不能有 CDN 或 TLS 终结。
4. 一分钟内节点显示“在线”。若显示“地址不可达”，说明节点在上报但控制面访问不到它的地址：检查防火墙，或在“编辑名称与地址”中改正。

服务器在 NAT 后面、出口与入口地址不同时，在命令末尾加 `--public-url http://<可访问的地址>:3341`。

节点上的常用操作：

```sh
journalctl -u yttt-relay -f                         # 日志
sudo yttt-relay-install --upgrade                   # 换成控制面当前提供的二进制，不重新登记
sudo yttt-relay-install --uninstall                 # 删除节点程序、配置与服务用户
```

安装脚本会创建系统用户 `yttt-relay` 运行服务；节点令牌在 `/etc/yttt-relay/node-token`（`root:yttt-relay`，0640）。

改了安装脚本后，可以在一台一次性的 systemd 容器里把节点的整个生命周期（部署、重启后恢复、升级、卸载）演练一遍：

```sh
packaging/relay-dev/rehearse-node.sh '<控制台给出的部署命令>' linux/arm64
```

控制面的 `public_url` 需要能从容器访问（如 `http://host.docker.internal:<端口>`）。`linux/amd64` 演练需要 Docker 开启 x86 模拟。

给已部署的节点加域名：控制台“重新生成部署命令”时填域名，在同一台机器上执行即可（脚本会重启服务）；之后在 Host 上重新登录 relay 账号或重开 P2P，让它换到新地址。

重新部署同一节点（换机器或令牌泄露）：控制台“重新生成部署命令”，在新机器上执行；登记成功的那一刻旧节点令牌失效。

## 3. 日常管理

- 账号：搜索与筛选、开户、重置密码（用户没有找回密码的途径，由管理员设置新密码后告知）、改套餐、迁移节点、封停或解封、设为或撤销管理员、移除设备。重置密码会注销该账号所有登录。
- 套餐：新建、修改、归档。速率与额度在节点下一次上报（约 15 秒）时生效；调低设备上限只限制新登记。归档的套餐保留现有账号但不再分配；有账号的套餐不能删除；注册用的套餐要先在“设置”里换掉才能归档。
- 设置：是否开放注册、新账号的套餐与节点。封停与撤销管理员会立即注销该账号的所有登录；节点在下一次上报（约 15 秒）时断开被封停账号的设备。
- 节点归属：每台 **Host** 自己选节点（应用“远程访问此电脑 → relay 账号 → 节点”，或 `yttt-server p2p nodes|node <id>`，或控制台“设备”页），连接它的设备**总是跟随 Host 所在的节点**，Host 与它的远程设备不会分在两个节点上。账号的节点是新 Host 的默认节点。
- 套餐与节点：套餐设“可用节点”和“同时使用的节点数”。每个节点各自限速，账号的 Host 分布在 N 个节点上最多得到 N 倍速率；为 1 时切换节点会让整个账号一起迁移。从套餐移出某节点、停用节点或降级后，其上的 Host 在下次取 ticket 时回到默认节点。月流量额度跨节点合计。
- 切换的传播：旧节点在下次上报（约 15 秒）时收到撤销列表，断开离开的设备并拒绝它们的旧 ticket（节点重启后列表会重放）；Host 每 5 分钟核对一次自己的节点，设备在重连时改连新节点。应用或命令行切换同一台 Host 之间有 10 分钟冷却，控制台不受限。
- 节点容量：节点可设在线设备上限与地区。满载的节点不再接收新的 Host 和新账号；“负载”里有近 24 小时与 30 天的在线设备和流量。
- 节点：停用后，其上的设备在下次上报时断开；停用或删除前先“迁移到其他节点”（Host、跟随它们的设备与默认节点在此的账号一起迁移，目标节点自动向这些账号的套餐开放）。
- 节点地址尽量不变：设备要到 ticket 续签时才换到新地址，Host 也需要重新开启 P2P。推荐给节点配域名。
- 所有管理操作记在“审计日志”。

## 4. 安全边界

- 登录、注册、节点登记、发起第三方登录、发起与确认短码共用按来源的限流；同一邮箱连续失败 10 次锁定 15 分钟。限流计数在进程内存里，多个控制面实例各算各的；登录流程本身的状态都在数据库，重启不丢。
- 浏览器会话是 HttpOnly、SameSite=Strict 的 cookie，所有修改请求还要带控制台专用请求头。cookie 里只认控制台会话；应用与命令行的设备会话放进 cookie 也不算登录。
- 应用的浏览器登录用 PKCE，授权码只发往 `127.0.0.1` / `[::1]`，60 秒内有效、只能用一次；短码 10 分钟有效。
- 节点只能为自己节点上的账号计费、取得它们的状态。
- 控制面分发节点程序：控制面一旦被攻破，攻击者能控制之后新部署的节点，但仍然接触不到任何用户的电脑；用户设备之间的身份与授权只在设备之间建立。
