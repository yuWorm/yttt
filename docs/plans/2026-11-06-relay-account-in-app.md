# 应用内登录 relay 账号

- 日期：2026-11-06
- 前置：[p2p-backend-plan](2026-11-06-p2p-backend-plan.md) B5（`yttt-control-client`、Host 的 `SetRelayAccount`）、[p2p-app-integration](2026-11-06-p2p-app-integration.md)（每台配对的 Host 一把设备密钥）
- 目的：付费 relay 在应用里可用：本机当 Host 时经账号的 relay 节点可达；本机当 Client 时经同一节点连到已配对的 Host，也能在只有 relay 可达时完成配对。

## 1. 账号存放

- 一个应用 profile 至多登录一个账号。会话 `{control_url, account_id, token}` 存系统凭据库（命名空间 `{profile}.relay-account`）；不含秘密的显示信息 `{control_url, email, account_id}` 存 `client-connections/relay-account.json`，界面读它，不必每次访问钥匙串。
- 密码只用于登录请求，不保存。会话失效（401）时界面提示重新登录。
- Host 与 Client 必须是同一账号（设计 S3）：账号决定 relay 节点，两端在同一节点上才能经 relay 互通。

## 2. 界面

“远程访问此电脑”页新增“relay 账号”一节（仅本机运行时显示）：

- 未登录：控制面地址、邮箱、密码、“登录”。
- 已登录：邮箱、套餐、节点 relay 地址、本月用量 / 额度、设备数 / 上限；“本机作为 Host 使用此账号”开关（`SetRelayAccount`）；“退出登录”。
- 退出登录：清除会话，若本机 Host 正在用此账号则同时 `SetRelayAccount { account: None }`。已登记的设备留在账号里，可在控制台移除。

## 3. Client 侧使用账号

- 配对：已登录时，为新生成的设备密钥登记设备（名称“<本机名> → <Host 名或前 8 位>”）并申请 ticket，endpoint 用 ticket 的 relay 与 token；配对失败或取消则从账号移除这把密钥。未登录时行为不变（配对码里的 relay，不带 token）。
- 连接：远程 Client 进程从凭据库读会话，登记（幂等）并申请 ticket，endpoint 使用 ticket 的 relay；在进程内按 `keep_ticket` 续签。取 ticket 失败时退回记录里的 relay 提示（不带 token），并写入连接日志。
- 忘记配对的 Host：同时从账号移除这把密钥（尽力而为，失败不阻止本地删除）。
- `yttt-control-client` 补 `remove_device`。

## 4. 不做

- 注册账号、支付、修改套餐（控制台的事）。
- 账号在多台设备间同步。

## 5. 任务

- E1 `yttt-control-client::remove_device`；应用侧账号存储模块（会话在凭据库、显示信息在文件），测试：存取、退出清除、显示信息不含 token。
- E2 “relay 账号”界面：登录、账号信息、Host 开关、退出；中英文 i18n；无窗口渲染测试。
- E3 Client 配对与连接使用账号（含失败回退、移除未完成配对的设备、忘记时移除）。
- E4 端到端（`#[ignore]`，需要 PostgreSQL）：控制面 + 节点 + Host 用账号 + 用应用侧代码路径配对与连接，只走 relay，用量增长。
- E5 更新 `docs/p2p-testing.md`：付费 relay 场景。
