# 按 Host 选节点、节点统计与一键安装

- 日期：2026-11-07
- 前置：[oauth-login](2026-11-07-oauth-login.md)（设备会话、浏览器登录）、[console-v2](2026-11-06-console-v2.md)、[relay-account-in-app](2026-11-06-relay-account-in-app.md)
- 已定：节点按设备选择，但 **Host 与连接它的远程 Client 永远在同一节点**；多节点限速按 a+b（每节点各自限速 + 套餐限制同时使用的节点数）；直接对外时由 Caddy 管证书；PostgreSQL 与 SQLite 双后端；节点在 app 内选择。

## 1. 节点模型：Host 选节点，Client 跟随 Host

### 1.1 现状与约束

- ticket 绑定一把设备密钥和一个节点（`Ticket { account_id, endpoint_id, node_id, … }`，节点核对 `node_id`）。relay 只在连到同一节点的端点之间转发。
- Client 为每台配对的 Host 生成一把独立的设备密钥、一个独立的 endpoint；一个 endpoint 只用一个 relay。
- 今天节点属于账号：所有设备拿账号节点的 ticket，Client 把自己 ticket 的 relay 当作 Host 的 relay（`src/remote_host.rs:503`）。

### 1.2 新模型

- **只有 Host 选节点。** 每个 Host 端点（本机作为 Host 时的 P2P 密钥）有自己的节点，账号的 `node_id` 退为“默认节点”：新 Host 落在它上面。
- **Client 跟随 Host，永不跨节点。** Client 申请 ticket 时说明要连哪台 Host；控制面核对该 Host 属于同一账号，按 **Host 的节点**签发 ticket 并返回该节点的 relay 地址。Client 的这个 endpoint 就以该节点为 relay。因为 Client 每台 Host 一个 endpoint，同时连在不同节点上的两台 Host 互不影响。
- 控制面不签发“Client 在 A 节点、Host 在 B 节点”的组合：Client 的节点完全由 Host 决定，Client 侧没有节点选择。
- 只有同一账号的 Client 能跟随（控制面校验）；其他账号或未登录的 Client 用不了付费节点，与现状一致。

### 1.3 数据

- `devices` 增加：
  - `role`：`host` / `client` / `unknown`（旧版本登记的设备）。
  - `node_id`：该端点所在节点。Host 自选；Client 等于其 `follows_host` 的节点；`unknown` 用账号默认节点。
  - `follows_host`：Client 跟随的 Host 端点（同账号 `devices` 中 `role = host` 的行）。
  - `node_changed_unix`：用于切换冷却。
- `accounts.node_id` 语义改为默认节点（列名不变，控制台文案改）。
- `plans` 增加 `max_nodes`（同时使用的节点数上限，默认 1）。
- `plan_nodes (plan_id, node_id)`：套餐可用的节点。迁移时为每个套餐写入全部现有节点，管理员之后在控制台收紧。
- `nodes` 增加 `region`（显示用，如“香港”）、`max_connections`（可空 = 不限）。

### 1.4 接口

- `GET /v1/nodes`（任意会话）：本账号套餐可用、状态为 active 的节点：`id、name、region、relay_url、load_percent、full`。`load_percent` 来自最近一次上报的连接数与 `max_connections`。
- `POST /v1/devices` 登记增加 `role` 与可选 `follows_host`（hex）。Host 登记时不带节点，落在账号默认节点（或已有的节点）。
- `POST /v1/tickets` 增加可选 `host`（hex）。
  - 本端点是 Host：签它自己节点的 ticket。
  - 带 `host`：校验同账号且对方是 Host，记下 `follows_host`，签对方节点的 ticket；对方不是本账号的 Host → 404 “这台 Host 不在你的账号里”。
  - 都没有（旧客户端）：沿用账号默认节点。
  - 返回值不变（`node_id`、`relay_url` 已有）。
- `PUT /v1/devices/{host}/node {node_id}`（任意会话，含设备会话）：切换一台 Host 的节点。校验：是本账号的 Host、节点在套餐可用列表且 active 且未满、冷却期（10 分钟）已过、切换后账号使用的节点数 ≤ `max_nodes`。设备会话可以调用：它不触及账号控制权，只影响本账号自己的连通。
- 控制台“设备”页：Host 一行显示节点并可切换；Client 显示“跟随 <Host>”。

### 1.5 切换的传播

1. 控制面更新 Host 的 `node_id`，并把所有 `follows_host` 为它的 Client 端点一并改到新节点，写 `node_changed_unix`。
2. 旧节点下一次上报（约 15 秒）时，回复里把这些端点放进已有的 `revoked_endpoints`（“不再是本节点的设备”）。节点断开它们，并拒绝它们此前签发的 ticket。
3. Host 与 Client 被断开后**立即重新申请 ticket**（不再只按 30 天的 2/3 续签）：响应里的 `node_id` 变了就 `set_relay` 到新节点。为避免风暴，同一端点 30 秒内最多重新申请一次。
4. Client 已有的“重连”流程在重连前重新申请 ticket；配对记录里的 relay 提示随之更新。
5. 管理员“迁移节点”改为：把某节点上的 Host（及跟随它们的 Client）迁到另一节点，传播方式同上；账号默认节点一并改。

### 1.6 计费与限速（a + b）

- 每个节点按账号各自限速（现状）；一个账号的 Host 分布在 N 个节点上时，最多得到 N 倍速率，由 `max_nodes` 限制 N。
- 月流量额度跨节点合计（`usage_months` 已按账号累计，节点上报都计入）。超额后每个节点都降到 `throttled_rate_bps`。
- 设备数上限 `max_devices` 不变：Host 与 Client 端点都计数。

### 1.7 兼容

- 旧客户端：不带 `role`/`host`，按账号默认节点工作；只要账号的 Host 都在默认节点，行为与今天相同。控制台提示“旧版本客户端只能连默认节点上的 Host”。
- 旧 Host：登记为 `unknown`，ticket 落在默认节点。升级后的 Host 再次登记时补上 `role = host`。

## 2. 节点统计与容量

- 节点上报里已有每个账号连着的端点列表。控制面据此记录：
  - `nodes.last_connected_devices`（已有）；
  - 新表 `node_samples (node_id, bucket_unix, connections_max, upstream_bytes)`，按 5 分钟分桶，保留 30 天；
  - 每个账号当前在线的端点（内存 + 最近一次上报时间），用于“在线设备”。
- 控制台：
  - 管理员“节点”页：每个节点的当前连接数 / 上限、近 24 小时与 30 天的连接数曲线与流量；可设 `region`、`max_connections`。
  - 用户“设备”页：每台设备是否在线、所在节点。
- 容量：满载（`connections >= max_connections`）的节点不能被新选择，也不作为新账号的默认节点；已在其上的不受影响。

## 3. app 内选节点

- “远程访问此电脑”页的 relay 账号一节新增“节点”：列出 `GET /v1/nodes`，并由 app 实测延迟（对每个节点 `relay_url` 的 `/healthz` 做 3 次 HTTPS 请求取中位数），显示名称、地区、延迟、负载；满载的置灰。
- 浏览器登录成功后，如果本机作为 Host 使用账号，直接弹出这个选择（默认选中当前节点，可跳过）；之后随时可在同一处切换。选择即调用 `PUT /v1/devices/{本机 Host 端点}/node`。
- 只作为 Client 的机器不显示节点选择：它跟随所连的 Host。
- 无屏 Host：`yttt-server p2p nodes` 列出节点（含延迟），`yttt-server p2p node <id>` 切换。

## 4. OAuth 设置进控制台

- 新表 `oauth_providers (provider, client_id, client_secret_sealed, enabled, updated_unix)`。
- `client_secret` 用由 `ticket_secret` 派生的密钥加密后存储（XChaCha20-Poly1305，密钥 = SHA-256("yttt oauth" ‖ ticket_secret)），数据库备份泄露也拿不到明文；更换 `ticket_secret` 后需要重新填写（文档说明）。
- 管理员“设置”页新增“第三方登录”：显示回调地址（`{public_url}/v1/auth/{provider}/callback`，可复制）、client id、secret（只写不回显）、启用开关。保存时校验 `public_url` 条件（GitHub 需要 `public_url`；Google 需要 https）。
- `control.toml` 的 `[oauth.*]` 在启动时若数据库中还没有该提供方则导入一次，之后以控制台为准；文档改为推荐在控制台配置。

## 5. PostgreSQL 与 SQLite 双后端

- `database_url` 决定后端：`postgres://…` 或 `sqlite:///var/lib/yttt-control/control.db`。
- 实现：
  - 数据访问集中到一个 `Db` 类型（内部 `PgPool` / `SqlitePool` 二选一），查询改写为两者都支持的 SQL：去掉 `::` 类型转换、`BYTEA` 只出现在迁移里、`FOR UPDATE` 换成事务（SQLite 用 `BEGIN IMMEDIATE`，PostgreSQL 保留行锁），`RETURNING`、`ON CONFLICT`、`UPDATE … FROM` 两者都有。占位符统一 `$N`，每个占位符在一条语句里只出现一次（sqlx-sqlite 按位置绑定）。
  - 迁移分两套：`migrations/postgres/`、`migrations/sqlite/`，内容一一对应；现有 0001–0004 原样移到 `postgres/`，sqlite 一次性写成等价的初始结构后同步演进。
  - SQLite 打开时启用 WAL、`foreign_keys = ON`、`busy_timeout`。
- 测试：`tests/control.rs` 用同一套用例分别跑两种后端（SQLite 不需要外部服务，默认就跑；PostgreSQL 仍由 `YTTT_TEST_DATABASE_URL` 开启）。
- 迁移现有部署：不自动在两种数据库之间搬数据；`yttt-control export` / `import`（JSON）留作后续。

## 6. 控制面一键安装

`install-control.sh`，作为 Release 资产发布：`curl -fsSL https://github.com/yuWorm/yttt/releases/latest/download/install-control.sh | sudo sh`。默认交互（从 `/dev/tty` 读输入），所有问题都有对应参数，`--yes` 用于无人值守。

问题依次为：

1. **安装方式**：Docker（compose）/ 直接安装（Linux + systemd，下载静态二进制）。
2. **数据库**：SQLite（推荐个人使用）/ PostgreSQL。Docker 下 PostgreSQL 用官方镜像容器；直接安装下用发行版的 `postgresql` 包（apt/dnf），创建库与用户。
3. **对外方式**：
   - **直接对外**：输入域名（检查已解析到本机）、邮箱（证书通知）。安装 Caddy（Docker 下为容器，直接安装下为官方静态二进制 + systemd），占用 80/443，自动申请与续期证书；控制面只监听 `127.0.0.1:8080`，`trusted_proxy = true`，`public_url = https://域名`。
   - **只启动服务**（自带 nginx 等）：选择监听地址（只绑 `127.0.0.1` / `0.0.0.0`）与端口，输入对外地址作为 `public_url`；在反向代理之后时 `trusted_proxy = true`。输出一段 nginx 配置示例。
4. **管理员**：邮箱、密码（隐藏输入两次），安装末尾执行 `create-admin`。
5. **注册**：是否开放注册（之后可在控制台改）。

自动生成并保存：

- 数据库密码、`ticket_secret`：随机生成。直接安装写入 `/etc/yttt-control/control.toml`（0600，属主为服务用户）；Docker 写入 `/opt/yttt-control/.env` 与 `control.toml`（0600）。安装结束打印文件位置，不打印秘密本身。
- 服务用户 `yttt-control`、数据目录 `/var/lib/yttt-control`、systemd 单元（含加固选项，参照 relay 单元）。

后续操作：`install-control.sh --upgrade`（换二进制/镜像与控制台静态文件，保留配置与数据）、`--reconfigure`（重新走对外方式等问题）、`--uninstall`（停止并移除服务；数据保留，`--purge` 才删除）、`--backup`（PostgreSQL `pg_dump` / SQLite `.backup`）。

## 7. relay 节点安装

- 控制台生成的部署命令不变（`curl … relay.sh | sudo YTTT_ENROLL=… sh`），脚本改为交互式（`/dev/tty`），参数仍然可用：
  1. **安装方式**：直接安装（现状，systemd）/ Docker（`--network host`，QUIC 需要真实地址；登记在一次性容器里完成，配置与证书目录挂载到宿主机）。
  2. **域名**：有域名（自带 ACME，开放 80/443/UDP 7842）/ 无域名（只走 relay、不能打洞，现状提示）/ 与控制面同机且由 Caddy 管证书（见第 8 节）。
- Docker 镜像：发布的镜像里把 `yttt-relay` 放进 `PATH`，节点用 `--entrypoint yttt-relay` 运行同一个镜像，不另发镜像。
- `--upgrade` / `--uninstall` 两种安装方式都支持。

## 8. 控制面与 relay 节点同机

个人用户常把两者放在一台小机器上。80/443 只能由一个进程持有，因此同机时一律由 Caddy 管：

- Caddy 为控制面域名与 relay 域名各签一张证书，`control.example → 127.0.0.1:8080`，`relay.example → 127.0.0.1:3340`（relay 改为本机明文监听，经 Caddy 的 HTTP Upgrade 转发）。
- relay 的 QUIC 地址探测（UDP 7842）需要 relay 域名的证书：relay 以 `[tls]` 文件方式读取 Caddy 存储目录里的证书（已支持文件证书与热加载）。Caddyfile 固定使用 Let's Encrypt 签发者，路径可预知；Docker 下以只读卷共享。
- 控制面安装脚本在“直接对外”时询问“是否同时把本机部署为 relay 节点”，是则在控制台中建好节点并直接登记，无需再复制部署命令。
- 待验证：iroh relay 在反向代理后的行为（升级请求、`/generate_204`、客户端地址）与额外一跳的 CPU 开销；验证不通过则同机时 relay 使用独立端口并自带 ACME（DNS 或非 443 端口），在文档中说明取舍。

## 9. 发布物

- 新增 Release 资产：`install-control.sh`、`yttt-control-web-<version>.tar.gz`（直接安装需要的控制台静态文件）。控制面直接安装时从同一 Release 下载 relay 二进制放进 `relay_binaries_dir`，供节点部署命令使用。
- 镜像 `ghcr.io/yuworm/yttt-relay` 继续包含控制面、控制台与 relay 二进制，并把 `yttt-relay` 放进 `PATH`。

## 10. 不做

- 跨节点连接（Host 与 Client 在不同节点）。
- 跨账号使用付费节点。
- 两种数据库之间的在线迁移（只做导出/导入的后续项）。
- 控制面多实例高可用。

## 11. 任务

- N1 节点模型：迁移（devices.role/node_id/follows_host/node_changed_unix、plans.max_nodes、plan_nodes、nodes.region/max_connections）；`/v1/nodes`、登记与 ticket 的 `role`/`host`、`PUT /v1/devices/{host}/node`；切换传播（`revoked_endpoints`）；管理员迁移改为按 Host。测试：Client 只能拿到 Host 节点的 ticket、非本账号 Host 404、切换后旧节点收到撤销、冷却、`max_nodes`、满载节点不可选、旧客户端走默认节点。
- N2 客户端：被断开后立即重新申请 ticket 并 `set_relay`（Host 与 Client，30 秒节流）；Client 申请 ticket 带 `host`；`yttt-server p2p nodes|node`。端到端测试（`#[ignore]`，PostgreSQL）：Host 切换节点后 Client 自动跟到新节点并恢复会话。
- N3 统计与容量：`node_samples`、在线端点、管理员节点页曲线与上限、用户设备页在线状态与节点。
- N4 app：节点列表与延迟、浏览器登录后的选择、切换；i18n；无窗口渲染测试。
- N5 OAuth 设置进控制台：`oauth_providers`、加密存储、设置页、从 `control.toml` 导入一次。
- N6 双后端：`Db` 抽象与查询改写、两套迁移、测试跑两种后端。
- N7 控制面安装脚本：Docker/直接安装 × SQLite/PostgreSQL × Caddy/只启动服务、管理员、秘密生成、升级/重新配置/卸载/备份；Release 资产与工作流。验证：在 HK（或新的干净机器）上各走一遍组合中的代表路径。
- N8 relay 安装脚本：交互化、Docker 方式、镜像内 `yttt-relay` 入 `PATH`。
- N9 同机部署：Caddy 双站点、relay 读 Caddy 证书、控制面安装时可一并部署节点；先做 iroh relay 反代可行性验证。
- N10 文档：`relay-operations.md` 重写安装章节、`p2p-testing.md`、CHANGELOG。
