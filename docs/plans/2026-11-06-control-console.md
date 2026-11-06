# 控制面前端（用户端 + 管理端）设计与实施计划

- 日期：2026-11-06
- 前置：[p2p-backend-plan](2026-11-06-p2p-backend-plan.md) Part B（`yttt-control` 已有账号、设备、ticket、用量接口）
- 工作树：`.worktrees/p2p-host`，分支 `feat/p2p-host`
- 已定：开放注册并给默认免费套餐；管理员是带 `admin` 角色的普通账号；前端由 `yttt-control` 同源托管；节点在管理端一键部署、自动上线，域名可选，带域名时自动申请证书（2026-11-06 追加，见 §8）。

## 1. 范围

做：自助注册与登录、用户自己的套餐/用量/设备管理、修改密码；管理员的总览、账号检索与处置（改套餐、改节点、封停、设管理员、移除设备）、套餐与节点只读查看、审计日志。

不做：支付、找回密码邮件、设备登记（设备必须用自己的私钥签名登记，只能由 yttt 应用完成，网页只能查看和移除）、在网页上编辑套餐（仍以配置文件为准，启动时同步进库）。节点改为在管理端创建与部署（§8），配置文件里的节点只作为首次启动的种子。

安全边界不变：控制台和控制面一样只管传输服务，不能下发 Host 身份或改 Host 的设备 ACL（设计 I1、I2）。

## 2. 技术选型

- `web/control-console/`：Vite + React 18 + TypeScript + `@douyinfe/semi-ui`（含 `@douyinfe/semi-icons`）+ `react-router-dom`。界面中文，使用 Semi 的 `zh_CN` 语言包。
- 单个 SPA：`/` 为用户端，`/admin` 为管理端，路由按 `me.role` 守卫；后端对每个管理接口再次校验角色，前端守卫只为体验。
- 构建产物 `web/control-console/dist/`。`yttt-control` 配置 `web_root` 指向该目录时提供静态文件，非 `/v1/*` 的 GET 回退到 `index.html`；未配置则只提供 API。docker 镜像多一个 node 构建阶段，把产物放到 `/usr/share/yttt-control/web`。

## 3. 浏览器会话与 CSRF

- 浏览器不持有 bearer token。`POST /v1/web/session` 登录成功后设置 cookie `yttt_session`：`HttpOnly`、`SameSite=Strict`、`Path=/`，请求经 HTTPS 时加 `Secure`。token 与设备用的会话同表（存 SHA-256）。
- 会话提取器同时接受 `Authorization: Bearer`（设备与应用）和 cookie（浏览器）。凭 cookie 的非 GET 请求必须带 `X-Yttt-Web: 1`：跨站页面加不了这个头而不触发 CORS 预检，控制面又不开 CORS，因此跨站伪造请求会失败（配合 `SameSite=Strict` 双重防护）。
- 静态页面响应头：`Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'`，以及 `X-Content-Type-Options: nosniff`、`Referrer-Policy: no-referrer`。
- 改密码后注销该账号的其他所有会话；登出删除当前会话。

## 4. 防暴力尝试

登录与注册按来源 IP 限流（内存令牌桶：每 IP 每分钟 20 次，突发 10 次；超过返回 429；同一邮箱连续失败 10 次锁定 15 分钟）。来源 IP 取 TCP 对端地址；配置 `trusted_proxy = true` 时取 `X-Forwarded-For` 最后一跳。argon2 校验本身已在阻塞线程池执行。

## 5. 后端变更（`yttt-control`）

迁移 `0002_console.sql`：

- `accounts.role TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('user','admin'))`
- `nodes.last_report_unix BIGINT`、`nodes.last_connected_devices INTEGER`（节点每次回报时更新，供总览显示节点是否在线）
- `admin_actions (id BIGSERIAL, admin_id BIGINT, action TEXT, target_account BIGINT NULL, detail TEXT, at_unix BIGINT)`

配置追加：

```toml
web_root = "/usr/share/yttt-control/web"   # 可选
trusted_proxy = false
[signup]
enabled = true
plan = "free"
node = "local-1"
```

接口（新增；原有接口不变）：

| 方法与路径 | 身份 | 说明 |
|---|---|---|
| `POST /v1/accounts` | 无（限流） | 注册；`signup.enabled` 为 false 时 404；之后与登录一样设 cookie |
| `POST /v1/web/session` | 无（限流） | 浏览器登录，设 cookie |
| `DELETE /v1/web/session` | 会话 | 登出 |
| `GET /v1/accounts/me` | 会话 | 追加 `role` |
| `GET /v1/accounts/me/usage` | 会话 | 最近 12 个月用量 |
| `POST /v1/accounts/me/password` | 会话 | 校验旧密码，换新密码，注销其他会话 |
| `GET /v1/admin/overview` | admin | 账号数、封停数、本月总用量、节点状态 |
| `GET /v1/admin/accounts?q=&offset=&limit=` | admin | 按邮箱搜索，分页 |
| `GET /v1/admin/accounts/{id}` | admin | 账号、设备、12 个月用量 |
| `PATCH /v1/admin/accounts/{id}` | admin | `plan`/`node`/`suspended`/`role` 任选；记审计；不能撤销自己的 admin |
| `DELETE /v1/admin/accounts/{id}/devices/{endpoint_id}` | admin | 移除设备；记审计 |
| `GET /v1/admin/plans` | admin | 只读 |
| 节点管理 | admin | 见 §8 |
| `GET /v1/admin/audit?limit=` | admin | 最近的管理操作 |

命令行：`yttt-control promote-admin --config <path> <email>` 把已有账号设为管理员（第一个管理员只能这样产生）。

封停与改套餐沿用现有机制：节点下一次回报（≤15 秒）就拿到新的状态；移除设备同样经 `revoked_endpoints` 生效。

## 6. 前端页面

用户端：登录 / 注册；概览（套餐、所在节点与 relay 地址、本月用量进度、设备数/上限、封停提示）；设备（列表、移除确认，说明“新设备在 yttt 应用里登录后自动登记”）；用量（12 个月柱状图，用 Semi 卡片 + 简单 SVG 条形，不引入图表库）；账号（修改密码、登出）。

管理端：总览（统计卡片、节点表：最后回报时间、在线设备数）；账号（搜索、分页表格、状态标签，行操作：封停/解封、改套餐、改节点、设/撤管理员，均二次确认）；账号详情（设备表可移除、用量）；节点（§8：列表与状态、“新建节点”对话框给出可一键复制的部署命令、重新生成命令、停用）；套餐（只读表）；审计日志。

所有字节数显示为 KB/MB/GB；时间显示本地时间。

## 8. 节点一键部署与自动上线

### 8.1 流程

1. 管理员在“节点”页点“新建节点”，填节点 ID（如 `tokyo-1`）、显示名、可选域名。控制面建一条 `pending` 节点和一次性登记令牌（32 字节随机，库里只存 SHA-256，1 小时过期，单次有效），返回部署命令：

   ```sh
   curl -fsSL https://控制面/install/relay.sh | sudo sh -s -- --enroll <令牌> [--domain relay.example.com]
   ```

   命令只在对话框里显示一次，带“复制”按钮；关闭后只能“重新生成”（旧令牌立即作废）。
2. 管理员在目标服务器上粘贴执行。脚本：
   - 检测架构（x86_64 / aarch64，仅 Linux + systemd）；
   - 从控制面下载 `yttt-relay`（`/install/bin/yttt-relay-linux-<arch>`），按 `/install/bin/SHA256SUMS` 校验；`--binary-url` 可改用别处的二进制；
   - 安装到 `/usr/local/bin/yttt-relay`，建 `/etc/yttt-relay/`（0700）；
   - 运行 `yttt-relay enroll --control <URL> --token <令牌> [--domain D]`：向控制面登记，写出 `/etc/yttt-relay/relay.toml` 与 `/etc/yttt-relay/node.secret`（0600）；
   - 安装并启动 systemd 服务（复用 `packaging/relay-selfhost/yttt-relay.service` 的形态），等健康检查通过后打印节点地址与需要放行的端口。脚本不改防火墙，只提示。
   - 重复执行（同一令牌已用）会失败并提示重新生成命令；对已登记的机器重新部署用新令牌，登记会轮换节点令牌。
3. 节点启动后每 15 秒回报用量；控制面记录 `last_report_unix`。控制台按最后回报时间显示：在线（45 秒内）、离线、待登记。

### 8.2 登记接口

`POST /v1/nodes/enroll { token, domain?, port? }`（无会话，按来源 IP 限流）：

- 校验令牌哈希、未过期、未使用；在同一事务里标记已用。
- 节点地址：带域名为 `https://<domain>`；否则 `http://<来源 IP>:<port，默认 3341>`。来源 IP 取 TCP 对端（`trusted_proxy` 时取 `X-Forwarded-For` 最后一跳），控制面不调用外部服务探测公网 IP。管理员之后可在控制台改地址。
- 生成新的节点令牌（随机 32 字节，库里存 SHA-256），返回 `{ node_id, node_token, ticket_keys, report_interval_secs, url }`。

### 8.3 relay 侧

- `yttt-relay enroll` 子命令，产出完整配置；`relay.toml` 里节点令牌改为引用文件 `node_token_file`，不写明文。
- 自动证书：`[acme] domains = [...]、contact、cache_dir`，用 `tokio-rustls-acme`（与 iroh-relay 同版本 0.9）在 443 端口做 TLS-ALPN-01；同时开 80 端口的探测服务与 UDP 7842 的 QUIC 地址探测。无域名时只开 `http://0.0.0.0:3341`。
- ticket 公钥轮换：用量回复追加 `ticket_keys`，节点运行时替换验签公钥，不必重启或重新登记。

### 8.4 数据与权限

- 迁移：`nodes` 追加 `name`、`state`（`pending`/`active`/`disabled`）、`created_unix`、`enrolled_unix`、`last_report_unix`、`last_connected_devices`、`version`；新表 `node_enrollments(token_hash PK, node_id, expires_unix, used_unix)`。
- 管理接口：`GET /v1/admin/nodes`、`POST /v1/admin/nodes`、`POST /v1/admin/nodes/{id}/enrollment`（重新生成命令）、`PATCH /v1/admin/nodes/{id}`（名称、地址、停用）、`DELETE /v1/admin/nodes/{id}`（仅当没有账号在该节点）。全部记审计。
- 停用节点：拒收该节点的回报（401），不再给它签 ticket，也不再把新注册账号分到它；已在上面的账号需管理员用“改节点”迁走。被停用的节点按缓存继续服务已连上的设备，直到它们的 ticket 过期。
- 注册默认节点：`signup.node` 若设置则用之，否则选在线节点中账号最少的一个。
- 安全：登记令牌是能让一台机器成为计费节点的凭据，但节点仍拿不到任何能接触 Host 的东西（设计 I5）；节点令牌只能上报用量。最坏情况（令牌泄露被抢先登记）是假节点上报虚假用量或拒绝服务，管理员停用该节点并重新生成即可。

## 9. 实施任务

- C1 后端：迁移、配置、cookie 会话与 CSRF 头、限流、注册、改密、用量历史、管理接口、审计、`promote-admin`、静态托管与安全头。测试（PostgreSQL，`#[ignore]`）：注册开关与默认套餐；cookie 会话缺 `X-Yttt-Web` 被拒；限流 429；普通用户访问管理接口 403；管理员改套餐/封停后节点回报得到新状态；不能撤销自己的 admin；改密注销其他会话；审计记录；SPA 回退与安全头。
- C2 前端工程：`web/control-console` 脚手架、API 客户端（统一带 `X-Yttt-Web`、401 跳转登录）、布局与路由守卫。
- C3 用户端页面。
- C4 管理端页面。
- C5 打包：Dockerfile 加 node 构建阶段，compose 的 `control.toml` 加 `web_root` 与 `[signup]`；`.gitignore` 追加 `web/control-console/node_modules`、`web/control-console/dist`（只追加）。
- C7 节点部署：迁移、登记接口、管理接口、安装脚本与二进制下发、`yttt-relay enroll`、ACME、公钥随回复下发；测试：令牌单次有效/过期/作废、登记后节点能上报、停用后回报被拒、脚本 `sh -n` 语法检查与在 docker 容器里的安装演练。
- C6 验证：`npm run build`（含 `tsc --noEmit`），起 docker 全栈，用浏览器走一遍：注册 → 概览 → 提升管理员 → 管理端封停/改套餐 → 用户端看到状态；截图核对布局。

## 10. 评审后修订（2026-11-06，与前文冲突处以本节为准）

已修复：节点上报只计费、只回答本节点上的账号；其它账号一律 `Suspended`，不透露设备登记情况（`576d7df`）。

### 10.1 控制台

- 登录、注册、登记、设备用的 `/v1/sessions` 共用一个按来源限流器（IPv6 按 /64 聚合，表项过期淘汰），另加按邮箱的连续失败计数。注册对已占用邮箱返回 409，可被用来探测邮箱是否注册，接受此代价。
- 登录与注册接口同样要求 `X-Yttt-Web: 1`，防登录 CSRF。
- 配置新增 `public_url`（控制面对外地址）。cookie 是否加 `Secure` 由它的协议决定；cookie 名为 `__Host-yttt_session`（https）或 `yttt_session`（http 开发）。
- 会话提取器拒绝已封停账号；管理员鉴权每次从库里读 `role`。封停或撤销管理员时注销该账号全部会话。
- SPA 回退排除 `/v1/*`、`/install/*`、`/healthz`。CSP 追加 `font-src 'self' data:`。

### 10.2 节点

- 迁移合并为一份 `0002_console.sql`。`nodes.token_hash` 改为可空，鉴权时拒绝空值。配置里的 `nodes` 改为可选，只作首次种子：`ON CONFLICT DO NOTHING`，不再覆盖库里轮换过的令牌和改过的地址。
- 停用节点：回报照常接收（200），但对该节点上的所有账号回答 `Suspended`，已连接的设备随即被断开；另提供“把该节点账号整体迁到另一节点”。
- 登记令牌：
  - 用一条原子语句消费：`UPDATE … WHERE token_hash=$1 AND used_unix IS NULL AND expires_unix>now RETURNING node_id`。
  - “重新生成”在同一事务里删掉该节点所有未使用的令牌；正在运行的节点令牌只在新令牌真正登记成功时才轮换。
  - 登记响应丢失时令牌已经用掉，只能重新生成。
- 令牌不出现在进程参数里：部署命令为 `curl -fsSL <public_url>/install/relay.sh -o yttt-relay-install.sh && echo '<sha256>  yttt-relay-install.sh' | sha256sum -c && sudo YTTT_ENROLL=<令牌> sh yttt-relay-install.sh [--domain D]`。脚本经环境变量把令牌传给 `yttt-relay enroll`（从 stdin 读）。
- 脚本完整性：
  - 命令里内嵌脚本自身的 sha256，脚本主体包在函数里、末尾才调用，避免执行到一半；
  - `SHA256SUMS` 与二进制同源，只能发现下载截断，不比 HTTPS 多提供完整性，这一点如实写进脚本注释；
  - `public_url` 不是 https 且未开 `dev_mode` 时，拒绝生成部署命令。
- I6 边界：由控制面分发 relay 二进制，意味着控制面一旦被攻破，攻击者能拿到之后新部署节点的 root。它仍然碰不到任何 Host，也碰不到 Host 的更新通道。这一点作为已接受的风险写进设计文档；以后改为用离线密钥签名二进制。
- 节点地址：脚本可用 `--public-url` 显式给出。否则带域名时为 `https://<domain>`，不带域名时为 `http://<来源 IP>:3341`（IPv6 写成 `[addr]`）。控制面登记后主动回探节点的 `/healthz`，失败时在控制台标黄。地址视为基本不可变：改了以后，设备要到 ticket 续签才会换到新地址，所以推荐使用域名。
- systemd：
  - 节点令牌经 `LoadCredential=node-token:/etc/yttt-relay/node-token`，从 `$CREDENTIALS_DIRECTORY` 读取，配置项为 `node_token_file`；
  - ACME 证书缓存放 `StateDirectory=yttt-relay`，保证持久化，避免撞上 Let's Encrypt 的重复签发限额；
  - 继续用 `DynamicUser=yes`。
- ACME：
  - 接入方式照搬 iroh-relay：接受连接时区分 ACME 验证连接与普通 TLS 连接，另起一个任务驱动证书状态；
  - QUIC 共用基于 resolver 的 TLS 配置；
  - 443 必须公网直达，前面不能有 CDN 或 TLS 终结；
  - 脚本先检查域名是否解析到本机；健康检查容忍证书还没签下来；配置支持 Let's Encrypt staging。
- 版本与时钟：用量上报追加 `version`；登记响应带服务器时间，偏差超过 60 秒时脚本警告。
- 注册时没有可用节点：返回 503，不把账号分到待登记的节点。待登记节点超过令牌有效期后，控制台显示“令牌已过期”。
- 审计：生成令牌、停用节点、迁移账号都记审计。
- 二进制：v1 只产出 `x86_64-unknown-linux-musl` 静态构建（docker 构建阶段用 cargo-zigbuild），放在控制面镜像的 `/usr/share/yttt-control/bin/`；aarch64 推后。
- 脚本支持 `--no-systemd`（前台运行，便于在 docker 里演练）、`--upgrade`（只换二进制、不重新登记）、`--uninstall`。

### 10.3 v1 不做

ticket 公钥热替换（登记时下发一次；现在也没有签名密钥轮换能力）、aarch64 节点、注册时“自动选账号最少的节点”（`signup.node` 必须配置）。

### 10.4 任务顺序

C1（控制台后端）→ C7a（节点入库、种子改造、`node_token_file`、停用语义）→ C7b（登记接口、`yttt-relay enroll`、安装脚本、musl 二进制、docker 演练）→ C7c（ACME）→ C2–C4（前端）→ C5（打包）→ C6（全流程验证）。
