# 运营付费 relay：控制面、控制台与节点

本文写给部署和运营 yttt relay 服务的人。relay 只转发用户设备之间端到端加密的流量；控制面和节点都拿不到任何能连接用户电脑的东西。

## 1. 控制面

镜像 `packaging/relay-dev/Dockerfile` 同时包含控制面、网页控制台和发给节点的静态 relay 二进制（x86_64 与 aarch64 各一个：构建机自身架构用 musl-gcc，另一个用 Zig 交叉编译）：

```sh
docker build -f packaging/relay-dev/Dockerfile -t yttt-relay .
```

已验证的是在 aarch64 构建机上产出两种二进制。在 x86_64 构建机上交叉编译 aarch64 时，Zig 0.13 可能不认 rustc 为 aarch64 传的链接参数 `--fix-cortex-a53-843419`；遇到时升级 `cargo-zigbuild`，或在 aarch64 机器上构建。

控制面配置（TOML）的关键项：

| 项 | 说明 |
|---|---|
| `listen` | 监听地址 |
| `public_url` | 浏览器与新节点访问控制面的地址，生产必须是 `https://`（放在反向代理后面时同时设 `trusted_proxy = true`） |
| `database_url` | PostgreSQL |
| `ticket_secret` | 签发 relay ticket 的 Ed25519 私钥（32 字节十六进制）；`yttt-control public-key <secret>` 打印公钥 |
| `web_root` | 控制台静态文件，镜像里是 `/usr/share/yttt-control/web` |
| `relay_binaries_dir` | 发给节点的二进制，镜像里是 `/usr/share/yttt-control/bin` |
| `[signup]` | `enabled`、新账号的 `plan` 与 `node` |
| `[[plans]]` | 套餐：速率、突发、每月额度、超额后速率、设备上限 |
| `[[nodes]]` | 可选，只在空库首次启动时写入；之后节点在控制台管理 |
| `dev_mode` | 开发用：允许 http 的部署命令和 `/v1/dev/*` 接口，生产关闭 |

新部署还没有可用节点，注册是关闭的。第一个管理员用命令行创建（密码从标准输入读）：

```sh
yttt-control create-admin --config control.toml you@example.com
```

账号落在 `[signup]` 的节点上；该节点不存在时会以“待部署”状态建出来，登录控制台后在节点页“重新生成部署命令”把它部署起来，注册随即开放。已有账号要升为管理员用 `yttt-control promote-admin --config control.toml <email>`。

## 2. 部署节点

1. 控制台 → 节点 → 新建节点：填 ID（如 `tokyo-1`）、显示名，可选域名。
2. 复制对话框里的命令，在目标 Linux 服务器（systemd，x86_64 或 aarch64）上执行。命令只显示一次，1 小时内有效、单次可用。
3. 放行端口：
   - 无域名：TCP 3341。节点地址为 `http://<服务器公网 IP>:3341`。
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

重新部署同一节点（换机器或令牌泄露）：控制台“重新生成部署命令”，在新机器上执行；登记成功的那一刻旧节点令牌失效。

## 3. 日常管理

- 账号：搜索、改套餐、迁移节点、封停或解封、设为或撤销管理员、移除设备。封停与撤销管理员会立即注销该账号的所有登录；节点在下一次上报（约 15 秒）时断开被封停账号的设备。
- 节点：停用后，其上所有账号的设备在下次上报时断开；停用或删除前先“迁移账号到其他节点”。账号的设备在续签 ticket 后才改用新节点。
- 节点地址尽量不变：设备要到 ticket 续签时才换到新地址，Host 也需要重新开启 P2P。推荐给节点配域名。
- 所有管理操作记在“审计日志”。

## 4. 安全边界

- 登录、注册、节点登记共用按来源的限流；同一邮箱连续失败 10 次锁定 15 分钟。
- 浏览器会话是 HttpOnly、SameSite=Strict 的 cookie，所有修改请求还要带控制台专用请求头。
- 节点只能为自己节点上的账号计费、取得它们的状态。
- 控制面分发节点程序：控制面一旦被攻破，攻击者能控制之后新部署的节点，但仍然接触不到任何用户的电脑；用户设备之间的身份与授权只在设备之间建立。
