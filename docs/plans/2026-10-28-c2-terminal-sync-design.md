# C2 设计:Host 维护发送基线,只推可见终端

- 日期:2026-10-28
- 依据:[`2026-10-28-cs-convergence-plan.md`](./2026-10-28-cs-convergence-plan.md) C2、[`2026-10-28-cs-convergence-analysis.md`](./2026-10-28-cs-convergence-analysis.md) §4.2
- 协议:v22 内追加,不升 `RESOURCE_PROTOCOL_VERSION`(按 `docs/wire-evolution.md`)

## 1. 现状(改造前)

| 环节 | 位置 | 行为 |
|---|---|---|
| 解析/捕获 | `yttt-terminal-core/src/semantic.rs` `SemanticSnapshotter` | 每终端一份全局语义状态,最快 4 ms 捕获一次,产出行级 `Delta`(整行替换,`line_id` 稳定)或 `Snapshot` |
| 扇出 | `yttt-host/src/terminal.rs` broadcast(64) | 所有 attachment 共享同一 `SharedTerminalUpdate`,编码一次 |
| 每连接 | `yttt-host/src/lib.rs` `serve_terminal_data_connection` | 16 ms 合并连续 delta;broadcast lag → 发 `ResyncRequired` |
| 发送队列 | `yttt-host/src/terminal_data.rs` | 512 KiB;溢出清空队列并发 `ResyncRequired`,之后阻塞直到发出 |
| Client | `yttt-client-core/src/lib.rs` | 任何 gap / `ResyncRequired` / `RawTail` → 控制通道 `RequestCheckpoint`;重连后对 catalog 中所有终端请求 checkpoint |
| 原始回放 | `RawReplayRing`,订阅终端 8 MiB | 重新 attach 时按 `after_sequence` 发 `RawTail`;Client 不解析 raw 字节,只当作 gap 信号,于是又触发一次 checkpoint |

问题:gap 修复跨两条连接(data 通道报告、control 通道补发),两者无顺序保证;控制通道的滚动/读取响应也直接改写 mirror,与 data 通道的 delta 交错产生新的 gap。不可见终端照常产生帧。

## 2. 设计

### 2.1 基线:每 data 连接一个"已入队基线",不复制画面

Host 不为每个 Client 复制 viewport。`SemanticSnapshotter` 的全局 delta 本身是连续链,只要按链发送,Client 就不会断号。每条 data 连接只记录已交给写端的最后一帧的元数据:

```text
SentBaseline { session_epoch, sequence, geometry_epoch, scrollback_epoch,
               history_size, display_offset, modes, process_state }
```

- 共享 delta 只有 `base_sequence == baseline.sequence` 才入队;`sequence <= baseline.sequence` 的过期帧丢弃;其余情况改发 Snapshot。
- Snapshot 总是取 Host 当前状态(必要时先补一次捕获),发出后基线整体替换。
- 内存:每连接一个小结构;共享编码保留(未合并的单个 delta 仍是同一份编码字节)。

不选"行哈希"或"每 Client 保存上一帧画面":前者仍需全局链,后者每可见终端每 Client 多一份画面,且失去共享编码,而全局链 + 序号校验已经足够判断 Client 持有什么。

### 2.2 diff 粒度

沿用现有行级 delta(整行替换,行内是 span)。滚动不单独编码:`history_size` 变化走 `delta_between`,`scrollback_epoch` 变化走 Snapshot。本阶段只改同步协议,不改捕获。

### 2.3 渲染槽:写端空闲才交帧

每 data 连接持有一个 `PendingTerminalFrame`(只保留最新状态的槽):新 delta 合并进槽;合并失败变为"需要 Snapshot"。16 ms tick 时,只有发送队列中没有待写帧才把槽交给写端。慢 Client 时槽持续合并,基线不前进;写端恢复后一次交付合并帧或一份全量 Snapshot。

512 KiB 队列上限保留为硬上限:若仍溢出(大图片等),清空队列并标记"需要 Snapshot",不再发 `ResyncRequired`。

### 2.4 可见性

- 新请求 `SetTerminalVisibility { revision, visible }`(追加变体,无需授权能力,只影响本 Client 的流),响应 `TerminalVisibilityApplied { revision }`。Host 忽略不大于已记录 revision 的请求。
- 语义:Client 上报过一次后,集合外的 attachment 视为不可见;从未上报的 Client(CLI、旧 Client)全部可见。Client 断开后记录清除;重连后 ClientCore 重发当前集合。
- Client 侧:每个工作窗口(`WorkbenchView`)每次渲染收集实际渲染的终端 pane,集合变化时上报;`ClientCore` 对所有窗口取并集,变化时由 control 会话发送。窗口释放时撤回该窗口的集合。
- Host 侧:
  - 终端订阅数只计可见 attachment;全部不可见的终端跳过输出捕获(与未 attach 相同),标题、退出仍触发捕获。
  - 不可见 attachment 不发画面帧。标题/cwd/模式位或进程状态变化时,发一个只含元数据的 delta(`base_sequence` = 基线,无行),保持 tab 标题和退出状态更新。
  - 进程退出总是发最终 Snapshot(无论可见与否)。
  - 不可见 → 可见:发一份当前 Snapshot。
- CLI `panes read`:改为向 Host 请求当前 checkpoint,不读本地 mirror,隐藏 pane 也返回最新内容。

### 2.5 mirror 只由 data 通道写

- data 连接建立时 Host 先发 Snapshot;此后只发连续 delta、Snapshot 或元数据 delta。
- attachment 的 `display_offset` 任意变化都由 data 通道发对应偏移的 Snapshot(以前只在回到底部时发)。Client 不再把 `TerminalScrolled` / `TerminalViewport` 响应写进 mirror。
- `TerminalAttached` 的 checkpoint 只在该终端尚无 mirror 时作为初始内容。
- Client 仍校验 `base_sequence`。出现 gap 视为对端违约:断开该终端 data 连接并重连(Host 以 Snapshot 开流)。这也覆盖旧 Host 仍发送 `ResyncRequired` 的情况。
- `TerminalExit` 先于最终帧到达时,事件暂存,等 data 通道交付最终帧或 data worker 结束再发布;不再为此请求 checkpoint。

### 2.6 删除项

- Client:checkpoint 请求通道与 `PendingRequest::Checkpoint`、重连后对全部终端的 checkpoint、`checkpoint_resyncs` 计数(改为 `data_lane_resets`)。
- Host:`ResyncRequired` / `RawTail` 的发送路径、`pending_raw_after`、`RawReplayRing` / `ReplayBudget`(每终端最多 8 MiB)及其诊断字段。`AttachTerminal.after_sequence` 与 `RequestCheckpoint.after_sequence` 保留在 wire 上但被忽略;`raw_replay_tail` 恒为空。
- wire 变体全部保留(v22 冻结),`RequestCheckpoint` 继续服务旧 Client。

### 2.7 兼容

| 组合 | 结果 |
|---|---|
| 新 Client + 新 Host | 本设计 |
| 旧 Client + 新 Host | 旧 Client 不上报可见性 → 全部可见;从不收到 `ResyncRequired`;其 checkpoint 请求仍有响应 |
| 新 Client + 旧 Host | `SetTerminalVisibility` 得到 `Unsupported`,本连接不再发送;旧 Host 的 `ResyncRequired` 触发 data 连接重连 |

## 3. 改造前基线

`host_terminal_sync_traffic_probe`(release,ignored):8 个终端各输出 2000 行,owner + observer 两个 Client,全部可见。

| 运行 | owner 字节 | observer 字节 | Host 语义捕获 | 共享编码 | 队列丢弃 | resync | 队列高水位 |
|---|---|---|---|---|---|---|---|
| 1 | 2,650,113 | 2,638,501 | 3,035 | 39 | 0 | 0 | 6,244 |
| 2 | 2,560,241 | 2,571,845 | 3,200 | 27 | 0 | 0 | 6,244 |
| 3 | 3,089,810 | 3,079,130 | 3,588 | 37 | 0 | 0 | 6,244 |

改造后同一探针(release):

| 运行 | owner 字节 | observer 字节 | observer 重新可见 | Host 语义捕获 | 共享编码 | 队列丢弃 | data 连接重建 | 队列高水位 |
|---|---|---|---|---|---|---|---|---|
| 全部可见 1 | 2,619,927 | 2,631,972 | — | 3,862 | 23 | 0 | 0 | 6,244 |
| 全部可见 2 | 2,812,434 | 2,784,270 | — | 3,704 | 25 | 0 | 0 | 6,244 |
| observer 隐藏 1 | 2,558,725 | 0 | 50,048 | 4,049 | 16 | 0 | 0 | 6,272 |
| observer 隐藏 2 | 2,559,726 | 0 | 50,048 | 3,443 | 16 | 0 | 0 | 6,272 |

全部可见时流量与改造前持平;observer 隐藏时输出期间 data 通道 0 字节,重新可见时一次收到 8 个终端的当前 snapshot。

真实桌面冒烟(debug 构建,隔离 profile,`yttt ctl` 驱动):一个 `while :; do i=$((i+1)); echo "n=$i"; done`
终端在标签切换下交替可见/隐藏,各 5 s,CPU 取各线程 user+sys 之和:

| 状态 | PTY 吞吐(行/s) | Host CPU | 桌面 CPU |
|---|---|---|---|
| 隐藏 | 432,431 / 437,670 | 135% / 136% | 1% / 2% |
| 可见 | 192,348 / 192,910 | 146% / 147% | 5% / 4% |

隐藏时 Host 跳过捕获,同等 CPU 下解析吞吐约 2.3 倍,桌面不再接收画面帧。`panes read` 在隐藏期间返回 Host 最新内容;
桌面退出后 DesktopOwned Host 随之退出。本机未授予屏幕录制/辅助功能权限,重新可见后的画面未做目视确认,
该路径由集成测试与探针的重新可见 snapshot(50,048 字节)覆盖。

## 4. 验收

- Client 侧不再有 `SequenceGap` → checkpoint 的恢复路径与 checkpoint 请求通道。
- 隐藏终端:输出期间该 Client data 通道无画面帧;标题变化仍到达;重新可见时收到一份 Snapshot。
- 慢 Client:停止读取期间 Host 队列不超过上限、无 `ResyncRequired`;恢复读取后 mirror 与 Host 一致,且之间没有 gap。
- 同一探针在"observer 全部隐藏"下复测,observer 字节显著下降。
