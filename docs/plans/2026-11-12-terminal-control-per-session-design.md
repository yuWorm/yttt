# 终端控制权按终端分配：设计

## 1. 背景

移动端设计（`spike/mobile-client` 分支 `docs/plans/2026-11-11-mobile-client-design.md` §7）留下两个协议问题：

1. **控制权。** 现在一个 Host 只有一个 controller，所有 Interactive lease 都归它（`crates/yttt-host/src/runtime.rs` `ControlState`）。`TakeTerminalController` 把全部 lease 移给接管方；没有“交还”请求，`ReleaseTerminalControl` 只放掉一个 lease，controller 不变。手机接管一个终端就会拿走电脑上的所有终端，这与 `docs/p2p-relay-architecture.md` §10.4“不采用全局抢占”相矛盾，也做不出“只接管当前 pane”“交还电脑”“进后台自动交还”。
2. **协议锁步。** 两端握手都声明 `ProtocolRange::exact(22)`。

### 1.1 协议锁步不需要改协议

`docs/wire-evolution.md` 已经规定：v22 起追加请求、响应、事件和带缺省的字段都不升版本；旧端收到不认识的请求回 `FailureCode::Unsupported`，连接继续；必须升版本时 Host 声明包含新旧版本的区间并同时服务（规则 7）。所以落后几个桌面版本的手机 App 照样能连 v22 的 Host，只是新功能会得到 `Unsupported`。

需要的只是移动端遵守同一约定：每个新请求都要处理 `Unsupported`，隐藏或降级对应功能。本设计的两个新请求就按此处理（§4.3）。

## 2. 已定的取舍（2026-11-12）

- 电脑上点“接管”只取回当前 pane。状态栏不再有接管按钮，只显示有几个终端由其他设备控制。
- 手机交还（主动交还或进后台）时，控制权还给手机接管之前控制这个终端的 Client。
- 新 Client 第一次写入仍然拿走全部终端（现状不变）。

## 3. 模型

保留现有的 **主控**（`ControlState.controller`）：新终端的 lease 归它，Client 第一次写入时成为主控并拿走全部终端，主控离开后由继任者接替。新增 **单独接管**：一个 lease 可以被某个 Client 显式拿走，并记住交还给谁。

```rust
struct LeaseState {
    holder: ClientInstanceId,
    lease_epoch: u64,
    /// Set when `holder` took this one terminal on its own; such a lease is
    /// left alone by implicit hand-overs and goes back when returned.
    taken: Option<TakenLease>,
}

struct TakenLease {
    returns_to: Option<ClientInstanceId>,
}
```

### 3.1 规则

| 操作 | 结果 |
|---|---|
| `TakeTerminal { session_id }`（新） | 这一个 lease 移给请求方，新 `lease_epoch`。请求方是主控时 `taken = None`（回到主控名下）；否则 `taken = Some(returns_to)`，`returns_to` 取原 lease 的 `taken.returns_to`（原 lease 本身也是单独接管的），否则取原 holder，没有原 lease 时为 `None`。不改主控，不记入 `writers`。 |
| `ReturnTerminals { session_ids }`（新） | 对请求方持有的每个列出的 lease（空列表 = 请求方单独接管的全部）：交给 **归还目标**，没有目标就放掉（无人持有）。没持有的 session 跳过。 |
| `TakeTerminalController`（现有，显式全部接管） | 不变：成为主控，移走全部 lease，包括单独接管的；全部 `taken = None`。 |
| 第一次写入（`claim_locked` 首次请求，现有） | 成为主控，移走全部 lease，**但跳过 `taken` 的 lease**：隐式操作不推翻显式接管。 |
| 主控离开（`release_client`，现有） | 先把离开者单独持有的 lease 交给各自的归还目标；清除其他 lease 里指向离开者的 `returns_to`；再按原规则选继任主控，继任者只拿不是 `taken` 的 lease。**继任候选只限请求过写入的 Client（`writers`）**，只观察过的手机不会被动成为主控。 |
| 断线（`suspend_client`） | 不变：60 s 内保留全部 lease，包括单独接管的。 |

**归还目标**：`returns_to` 仍是已知 Client（在线或挂起）则交给它；否则交给主控（不是请求方本身时）；否则放掉。交给主控时 `taken = None`；交给 `returns_to` 时，如果它不是主控，`taken = Some(TakenLease { returns_to: None })`，仍不受隐式移交影响。

### 3.2 事件

不加新事件，沿用现有的：

- 失去 lease 的一方：被别人拿走收 `TerminalLeaseRevoked`，放掉（无人持有）收 `TerminalLeaseReleased`。
- 得到 lease 的一方：收 `TerminalControlGranted`。现有桌面端收到后会等 Host 的 geometry epoch，再把 PTY 改回自己的尺寸（`accept_control_grant`）。
- 只有主控变化时才发 `TerminalControllerChanged`。单独接管不改主控。

## 4. 协议（v22 追加，不升版本）

### 4.1 请求

```rust
/// Takes this one terminal; the Host remembers whom to give it back to.
/// Answered with `TerminalLease`.
TakeTerminal { session_id: TerminalSessionId },
/// Gives terminals this Client holds back to whoever held them before it
/// took them, or to the terminal controller. An empty list returns every
/// terminal this Client took on its own. Answered with `Applied`.
ReturnTerminals { session_ids: Vec<TerminalSessionId> },
```

两者需要 `Capability::TerminalInteractive`（与 `TakeTerminalController` 相同）。`TakeTerminal` 对不存在的终端回 `NotFound`。`ReturnTerminals` 是幂等的，App 进后台时可以放心重发。

### 4.2 wire 样本

两个变体追加到 `Request`，按 `docs/wire-evolution.md` 规则 3 用 `YTTT_BLESS_WIRE_SHAPES=1` 重写 `wire-shapes-v22.json`，在“当前版本”里记一笔。

### 4.3 新旧组合

| Client \ Host | 旧 Host | 新 Host |
|---|---|---|
| 旧桌面 | 现状 | 手机单独接管某个终端时，旧桌面在那个 pane 上收到 `LeaseRevoked` 变只读；状态栏按主控判断，不提示。点 pane 上的接管走 `TakeTerminalController`，拿回全部（包括手机那个）。可用，只是粒度粗。 |
| 新桌面 | `TakeTerminal` 回 `Unsupported`，退回 `TakeTerminalController`。 | 本设计 |
| 手机 | `TakeTerminal` 回 `Unsupported`：只能观察，提示 Host 需要升级。不退回全部接管，以免拿走电脑上的所有终端。 | 本设计 |

## 5. 桌面端

- pane 只读提示上的“接管”发 `TakeTerminal { session_id }`，遇到 `Unsupported` 再发 `TakeTerminalController`。
- 状态栏去掉接管按钮，文字改为“N 个终端由其他设备控制”。N 是当前窗口里因为别的 Client 持有 lease 而只读的运行中 pane 数（沿用 `observes_another_controller` 的判断）。这个算法对旧 Host 也成立：旧 Host 上主控不是自己时，所有 pane 都只读。
- 其余不变：重连后 `reclaim_host_lease` 照常隐式申请。它遇到单独接管的 lease 会得到 `LeaseConflict` 而保持只读，这正是第 3.1 节“隐式操作不推翻显式接管”的结果。

## 6. 移动端（`spike/mobile-client` 分支，本计划之后做）

- `take_control(session)` 改发 `TakeTerminal`，得到该终端的 lease epoch 后再 resize、放行输入。
- 交还按钮和进后台时发 `ReturnTerminals { session_ids: [] }`。iOS 在 `beginBackgroundTask` 内发，Android 在 `onStop` 发。进程被杀来不及发时，由 Host 的 60 s 宽限期兜底，之后按 §3.1 交还。
- `Unsupported` 时隐藏接管入口。
- 同时更新移动端设计 §7 第 1、2 条。

## 7. 不做

- 不缩短手机的断线宽限期（仍为 60 s）。
- 不改新建终端的规则：仍是非主控 Client 新建终端前要先成为主控（`claim_terminal_control`）。
- 不加“谁在控制这个终端”的查询或事件：手机只需要知道自己有没有控制权，现有 lease 事件已经够用。

## 8. 测试

- `crates/yttt-host/tests/terminal_runtime.rs`：
  - 单独接管只移走一个 lease，主控不变；
  - 交还给原持有者，原持有者已离开时交给主控，没有主控时放掉；
  - 第一次写入和继任都跳过单独接管的 lease，显式全部接管不跳过；
  - 只观察的 Client 不当继任者（同时修改 `a_departed_controller_hands_control_to_the_latest_present_viewer`，让两个 viewer 先请求写入）；
  - 连续接管时，归还目标指向最初的持有者。
- `crates/yttt-host/tests/control_connection.rs`：两个连接，B 用 `TakeTerminal` 拿一个终端，A 只在那个终端上收到 `TerminalLeaseRevoked`；B 用 `ReturnTerminals([])` 交还后，A 收到 `TerminalControlGranted`。
- `crates/yttt-protocol`：wire 样本、请求能力与 `session_id` 映射。
