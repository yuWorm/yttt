# 浏览器授权登录与 GitHub/Google 登录

- 日期：2026-11-07
- 前置：[relay-account-in-app](2026-11-06-relay-account-in-app.md)（应用内账号、会话存凭据库）、[console-v2](2026-11-06-console-v2.md)（控制台）
- 评审：已按设计评审修订（会话范围、设置密码需近期登录、各项校验细则）。
- 目的：应用与无屏 Host 不再输入邮箱密码：应用跳浏览器、在控制台点“允许”即登录；无屏 Host 显示网址和短码，在任意设备上确认。控制台支持用 GitHub / Google 账号注册和登录，账号可以完全不设密码。

## 1. 三条流程

### 1.1 应用：授权码 + PKCE，回环地址回调（RFC 8252）

1. 应用生成 `state`（32 字节随机）与 PKCE `verifier`，在 `127.0.0.1` 随机端口监听，打开浏览器：
   `{control}/authorize?redirect_uri=http://127.0.0.1:{port}/callback&state=…&code_challenge=…&code_challenge_method=S256&label=<本机名>`
2. 控制台 `/authorize` 页：未登录先登录（含 GitHub/Google），回到本页；显示“允许 **<label>** 使用你的 relay 账号 **<email>**？”与“允许 / 拒绝”。
3. “允许”→ `POST /v1/oauth/authorize`（cookie + 控制台请求头）→ 控制面存一次性授权码（只存哈希，60 秒，绑定 challenge、redirect_uri、label、账号），返回带 `code`、`state` 的回调地址，页面跳过去。“拒绝”→ 跳回调地址带 `error=access_denied`。
4. 应用的回环监听收到回调，核对 `state`，`POST /v1/oauth/token {grant_type: authorization_code, code, code_verifier, redirect_uri}` 换得会话（与密码登录相同的 `SessionGranted`），回调页显示“已登录，可以关闭此页”。
5. 等待 10 分钟或用户点“取消”即放弃，监听关闭。

`redirect_uri` 只接受 `http://127.0.0.1:<port>/…` 与 `http://[::1]:<port>/…`：授权码只会送到发起登录的那台机器上，别人诱导用户点开的授权链接拿不到码；码没有 verifier 也换不到会话。

### 1.2 无屏 Host：设备码（RFC 8628）

1. `yttt-server p2p login <control-url>`（不再要求邮箱）→ `POST /v1/oauth/device {label}` → `{device_code, user_code, verification_uri, verification_uri_complete, interval, expires_in}`。`user_code` 为 8 个易读字符（去掉 0/O/1/I），显示成 `ABCD-EFGH`，10 分钟有效。
2. 终端打印网址、短码，以及 `verification_uri_complete` 的二维码（复用配对二维码的渲染）。
3. 控制台 `/device` 页：登录后输入短码（带 `?code=` 时预填），显示 label，“允许 / 拒绝”→ `POST /v1/oauth/device/approve`。
4. CLI 每 `interval` 秒 `POST /v1/oauth/token {grant_type: device_code, device_code}`：`authorization_pending` / `slow_down`（轮询过快）/ `access_denied` / `expired_token` / 成功返回会话。
5. `--password <email>` 保留原邮箱密码登录（脚本化部署用，密码仍取 `YTTT_RELAY_PASSWORD`）。

### 1.3 控制台：GitHub / Google

1. 登录、注册页按配置显示“使用 GitHub 登录”“使用 Google 登录”（`GET /v1/auth/providers`）。
2. `GET /v1/auth/{provider}/start?next=<站内路径>` → 存登录状态（`state`、PKCE verifier、`next`、是否为“关联”）→ 设 `yttt_oauth` cookie（`SameSite=Lax`，10 分钟，只放随机 state）→ 跳转提供方。
3. `GET /v1/auth/{provider}/callback?code&state` → 核对 cookie 与 state → 服务端用 client secret 换 token → 取身份：
   - GitHub：`/user`（数字 id）+ `/user/emails` 里已验证的主邮箱。
   - Google：OpenID Connect userinfo，要求 `email_verified`。
4. 身份映射（新表 `identities`，主键 `(provider, subject)`，subject 是提供方的不变 id，不是邮箱）：
   - 已关联 → 登录该账号。
   - 未关联、邮箱未注册 → 注册开放时按注册设置的套餐与节点建号（无密码）并关联；注册关闭则拒绝。
   - 未关联、邮箱已有账号 → **不自动合并**，提示“该邮箱已有账号，请用密码登录后在『账号』页关联”。避免靠提供方邮箱接管已有账号（尤其管理员）。
5. 成功后设会话 cookie，跳到 `next`（只允许站内路径，防开放跳转）。会话 cookie 仍为 `SameSite=Strict`；从提供方跳回是跨站导航，所以只有 `yttt_oauth` 用 Lax。
6. 账号页：已关联的登录方式列表、“关联 GitHub/Google”、解除关联（账号没有密码时不能解除最后一个）；没有密码的账号“修改密码”改为“设置密码”，不要求当前密码。

## 2. 会话范围（评审 blocker）

授权页与设备码页可以被钓鱼：攻击者自己发起登录，把链接发给受害者，受害者点了“允许”，攻击者就拿到会话。所以浏览器授权与设备码换出的会话**只能做设备的事**：

- 会话表加 `scope`：`web`（控制台 cookie 登录）与 `device`（应用、CLI：授权码、设备码、`POST /v1/sessions` 密码登录）。
- `device` 会话只能访问 `/v1/accounts/me`、设备登记/列表/移除、ticket。这些正是它被授权的用途：用账号的 relay 流量。
- `/v1/admin/*`、改密码、设置密码、关联/解除关联、`/v1/oauth/authorize`、设备码确认只认 cookie 携带的 `web` 会话。
- 无密码账号“设置密码”要求当前 `web` 会话在 5 分钟内刚登录（会话表记 `signed_in_unix`）；否则控制台引导重新登录后再设。被偷的 cookie 不能借此变成永久的密码登录。
- 授权页和设备码页写明：“只有当这台设备就在你面前、并且是你刚刚发起的登录时才允许”；显示发起方 IP、时间，label 标注“由设备自报”；带 `?code=` 预填时仍需核对短码再确认。

## 3. 控制面

- 迁移 `0004_oauth.sql`：
  - `accounts.password_hash` 可为空（只用第三方登录的账号）；`check_login` 遇空哈希按失败处理，仍走诱饵哈希以保持耗时一致。
  - `identities (provider, subject, account_id, email, created_unix)`，`ON DELETE CASCADE`。
  - `oauth_codes (code_hash, account_id, challenge, redirect_uri, label, expires_unix)`
  - `sessions` 加 `scope` 与 `signed_in_unix`；已有会话无法区分来源，一律标为 `device`：应用照常可用，控制台需重新登录一次。只认 `web` 的接口同时要求会话来自 cookie。
  - `device_grants (device_code_hash, user_code UNIQUE, label, requester_ip, account_id NULL, denied, failures, created_unix, expires_unix, last_poll_unix)`。
  - `login_states (state_hash, provider, verifier, next, link_account NULL, expires_unix)`。
  - 过期行在写入同类数据时顺带删除，与 `open_session` 清理过期会话的做法一致。
- 配置（密钥属于部署，留在配置文件，和 `ticket_secret` 一样不进数据库）：

  ```toml
  [oauth.github]
  client_id = "…"
  client_secret = "…"

  [oauth.google]
  client_id = "…"
  client_secret = "…"
  ```

  配置了提供方时必须有 `public_url`（回调地址 `{public_url}/v1/auth/{provider}/callback`），否则启动失败并说明原因。提供方的端点地址可在配置里覆盖，仅供测试接假提供方。
- 授权码和设备码换出的会话有效期与密码登录相同（30 天），暂停账号时一并失效；范围见第 2 节。
- 审计：所有账号的关联、解除关联写入审计日志。
- 校验细则：
  - `redirect_uri` 用 URL 解析：只收 `http` 且主机为 `127.0.0.1` 或 `[::1]`，端口任意，拒绝 userinfo 与 fragment；换 token 时与存的完全相等；只收 S256。非法时页面只显示错误，绝不跳转；“拒绝”跳回时也带 `state`。
  - 授权码、设备码用 `DELETE … RETURNING` / 单条 `UPDATE` 原子地取用，只能用一次。
  - 授权、设备码确认要求账号未暂停。
  - 设备码：发起与输入短码走现有按来源限速；轮询不走（device_code 256 位随机），只按固定 `interval` 回 `slow_down`；同一 grant 输错 5 次作废。短码比对前统一大写、去掉 `-` 与空格。`verification_uri` 用 `public_url`，没有时用 CLI 传入的控制面地址拼。
  - `next` 必须以单个 `/` 开头，拒绝 `//`、`/\\`、反斜杠与控制字符；密码登录与第三方登录共用同一回跳逻辑。
  - `yttt_oauth` cookie 与会话 cookie 同样随 `public_url` 决定 `__Host-` 前缀与 `Secure`，`HttpOnly`、`Path=/`、`SameSite=Lax`；回调后清除并删除对应 `login_states`。
  - 回调成功返回 200 页面，用 `<meta http-equiv="refresh">` 站内跳转（不用 302），避免跨站重定向链不带 Strict cookie。
  - “关联”用 `POST`（控制台请求头 + cookie），返回提供方地址再由前端跳转；解除关联后结束该账号其他会话。
  - GitHub：OAuth App，scope `read:user user:email`，换 token 带 `Accept: application/json`，PKCE 与 `state` 都用。Google：scope `openid email`，subject 取 `sub`，要求 `email_verified`，用 userinfo。配置了 Google 而 `public_url` 不是 https 时启动失败。
  - 同邮箱已被（可能是他人）用密码注册时，提示“该邮箱已有账号：请用密码登录后在『账号』页关联；如果不是你注册的，请联系管理员”。第三方新建账号复用注册检查（开关、节点 active、套餐未归档）。
  - `/v1/oauth/token` 是自家客户端用的 JSON 接口，不宣称与 RFC 6749 表单编码兼容；错误统一 400 + `{error}`。
  - 限速在内存，多实例各算各的（运维文档注明）；流程状态都在数据库，重启不丢。

## 4. 应用

- `yttt-control-client` 新增 `browser_login` 模块：`BrowserLogin::start(control_url, label) -> (authorize_url, PendingLogin)`、`PendingLogin::finish(timeout) -> ControlSession`；回环监听只处理一个 `GET /callback`，请求行和头部有大小上限。设备码：`start_device(label)`、`poll_device(...)`。
- “relay 账号”未登录时：服务地址 + “在浏览器中登录”（主按钮）。等待时显示“已在浏览器中打开登录页…”与“取消”。“改用邮箱密码登录”展开原表单，作为浏览器不可用时的退路。
- 登录后用 `/v1/accounts/me` 的邮箱写显示信息（浏览器登录时应用不知道邮箱）。
- 中英文 i18n。

## 5. 不做

- 其他提供方、通用 OIDC 发现。
- 应用会话列表与逐个吊销（控制台“退出所有会话”另做）。
- 自定义 URL scheme（`yttt://`）回调：回环地址已满足桌面三平台。

## 6. 任务

- O1 迁移、会话范围、`password_hash` 可空、授权码 + PKCE 与设备码端点；测试：device 会话进不了管理与改密码接口、PKCE 正确/错误、码只能用一次、过期、`redirect_uri` 只收回环、设备码 pending → 允许/拒绝/过期、`slow_down`。
- O2 GitHub/Google：配置、start/callback、身份映射（已关联、新建、同邮箱拒绝合并、注册关闭）、关联与解除、无密码账号设置密码；测试接假提供方。
- O3 控制台：`/authorize`、`/device`、登录与注册页的提供方按钮、`next` 回跳、账号页登录方式与设置密码。
- O4 `yttt-control-client` 浏览器登录与设备码；测试：回环回调、state 不符、取消、拒绝。
- O5 应用面板改为浏览器登录（保留密码退路）、i18n、无窗口渲染测试；`yttt-server p2p login` 改为设备码（`--password` 保留）。
- O6 文档：`docs/relay-operations.md`（创建 GitHub/Google OAuth 应用、回调地址、配置）、`control.example.toml`、`docs/p2p-testing.md`、CHANGELOG。
- O7 真实验证：HK 控制面升级，`ego` 走一遍控制台授权页与设备码页；本机应用浏览器登录；HK 上 `yttt-server p2p login` 设备码。GitHub/Google 需要你创建 OAuth 应用并给出 client id/secret 后验证。
