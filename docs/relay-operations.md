# 运营付费 relay：控制面、控制台与节点

本文写给部署和运营 yttt relay 服务的人。relay 只转发用户设备之间端到端加密的流量；控制面和节点都拿不到任何能连接用户电脑的东西。

## 1. 控制面

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
| `database_url` | PostgreSQL |
| `ticket_secret` | 签发 relay ticket 的 Ed25519 私钥（32 字节十六进制）；`yttt-control public-key <secret>` 打印公钥 |
| `web_root` | 控制台静态文件，镜像里是 `/usr/share/yttt-control/web` |
| `relay_binaries_dir` | 发给节点的二进制，镜像里是 `/usr/share/yttt-control/bin` |
| `[signup]` | 可选，只在空库首次启动时写入：是否开放注册、新账号的 `plan` 与 `node`；之后在控制台“设置”里改 |
| `[[plans]]` | 可选，只在空库首次启动时写入；之后套餐在控制台“套餐”里新建、修改、归档 |
| `[[nodes]]` | 可选，只在空库首次启动时写入；之后节点在控制台管理 |
| `dev_mode` | 开发用：允许 http 的部署命令和 `/v1/dev/*` 接口，生产关闭 |

新部署还没有可用节点，注册是关闭的。第一个管理员用命令行创建（密码从标准输入读）：

```sh
docker compose exec -T control yttt-control create-admin --config /etc/yttt/control.toml you@example.com
```

账号落在注册设置（首次启动时取自 `[signup]`）的套餐和节点上；该节点不存在时会以“待部署”状态建出来，登录控制台后在节点页“重新生成部署命令”把它部署起来，注册随即开放。已有账号要升为管理员用 `yttt-control promote-admin --config control.toml <email>`。

## 2. 部署节点

1. 控制台 → 节点 → 新建节点：填 ID（如 `tokyo-1`）、显示名，可选域名。
2. 复制对话框里的命令，在目标 Linux 服务器（systemd，x86_64 或 aarch64）上执行。命令只显示一次，1 小时内有效、单次可用。
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
- 节点：停用后，其上所有账号的设备在下次上报时断开；停用或删除前先“迁移账号到其他节点”。账号的设备在续签 ticket 后才改用新节点。
- 节点地址尽量不变：设备要到 ticket 续签时才换到新地址，Host 也需要重新开启 P2P。推荐给节点配域名。
- 所有管理操作记在“审计日志”。

## 4. 安全边界

- 登录、注册、节点登记共用按来源的限流；同一邮箱连续失败 10 次锁定 15 分钟。
- 浏览器会话是 HttpOnly、SameSite=Strict 的 cookie，所有修改请求还要带控制台专用请求头。
- 节点只能为自己节点上的账号计费、取得它们的状态。
- 控制面分发节点程序：控制面一旦被攻破，攻击者能控制之后新部署的节点，但仍然接触不到任何用户的电脑；用户设备之间的身份与授权只在设备之间建立。
