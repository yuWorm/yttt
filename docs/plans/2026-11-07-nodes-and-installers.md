# 按 Host 选节点、节点统计与一键安装

- 日期：2026-11-07
- 前置：[oauth-login](2026-11-07-oauth-login.md)（设备会话、浏览器登录）、[console-v2](2026-11-06-console-v2.md)、[relay-account-in-app](2026-11-06-relay-account-in-app.md)
- 已定：节点按设备选择，但 **Host 与连接它的远程 Client 永远在同一节点**；多节点限速按 a+b（每节点各自限速 + 套餐限制同时使用的节点数）；直接对外时由 Caddy 管证书；PostgreSQL 与 SQLite 双后端；节点在 app 内选择。
- 评审：已按设计评审修订（计费与 standing 按端点归属、Host 自己发起切换并轮询、持久撤销表、`max_nodes` 计数与加锁、签发时复核、role 不可变、设备会话只能切自己、双后端改写规则、同机 TLS、安装脚本幂等与版本固定、独立的加密密钥）。

## 1. 节点模型：Host 选节点，Client 跟随 Host

### 1.1 现状与约束

- ticket 绑定一把设备密钥和一个节点（`Ticket { account_id, endpoint_id, node_id, … }`，节点核对 `node_id`）。relay 只在连到同一节点的端点之间转发。
- Client 为每台配对的 Host 生成一把独立的设备密钥、一个独立的 endpoint；一个 endpoint 只用一个 relay。
- 今天节点属于账号：所有设备拿账号节点的 ticket，Client 把自己 ticket 的 relay 当作 Host 的 relay（`src/remote_host.rs:503`）。
- 节点上报的计费与 standing 现在按“账号所在节点”过滤（`api.rs` `node_usage` 中 `a.node_id = $node`）：账号的设备出现在别的节点上会被判为 Suspended、不计费。本设计必须改掉这一点（1.6）。

### 1.2 新模型

- **只有 Host 选节点。** 每个 Host 端点（本机作为 Host 时的 P2P 密钥）有自己的节点；账号的 `node_id` 退为“默认节点”：新 Host 落在它上面。
- **Client 跟随 Host，永不跨节点。** Client 申请 ticket 时说明要连哪台 Host；控制面核对该 Host 属于同一账号，按 **Host 的节点**签发 ticket 并返回该节点的 relay 地址。因为 Client 每台 Host 一个 endpoint，同时连在不同节点上的两台 Host 互不影响。
- 控制面不签发“Client 与它的 Host 不在同一节点”的组合：Client 没有自己的节点选择。
- 只有同一账号的 Client 能跟随；其他账号或未登录的 Client 用不了付费节点，与现状一致。

### 1.3 数据

- `devices` 增加：
  - `role`：`host` / `client` / `unknown`（旧版本登记的设备）。**一旦为 host 或 client 就不可改**，只允许 `unknown` 升级为二者之一，防止改过的客户端在角色间切换绕过限制。
  - `node_id`：该端点所在节点。Host 自选；Client 等于 `follows_host` 的节点；`unknown` 用账号默认节点。
  - `follows_host`：Client 跟随的 Host，必须指向同账号 `role = host` 的设备（不能指向 Client，没有跟随链）。Host 被移除时，跟随它的 Client 一并移除（`ON DELETE CASCADE`），其 ticket 进入撤销表。
  - `node_changed_unix`：切换冷却。
- `accounts.node_id` 语义改为默认节点（列名不变，控制台文案改）。
- `plans` 增加 `max_nodes`（同时使用的节点数上限，默认 1）。
- `plan_nodes (plan_id, node_id)`：套餐可用的节点。迁移时为每个套餐写入全部现有节点。
- `nodes` 增加 `region`（显示用）、`max_connections`（可空 = 不限）。
- `node_revocations (node_id, endpoint_id, revoked_unix)`：某节点必须拒绝的端点（1.5），保留 31 天（大于 ticket 有效期）。

### 1.4 接口

- `GET /v1/nodes`（任意会话）：本账号套餐可用、状态 active 的节点：`id、name、region、relay_url、load_percent、full`。
- `POST /v1/devices` 登记增加 `role` 与可选 `follows_host`（hex）。**登记、角色与 Host 校验在同一个事务里**：`follows_host` 不是本账号的 Host 时整体失败、不占 `max_devices`；Client 收到 404 时回退到配对码里的 relay（不带 token），与未登录时相同。
- `POST /v1/tickets` 增加可选 `host`：
  - 本端点是 Host：签它自己节点的 ticket。
  - Client：签 `follows_host` 所在节点的 ticket（请求里的 `host` 必须与登记的一致）。
  - `unknown`（旧客户端）：账号默认节点。
  - **签发时复核**：节点不在套餐可用列表、已停用、或账号超出 `max_nodes` 时，Host 被改回默认节点（走 1.5 的传播），本次按默认节点签发。套餐降级、收紧 `plan_nodes`、停用节点因此在下一次签发时生效。
- 切换节点（只改 Host）：
  - **由 Host 进程自己发起。** app 通过 Host 的管理通道发 `RemoteAccessRequest::SetRelayNode { node_id }`，无屏 Host 用 `yttt-server p2p node <id>`；Host 进程用自己的 Host 私钥签 `ProofPurpose::SwitchNode` 证明，调用 `PUT /v1/devices/{host}/node`。设备会话只能切换它能证明持有私钥的那台 Host，泄露的设备会话动不了账号里其他 Host。
  - 控制台（web 会话）可以切换任意 Host，不受冷却限制；切换在 Host 下一次轮询（1.5）时生效。
  - 校验：节点在套餐可用列表且 active 且未满；设备会话受冷却（10 分钟）限制；切换后账号使用的节点数 ≤ `max_nodes`。整个检查与写入对 `accounts` 行加锁（PostgreSQL `FOR UPDATE`，SQLite 写事务），并发切换不会越过上限。
  - **`max_nodes = 1` 时切换即整体迁移**：账号默认节点、全部 Host 及跟随它们的 Client 一起原子地改到新节点。否则单独切一台 Host 必然超限。
- 节点数的计数口径：`role = host` 的设备所在节点 ∪ 账号默认节点（`unknown` 设备落在默认节点，不额外计数）。新 Host 登记时落在默认节点，因此不会让计数增加。

### 1.5 切换的传播

1. 控制面在事务里更新 Host 的 `node_id`、所有 `follows_host` 为它的 Client 的 `node_id`，并为旧节点写入 `node_revocations`（这些端点 + 现在时间）。
2. **持久撤销**：节点上报的回复（`UsageReply`）增加顶层字段 `revocations`，下发该节点尚未确认的撤销；节点在下一次上报里带回已应用的最大时间作为确认。节点启动后第一次上报带 `since = 0`，控制面重放最近 31 天的撤销，节点重启不会让旧 ticket 复活。离线设备的旧 ticket 也因此在旧节点失效。
3. 节点据撤销断开这些端点，并拒绝它们在撤销时间之前签发的 ticket。比较时用控制面写入的时间与 ticket 的 `issued_unix`（都来自控制面时钟），不依赖节点本地时钟。
4. **Host 发现自己被切换**：iroh 的 relay 连接被拒后会静默重试，应用层收不到事件，所以不靠“断开后重取”。Host 的 ticket 维护任务改为每 5 分钟申请一次 ticket（便宜的签名调用），`node_id` 或 `relay_url` 变了就 `set_relay`；Host 自己发起的切换则立即生效。
5. Client 在每次连接时都申请 ticket（`connect_paired` 已如此），重连时自然跟到新节点；配对记录里的 relay 提示随之更新。
6. 管理员“迁移节点”：把某节点上的 Host（及其 Client）与以该节点为默认节点的账号迁到另一节点，跳过套餐检查；目标节点不在账号套餐可用列表时，控制台先要求一并调整套餐可用节点，避免下一次签发复核又把它们改回去。

### 1.6 计费与限速（a + b）

- **计费与 standing 按端点归属，不再看账号所在节点**：节点上报的每个端点只要在 `devices` 中属于该账号，其流量就计入该账号（`usage_months`、`usage_days` 已按账号与节点记），standing 按账号计算。上报的端点中 `devices.node_id != 本节点` 的，进入撤销（1.5）。
- 每个节点按账号各自限速；一个账号的 Host 分布在 N 个节点上时最多得到 N 倍速率，由 `max_nodes` 限制 N。
- 月流量额度跨节点合计；超额后每个节点都降到 `throttled_rate_bps`。
- `max_devices` 不变：Host 与 Client 端点都计数。

### 1.7 兼容

- 旧客户端不带 `role`/`host`，按账号默认节点工作；账号的 Host 都在默认节点时行为与今天相同（`max_nodes = 1` 时总是如此）。控制台提示“旧版本客户端只能连默认节点上的 Host”。
- 旧 Host 登记为 `unknown`，升级后再次登记时升级为 `host`。

## 2. 节点统计与容量

- 节点上报里已有每个账号连着的端点列表。控制面据此记录：
  - `nodes.last_connected_devices`（已有）；
  - `node_samples (node_id, bucket_unix, connections_max, upstream_bytes)`：5 分钟分桶保留 48 小时，另按天汇总保留 90 天；由控制面后台任务清理与汇总；
  - 每个账号当前在线的端点（最近一次上报时间在 2 个上报周期内）。
- 控制台：
  - 管理员“节点”页：当前连接数 / 上限、近 24 小时曲线、近 30 天每日峰值与流量；可设 `region`、`max_connections`、该节点向哪些套餐开放。
  - 用户“设备”页：每台设备是否在线、所在节点（Client 显示“跟随 <Host>”），Host 可在此切换节点（web 会话）。
- 容量：满载的节点不能被新选择，也不作为新账号的默认节点；已在其上的不受影响。

## 3. app 内选节点

- “远程访问此电脑”页的 relay 账号一节新增“节点”，仅当本机 Host 使用账号时显示：列出 `GET /v1/nodes`，app 实测延迟（先预热一次连接，再在复用的连接上对 `/healthz` 取 3 次中位数，去掉 DNS 与 TLS 握手；没有域名的节点为 http 也照测），显示名称、地区、延迟、负载；满载的置灰。
- 浏览器登录成功后、本机 Host 开始使用账号时，弹出这个选择（默认当前节点，可跳过）；之后随时可在同一处切换。选择经 Host 进程发起（`SetRelayNode`）。
- 只作为 Client 的机器不显示节点选择。
- 无屏 Host：`yttt-server p2p nodes` 列出节点（含延迟），`yttt-server p2p node <id>` 切换。

## 4. OAuth 设置进控制台

- 新表 `oauth_providers (provider, client_id, client_secret_sealed, nonce, enabled, updated_unix)`。
- 加密：控制面配置新增自动生成的 `secrets_key`（32 字节 hex，安装脚本生成）；用 HKDF-SHA256 派生加密密钥，AES-256-GCM（依赖树里已有；秘密只有几条，随机 96 位 nonce 足够），随机 nonce 与密文一起存，provider 名作为附加数据。与 `ticket_secret` 无关，轮换 ticket 密钥不影响第三方登录。备份数据库时不包含配置文件，备份泄露拿不到明文。
- 管理员“设置”页新增“第三方登录”：显示回调地址（可复制）、client id、secret（只写不回显）、启用开关；保存时校验 `public_url` 条件（GitHub 需要 `public_url`，Google 需要 https）。
- `control.toml` 的 `[oauth.*]` 在启动时若数据库中还没有该提供方则导入一次；两边不一致时启动日志警告，以数据库为准。

## 5. PostgreSQL 与 SQLite 双后端

- `database_url` 决定后端：`postgres://…` 或 `sqlite:///var/lib/yttt-control/control.db`。
- 方案：先用 `sqlx::Any` 做一次可行性验证（类型映射：`BYTEA`/`BLOB`、整型、`bool`），可行就用它让查询只写一份；不可行则用按后端泛型的查询函数（`sqlx::Database` 泛型 + 少量按方言分支），而不是每条查询写两份。
- SQL 改写规则：
  - 类型转换统一为 `CAST(x AS BIGINT)`、`CAST($2 AS TEXT)`，两边都支持；不能直接去掉（PostgreSQL 的 `SUM(bigint)` 返回 NUMERIC，`$2 IS NULL` 无法推断参数类型）。
  - `= ANY($1)` 改为 `IN (…)` 分批绑定或 `json_each($1)` / `jsonb_array_elements_text`。
  - `GREATEST(a, b)` 在 SQLite 写成 `MAX(a, b)`（按方言分支）。
  - `FOR UPDATE`：PostgreSQL 保留；SQLite 所有写事务以 `BEGIN IMMEDIATE` 开始（默认 DEFERRED 事务读后升级写会直接 SQLITE_BUSY，`busy_timeout` 无效）。
  - 占位符统一 `$N`，可以重复使用（sqlx-sqlite 按编号绑定）。
- 迁移分两套：`migrations/postgres/`（现有 0001–0004 逐字节不变地移入，已有部署的迁移校验和不变）与 `migrations/sqlite/`（等价结构，之后同步演进）。
- SQLite 打开时启用 WAL、`foreign_keys = ON`、`busy_timeout = 5000`。
- 测试：`tests/control.rs` 同一套用例跑两种后端；SQLite 默认就跑，PostgreSQL 由 `YTTT_TEST_DATABASE_URL` 开启。
- 不做两种数据库之间的数据迁移（后续可加 `export` / `import`）。

## 6. 控制面一键安装

`install-control.sh` 作为 Release 资产发布，**发布时把版本号写进脚本**：从 `releases/download/<tag>/` 下载同版本资产并按 `SHA256SUMS` 校验；Docker 方式在 compose 里固定 `YTTT_VERSION`。脚本整体包在 `main()` 里，下载不完整不会执行半截。

交互从 `/dev/tty` 读取；打不开 `/dev/tty` 时立即失败并提示使用参数与 `--yes`；隐藏输入时用 trap 恢复终端回显。所有问题都有对应参数。

**先检测已有安装**：发现已有配置就进入升级 / 重新配置菜单，**绝不重新生成 `ticket_secret`、`secrets_key` 与数据库密码**（重新生成会让所有节点与 ticket 失效）。

问题依次为：

1. **安装方式**：Docker（compose）/ 直接安装（Linux + systemd，下载静态二进制与控制台静态文件）。
2. **数据库**：SQLite（推荐个人使用）/ PostgreSQL。
   - Docker：官方 `postgres` 容器，密码随机生成。
   - 直接安装：发行版的 `postgresql` 包（RHEL 系先 `initdb`）；用 Unix socket + peer 认证，数据库用户与服务用户同名，不需要数据库密码。
3. **对外方式**：
   - **直接对外**：域名（检查是否解析到本机，不一致只警告——NAT 与云主机常见）、证书通知邮箱。安装 Caddy（Docker 下为容器，直接安装下为官方静态二进制 + systemd），占用 80/443，自动申请与续期证书；控制面只监听 `127.0.0.1:8080`，`trusted_proxy = true`，`public_url = https://域名`。
   - **只启动服务**（自带 nginx 等）：监听地址（只绑 `127.0.0.1` / `0.0.0.0`）与端口、对外地址（`public_url`）、是否在反向代理之后（`trusted_proxy`）。输出 nginx 配置示例。
4. **管理员**：邮箱、密码（隐藏输入两次），末尾执行 `create-admin`。
5. **注册**：是否开放注册（之后可在控制台改）。

文件与权限：

- 直接安装：服务用户 `yttt-control`，配置 `/etc/yttt-control/control.toml`（0600，属主服务用户），数据 `/var/lib/yttt-control`，systemd 单元带加固选项。
- Docker：`/opt/yttt-control/{compose.yml,.env,control.toml}`（0600）。镜像以 `nobody`（65534）运行，安装脚本把 `control.toml` 与 SQLite 数据卷 chown 给 65534。
- 秘密（`ticket_secret`、`secrets_key`、PostgreSQL 容器密码）随机生成；安装结束打印文件位置，不打印秘密。

后续：`--upgrade`（换二进制 / 镜像与控制台文件，保留配置与数据）、`--reconfigure`（重走对外方式等，不碰秘密）、`--uninstall`（停止并移除服务；数据保留，`--purge` 才删除）。

## 7. relay 节点安装

- 控制台生成的部署命令保持“下载脚本 → 核对 SHA-256 → 执行”，脚本改为交互式（`/dev/tty`，同第 6 节的规则），参数仍可用：
  1. **安装方式**：直接安装（现状，systemd）/ Docker（`--network host`；登记在一次性容器里完成，配置与证书目录挂载到宿主机，属主 65534；绑定 443/80 需要容器以 root 运行并只保留 `NET_BIND_SERVICE`，或节点改用高位端口）。
  2. **域名**：有域名（自带 ACME，开放 80/443/UDP 7842）/ 无域名（只走 relay、不能打洞）/ 与控制面同机（第 8 节）。
- Docker 方式用同一个镜像，`--entrypoint yttt-relay`（镜像里已有 `/usr/local/bin/yttt-relay`）。
- `--upgrade` / `--uninstall` 两种方式都支持。

## 8. 控制面与 relay 节点同机（先做可行性验证）

80/443 只能由一个进程持有，同机时由 Caddy 管：

- Caddy（Docker 下用 host 网络）为两个域名签证书：`control.example → 127.0.0.1:8080`，`relay.example → 127.0.0.1:3340`（relay 本机明文监听，经 Caddy 的 HTTP Upgrade 转发）。relay 的 captive portal 不再单独占 :80，由 Caddy 转发。
- relay 新增只给 QUIC 地址探测用的 `quic_tls { cert_chain, private_key }`（HTTP 监听仍为明文），按文件修改时间定期重载；Caddyfile 固定 Let's Encrypt 签发者，证书路径可预知；安装脚本用 ACL 给 relay 服务用户读取这两个文件的权限（Caddy 私钥默认 0600 属主 caddy）。
- 控制面安装脚本在“直接对外”时询问“是否同时把本机部署为 relay 节点”，是则在控制台中建好节点并直接登记。
- **先验证**：iroh relay 在反向代理后的升级请求、`/generate_204`、客户端地址处理，以及额外一跳的 CPU 开销。不通过则同机时 relay 使用独立端口并自带 ACME（TLS-ALPN 需要 443，故只能换用 DNS 验证或放弃同机打洞），文档说明取舍。本节在验证通过后才进入实现。

## 9. 发布物

- 新增 Release 资产：`install-control.sh`（内嵌版本号）、`yttt-control-web-<version>.tar.gz`。控制面直接安装时从同一 Release 下载 relay 二进制放进 `relay_binaries_dir`，供节点部署命令使用。所有资产进 `SHA256SUMS`。
- 镜像 `ghcr.io/yuworm/yttt-relay` 继续包含控制面、控制台与 relay 二进制。

## 10. 不做

- 跨节点连接（Host 与 Client 在不同节点）。
- 跨账号使用付费节点。
- 两种数据库之间的数据迁移；控制面多实例高可用；安装脚本的备份命令（文档给出 `pg_dump` / `sqlite3 .backup` 用法）。

## 11. 任务

- N1 节点模型与计费：迁移（devices.role/node_id/follows_host/node_changed_unix、plans.max_nodes、plan_nodes、nodes.region/max_connections、node_revocations）；计费与 standing 按端点归属；`/v1/nodes`；登记事务（角色不可变、跟随校验）；ticket 的 `host` 与签发时复核；切换接口（`SwitchNode` 证明、冷却、`max_nodes` 加锁、`max_nodes = 1` 整体迁移）；持久撤销与确认、节点启动重放；管理员迁移按 Host。测试：Client 只拿到 Host 节点的 ticket；非本账号 Host 不占设备数；非默认节点上的流量计费且不被判 Suspended；切换后旧节点撤销（含离线设备与节点重启）；冷却与并发切换不越过 `max_nodes`；降级后下一次签发改回默认节点；设备会话不能切别的 Host；旧客户端走默认节点。
- N2 客户端与 Host：Host 每 5 分钟取 ticket 并在节点变化时 `set_relay`；`SetRelayNode` 管理请求与 `SwitchNode` 证明；Client 申请 ticket 带 `host`、404 回退；`yttt-server p2p nodes|node`。端到端测试（`#[ignore]`，PostgreSQL）：Host 切换节点后 Client 自动跟到新节点并恢复会话，旧节点拒绝旧 ticket。
- N3 统计与容量：`node_samples`（48 小时 5 分钟 + 90 天按天）与清理任务、在线端点、管理员节点页、用户设备页在线状态与节点切换。
- N4 app：节点列表与延迟、登录后选择、切换；i18n；无窗口渲染测试。
- N5 OAuth 设置进控制台：`secrets_key`、`oauth_providers` 加密存储、设置页、从 `control.toml` 导入一次。
- N6 双后端：`sqlx::Any` 可行性验证 → 查询改写、两套迁移、`BEGIN IMMEDIATE`、测试跑两种后端。
- N7 控制面安装脚本：Docker/直接安装 × SQLite/PostgreSQL × Caddy/只启动服务、管理员、秘密生成与幂等、升级/重新配置/卸载；Release 资产与工作流。验证：在 HK（或新的干净机器）上走代表组合。
- N8 relay 安装脚本：交互化、Docker 方式。
- N9 同机部署：先做 iroh relay 反代与 `quic_tls` 可行性验证，通过后实现 Caddy 双站点与一并部署节点。
- N10 文档：`relay-operations.md` 重写安装章节、`p2p-testing.md`、CHANGELOG。
